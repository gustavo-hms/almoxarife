use anyhow::anyhow;
use anyhow::Context;
use anyhow::Result;
use async_std::fs;
use async_std::os::unix;
use async_std::process::Command;
use async_std::process::Stdio;
use std::env;
use std::iter;
use std::path::Path;
use std::path::PathBuf;
use url::Url;

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
    pub fn builder(name: &str, xdg: &Xdg) -> PluginBuilder {
        let repository_path = xdg.data.join(name);
        let link_path = xdg.autoload.join(name);

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

    pub async fn update(&self) -> Result<(&str, String)> {
        if let Location::Url(url) = &self.location {
            if self.repository_path_exists().await {
                self.pull().await?;
            } else {
                self.clone_repo(url).await?;
            }
        }

        self.symlink().await?;
        Ok((&self.name, self.config()))
    }

    async fn symlink(&self) -> Result<()> {
        if !self.disabled {
            unix::fs::symlink(&self.repository_path, &self.link_path)
                .await
                .with_context(|| format!("Couldn't activate the plugin {}", self.name))?;
        }

        // println!("Plugin {} updated.", self.name);
        Ok(())
    }

    async fn clone_repo(&self, url: &Url) -> Result<()> {
        let location = format!("{}.git", url);

        let status = Command::new("git")
            .arg("clone")
            .arg(location)
            .arg(&self.repository_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await?;

        match status.code() {
            None | Some(0) => Ok(()),
            Some(code) => Err(anyhow!(
                "Git exited with status {code} while cloning {}",
                self.name
            )),
        }
    }

    async fn pull(&self) -> Result<()> {
        let status = Command::new("git")
            .arg("pull")
            .current_dir(&self.repository_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await?;

        match status.code() {
            None | Some(0) => Ok(()),
            Some(code) => Err(anyhow!(
                "Git exited with status {code} while pulling from {}",
                self.name
            )),
        }
    }

    pub fn config(&self) -> String {
        format!("try %[ require-module {} ]\n{}\n", self.name, self.config)
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

    pub fn build(mut self) -> Result<Plugin> {
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

pub struct Xdg {
    pub config: PathBuf,
    pub autoload: PathBuf,
    pub data: PathBuf,
}

impl Xdg {
    pub fn new() -> Xdg {
        let home = env::var("HOME").expect("Could not read HOME environment variable");
        let home = Path::new(&home);

        let config = if let Ok(config) = env::var("XDG_CONFIG_HOME") {
            PathBuf::from(&config)
        } else {
            home.join(".config")
        };

        let data = if let Ok(data) = env::var("XDG_DATA_HOME") {
            PathBuf::from(&data).join("balaio")
        } else {
            home.join(".local/share/balaio")
        };

        let autoload = config.join("kak/autoload/balaio");

        Xdg {
            config,
            autoload,
            data,
        }
    }
}

#[cfg(test)]
mod test {
    use async_std::path::PathBuf;

    use async_std::{prelude::FutureExt, task};
    use url::Url;

    use super::*;

    #[test]
    fn update() {
        let config = PathBuf::from(tempfile::tempdir().unwrap().path());
        let data = PathBuf::from(tempfile::tempdir().unwrap().path());

        let xdg = Xdg {
            config: config.clone(),
            autoload: config.clone(),
            data: data.clone(),
        };

        let luar = Plugin::builder("luar", &xdg)
            .set_location(Url::parse("https://github.com/gustavo-hms/luar").unwrap())
            .build()
            .unwrap();

        let peneira = Plugin::builder("peneira", &xdg)
            .set_location(Url::parse("https://github.com/gustavo-hms/peneira").unwrap())
            .build()
            .unwrap();

        task::block_on(async {
            let first = luar.update();
            let second = peneira.update();
            let (first, second) = first.join(second).await;

            assert!(first.unwrap().success());
            assert!(second.unwrap().success());
            assert!(data.join("luar").metadata().await.is_ok());
            assert!(data.join("peneira").metadata().await.is_ok());

            // Check if the second call to update doesn't try to clone again
            let first = luar.update();
            let second = peneira.update();
            let (first, second) = first.join(second).await;

            assert!(first.unwrap().success());
            assert!(second.unwrap().success());
        })
    }
}
