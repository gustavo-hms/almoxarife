use colorized::Color;
use colorized::Colors;
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt::Display;
use std::os::unix;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::fs;
use tokio::process::Command;

use crate::config::Config;

#[derive(Debug)]
pub enum Error {
    Clone { name: String, message: String },
    Pull { name: String, message: String },
    Link { name: String, message: String },
}

impl Error {
    pub fn plugin(&self) -> &str {
        match self {
            Error::Clone { name, .. } => name,
            Error::Pull { name, .. } => name,
            Error::Link { name, .. } => name,
        }
    }

    pub fn error(&self) -> String {
        match self {
            Error::Clone { name, message } => {
                format!("{}: could not clone: {message}", name.color(Colors::RedFg))
            }

            Error::Pull { name, message } => {
                format!("{}: could not update: {message}", name.color(Colors::RedFg))
            }

            Error::Link { name, message } => {
                format!(
                    "{}: could not activate: {message}",
                    name.color(Colors::RedFg)
                )
            }
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error())
    }
}

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

    async fn repository_path_exists(&self) -> bool {
        fs::metadata(&self.repository_path).await.is_ok()
    }

    pub async fn update(self) -> Result<Status, Error> {
        let config = self.config();
        let name = self.name.clone();

        let status = match (self.is_local, self.repository_path_exists().await) {
            (true, true) => Status::Local { name, config },

            (true, false) => {
                return Err(Error::Link {
                    name,
                    message: format!("the path {} is empty", self.location),
                })
            }

            (false, true) => match self.pull().await? {
                None => Status::Unchanged { name, config },
                Some(log) => Status::Updated { name, log, config },
            },

            (false, false) => {
                self.clone_repo(&self.location).await?;
                Status::Installed { name, config }
            }
        };

        self.symlink().await?;
        Ok(status)
    }

    async fn symlink(&self) -> Result<(), Error> {
        if !self.disabled {
            unix::fs::symlink(&self.repository_path, &self.link_path).map_err(|e| Error::Link {
                name: self.name.clone(),
                message: e.to_string(),
            })?;
        }

        Ok(())
    }

    async fn clone_repo(&self, url: &str) -> Result<(), Error> {
        let location = format!("{url}.git");

        let status = Command::new("git")
            .arg("clone")
            .arg(location)
            .arg(&self.repository_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(|e| Error::Clone {
                name: self.name.clone(),
                message: e.to_string(),
            })?;

        match status.code() {
            None | Some(0) => Ok(()),
            Some(code) => Err(Error::Clone {
                name: self.name.clone(),
                message: format!("git exited with status {}", code),
            }),
        }
    }

    async fn pull(&self) -> Result<Option<String>, Error> {
        let old_revision = self.current_revision().await;

        let status = Command::new("git")
            .arg("pull")
            .current_dir(&self.repository_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(|e| Error::Pull {
                name: self.name.clone(),
                message: e.to_string(),
            })?;

        if let Some(code) = status.code() {
            if code != 0 {
                return Err(Error::Pull {
                    name: self.name.clone(),
                    message: format!("git exited with status {}", code),
                });
            }
        }

        if let Some(old) = old_revision {
            if let Some(new) = self.current_revision().await {
                return Ok(self.log(old, new).await);
            }
        }

        Ok(None)
    }

    pub fn config(&self) -> String {
        format!("try %[ require-module {} ]\n{}\n", self.name, self.config)
    }

    async fn current_revision(&self) -> Option<String> {
        let output = Command::new("git")
            .current_dir(&self.repository_path)
            .args(["rev-parse", "HEAD"])
            .output()
            .await
            .ok()?;

        let mut revision = String::from_utf8_lossy(&output.stdout).to_string();
        revision.pop(); // Remove \n
        Some(revision)
    }

    async fn log(&self, old_revision: String, new_revision: String) -> Option<String> {
        if old_revision == new_revision {
            return None;
        }

        let range = format!("{old_revision}..{new_revision}");

        let output = Command::new("git")
            .current_dir(&self.repository_path)
            .args(["log", &range, "--oneline", "--no-decorate", "--reverse"])
            .output()
            .await
            .ok()?;

        Some(String::from_utf8_lossy(&output.stdout).to_string())
    }
}
