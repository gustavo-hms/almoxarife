use anyhow::anyhow;
use smol::fs;
use smol::fs::unix;
use smol::process::Command;
use smol::process::Stdio;
use std::iter;
use std::path::PathBuf;
use thiserror::Error;
use url::Url;

use crate::config::Config;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Could not clone {0}: {1}")]
    Clone(String, String),
    #[error("Could not update {0}: {1}")]
    Pull(String, String),
    #[error("Could not activate {0}: {1}")]
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

pub enum Status<'a> {
    Installed {
        name: &'a str,
        config: String,
    },
    Updated {
        name: &'a str,
        log: String,
        config: String,
    },
    Unchanged {
        name: &'a str,
        config: String,
    },
    Local {
        name: &'a str,
        config: String,
    },
}

#[derive(Debug)]
pub enum Location {
    Url(Url),
    Path(PathBuf),
}

#[derive(Debug)]
pub struct Plugin {
    pub name: String,
    location: Location,
    disabled: bool,
    config: String,
    repository_path: PathBuf,
    link_path: PathBuf,
    children: Vec<Plugin>,
}

impl Plugin {
    pub fn builder(name: &str, config: &Config) -> PluginBuilder {
        let repository_path = config.almoxarife_data_dir.join(name);
        let link_path = config.autoload_plugins_dir.join(name);

        PluginBuilder {
            name: name.to_string(),
            location: None,
            disabled: false,
            config: String::new(),
            repository_path,
            link_path,
            children: Vec::new(),
        }
    }

    pub fn iter(&self) -> Box<dyn Iterator<Item = &Plugin> + '_> {
        if self.disabled {
            return Box::new(iter::empty());
        }

        let children = self.children.iter().flat_map(|child| child.iter());
        Box::new(iter::once(self).chain(children))
    }

    async fn repository_path_exists(&self) -> bool {
        fs::metadata(&self.repository_path).await.is_ok()
    }

    pub async fn update(&self) -> Result<Status<'_>, Error> {
        let status = match (&self.location, self.repository_path_exists().await) {
            (Location::Url(_), true) => match self.pull().await? {
                None => Status::Unchanged {
                    name: &self.name,
                    config: self.config(),
                },

                Some(log) => Status::Updated {
                    name: &self.name,
                    log,
                    config: self.config(),
                },
            },

            (Location::Url(url), false) => {
                self.clone_repo(url).await?;
                Status::Installed {
                    name: &self.name,
                    config: self.config(),
                }
            }

            _ => Status::Local {
                name: &self.name,
                config: self.config(),
            },
        };

        self.symlink().await?;
        Ok(status)
    }

    async fn symlink(&self) -> Result<(), Error> {
        if !self.disabled {
            unix::symlink(&self.repository_path, &self.link_path)
                .await
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

pub struct PluginBuilder {
    name: String,
    location: Option<Location>,
    disabled: bool,
    config: String,
    repository_path: PathBuf,
    link_path: PathBuf,
    children: Vec<Plugin>,
}

impl PluginBuilder {
    pub fn set_location(mut self, location: &str) -> PluginBuilder {
        match Url::parse(location) {
            Ok(url) => self.location = Some(Location::Url(url)),
            _ => self.location = Some(Location::Path(location.into())),
        };

        self
    }

    pub fn set_config(mut self, config: String) -> PluginBuilder {
        self.config = config;
        self
    }

    pub fn set_disabled(mut self, disabled: bool) -> PluginBuilder {
        self.disabled = disabled;
        self
    }

    pub fn add_child(mut self, child: Plugin) -> PluginBuilder {
        self.children.push(child);
        self
    }

    pub fn build(mut self) -> anyhow::Result<Plugin> {
        let location = self
            .location
            .ok_or_else(|| anyhow!("Missing `location` field for plugin {}", self.name))?;

        if let Location::Path(path) = &location {
            self.repository_path = path.clone();
        };

        Ok(Plugin {
            name: self.name,
            disabled: self.disabled,
            config: self.config,
            repository_path: self.repository_path,
            link_path: self.link_path,
            children: self.children,
            location,
        })
    }
}
