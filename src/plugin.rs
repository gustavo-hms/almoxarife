use colorized::Color;
use colorized::Colors;
use serde::Deserialize;
use std::collections::HashMap;
use std::error;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::os::unix;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

use crate::config::Config;

type Name = String;
type Message = String;

#[derive(Debug)]
pub enum Error {
    Clone(Name, Message),
    Pull(Name, Message),
    Link(Name, Message),
}

impl Error {
    pub fn plugin(&self) -> &str {
        match self {
            Error::Clone(name, _) => name,
            Error::Pull(name, _) => name,
            Error::Link(name, _) => name,
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Clone(name, message) => {
                write!(
                    f,
                    "{}: could not clone: {message}",
                    name.color(Colors::RedFg)
                )
            }

            Error::Pull(name, message) => {
                write!(
                    f,
                    "{}: could not update: {message}",
                    name.color(Colors::RedFg)
                )
            }

            Error::Link(name, message) => {
                write!(
                    f,
                    "{}: could not activate: {message}",
                    name.color(Colors::RedFg)
                )
            }
        }
    }
}

impl error::Error for Error {}

pub enum Status {
    Installed {
        name: String,
        config: String,
    },
    Updated {
        name: String,
        log: String,
        config: String,
    },
    Unchanged {
        name: String,
        config: String,
    },
    Local {
        name: String,
        config: String,
    },
}

#[derive(Debug, Deserialize)]
pub struct PluginTree {
    location: String,
    #[serde(default)]
    config: String,
    #[serde(default)]
    disabled: bool,
    #[serde(flatten)]
    children: HashMap<String, PluginTree>,
}

impl PluginTree {
    pub fn plugins(&self, name: String, config: &Config) -> Vec<Plugin> {
        if self.disabled {
            return Vec::new();
        }

        let children = self
            .children
            .iter()
            .flat_map(|(child_name, child)| child.plugins(child_name.clone(), config));

        let mut plugins = vec![Plugin::new(name, self, config)];

        for child in children {
            plugins.push(child);
        }

        plugins
    }
}

pub struct Plugin {
    name: String,
    location: String,
    is_local: bool,
    repository_path: PathBuf,
    link_path: PathBuf,
    disabled: bool,
    config: String,
}

fn is_local(location: &str) -> bool {
    !location.starts_with("https://")
        && !location.starts_with("http://")
        && !location.starts_with("git://")
}

impl Plugin {
    fn new(name: String, node: &PluginTree, config: &Config) -> Plugin {
        let link_path = config.autoload_plugins_dir.join(&name);

        let (is_local, repository_path) = if is_local(&node.location) {
            (true, PathBuf::from(&node.location))
        } else {
            (false, config.almoxarife_data_dir.join(&name))
        };

        Plugin {
            name,
            disabled: node.disabled,
            config: node.config.clone(),
            location: node.location.clone(),
            is_local,
            repository_path,
            link_path,
        }
    }

    fn repository_path_exists(&self) -> bool {
        fs::metadata(&self.repository_path).is_ok()
    }

    pub fn update(self) -> Result<Status, Error> {
        let config = self.config();
        let name = self.name.clone();

        let status = match (self.is_local, self.repository_path_exists()) {
            (true, true) => Status::Local { name, config },

            (true, false) => {
                return Err(Error::Link(
                    name,
                    format!("the path {} is empty", self.location),
                ))
            }

            (false, true) => match self.pull()? {
                None => Status::Unchanged { name, config },
                Some(log) => Status::Updated { name, log, config },
            },

            (false, false) => {
                self.clone_repo(&self.location)?;
                Status::Installed { name, config }
            }
        };

        self.symlink()?;
        Ok(status)
    }

    fn symlink(&self) -> Result<(), Error> {
        if !self.disabled {
            unix::fs::symlink(&self.repository_path, &self.link_path)
                .map_err(|e| Error::Link(self.name.clone(), e.to_string()))?;
        }

        Ok(())
    }

    fn clone_repo(&self, url: &str) -> Result<(), Error> {
        let location = format!("{url}.git");

        let status = Command::new("git")
            .arg("clone")
            .arg(location)
            .arg(&self.repository_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| Error::Clone(self.name.clone(), e.to_string()))?;

        match status.code() {
            None | Some(0) => Ok(()),
            Some(code) => Err(Error::Clone(
                self.name.clone(),
                format!("git exited with status {}", code),
            )),
        }
    }

    fn pull(&self) -> Result<Option<String>, Error> {
        let old_revision = self.current_revision();

        let status = Command::new("git")
            .arg("pull")
            .current_dir(&self.repository_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| Error::Pull(self.name.clone(), e.to_string()))?;

        if let Some(code) = status.code() {
            if code != 0 {
                return Err(Error::Pull(
                    self.name.clone(),
                    format!("git exited with status {}", code),
                ));
            }
        }

        if let Some(old) = old_revision {
            if let Some(new) = self.current_revision() {
                return Ok(self.log(old, new));
            }
        }

        Ok(None)
    }

    pub fn config(&self) -> String {
        format!("try %[ require-module {} ]\n{}\n", self.name, self.config)
    }

    fn current_revision(&self) -> Option<String> {
        let output = Command::new("git")
            .current_dir(&self.repository_path)
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()?;

        let mut revision = String::from_utf8_lossy(&output.stdout).to_string();
        revision.pop(); // Remove \n
        Some(revision)
    }

    fn log(&self, old_revision: String, new_revision: String) -> Option<String> {
        if old_revision == new_revision {
            return None;
        }

        let range = format!("{old_revision}..{new_revision}");

        let output = Command::new("git")
            .current_dir(&self.repository_path)
            .args(["log", &range, "--oneline", "--no-decorate", "--reverse"])
            .output()
            .ok()?;

        Some(String::from_utf8_lossy(&output.stdout).to_string())
    }
}
