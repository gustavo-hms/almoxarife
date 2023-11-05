use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::os::unix;
use std::path::PathBuf;
use std::process::Stdio;
use thiserror::Error;
use tokio::process::Command;
use url::Url;

use crate::config::Config;

#[derive(Error, Debug)]
pub enum Error {
    #[error("could not clone {0}: {1}")]
    Clone(String, String),
    #[error("could not update {0}: {1}")]
    Pull(String, String),
    #[error("could not activate {0}: {1}")]
    Link(String, String),
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

#[derive(Debug)]
pub enum Location {
    Url(Url),
    Path(PathBuf),
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

#[derive(Debug)]
pub struct Plugin {
    name: String,
    location: Location,
    disabled: bool,
    config: String,
    repository_path: PathBuf,
    link_path: PathBuf,
}

impl Plugin {
    fn new(name: String, node: &PluginTree, config: &Config) -> Plugin {
        let link_path = config.autoload_plugins_dir.join(&name);

        let (location, repository_path) = match Url::parse(&node.location) {
            Ok(url) => (Location::Url(url), config.almoxarife_data_dir.join(&name)),

            Err(_) => {
                let path: PathBuf = (&node.location).into();
                (Location::Path(path.clone()), path)
            }
        };

        Plugin {
            name,
            disabled: node.disabled,
            config: node.config.clone(),
            location,
            repository_path,
            link_path,
        }
    }

    fn repository_path_exists(&self) -> bool {
        fs::metadata(&self.repository_path).is_ok()
    }

    pub async fn update(self) -> Result<Status, Error> {
        let config = self.config();
        let name = self.name.clone();

        let status = match (&self.location, self.repository_path_exists()) {
            (Location::Url(_), true) => match self.pull().await? {
                None => Status::Unchanged { name, config },
                Some(log) => Status::Updated { name, log, config },
            },

            (Location::Url(url), false) => {
                self.clone_repo(url).await?;
                Status::Installed { name, config }
            }

            _ => Status::Local { name, config },
        };

        self.symlink().await?;
        Ok(status)
    }

    async fn symlink(&self) -> Result<(), Error> {
        if !self.disabled {
            unix::fs::symlink(&self.repository_path, &self.link_path)
                .map_err(|e| Error::Link(self.name.clone(), e.to_string()))?;
        }

        Ok(())
    }

    async fn clone_repo(&self, url: &Url) -> Result<(), Error> {
        let location = format!("{}.git", url);

        let status = Command::new("git")
            .arg("clone")
            .arg(location)
            .arg(&self.repository_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(|e| Error::Clone(self.name.clone(), e.to_string()))?;

        match status.code() {
            None | Some(0) => Ok(()),
            Some(code) => Err(Error::Clone(
                self.name.clone(),
                format!("git exited with status {}", code),
            )),
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
            .args(["rev-parse", "HEAD"])
            .output()
            .await
            .ok()?;

        Some(String::from_utf8_lossy(&output.stdout).to_string())
    }

    async fn log(&self, old_revision: String, new_revision: String) -> Option<String> {
        if old_revision == new_revision {
            return None;
        }

        let range = format!("{old_revision}..{new_revision}");

        let output = Command::new("git")
            .args(["log", &range, "--oneline", "--no-decorate", "--reverse"])
            .output()
            .await
            .ok()?;

        Some(String::from_utf8_lossy(&output.stdout).to_string())
    }
}
