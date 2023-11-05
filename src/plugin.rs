use anyhow::anyhow;
use std::fs;
use std::future::Future;
use std::iter;
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

#[derive(Debug)]
pub struct PluginTree {
    parent: Plugin,
    children: Vec<PluginTree>,
}

impl PluginTree {
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

    pub fn into_iter(self) -> Box<dyn Iterator<Item = Plugin>> {
        if self.parent.disabled {
            return Box::new(iter::empty());
        }

        let children = self
            .children
            .into_iter()
            .flat_map(|child| child.into_iter());

        Box::new(iter::once(self.parent).chain(children))
    }
}

#[derive(Debug)]
pub struct Plugin {
    pub name: String,
    location: Location,
    disabled: bool,
    config: String,
    repository_path: PathBuf,
    link_path: PathBuf,
}

impl Plugin {
    fn repository_path_exists(&self) -> bool {
        fs::metadata(&self.repository_path).is_ok()
    }

    pub fn update(self) -> impl Future<Output = Result<Status, Error>> + Send {
        async move {
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

pub struct PluginBuilder {
    name: String,
    location: Option<Location>,
    disabled: bool,
    config: String,
    repository_path: PathBuf,
    link_path: PathBuf,
    children: Vec<PluginTree>,
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

    pub fn add_child(mut self, child: PluginTree) -> PluginBuilder {
        self.children.push(child);
        self
    }

    pub fn build(mut self) -> anyhow::Result<PluginTree> {
        let location = self
            .location
            .ok_or_else(|| anyhow!("missing `location` field for plugin {}", self.name))?;

        if let Location::Path(path) = &location {
            self.repository_path = path.clone();
        };

        Ok(PluginTree {
            parent: Plugin {
                name: self.name,
                disabled: self.disabled,
                config: self.config,
                repository_path: self.repository_path,
                link_path: self.link_path,
                location,
            },
            children: self.children,
        })
    }
}
