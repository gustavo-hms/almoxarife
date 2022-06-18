use anyhow::anyhow;
use anyhow::Result;
use async_std::fs;
use async_std::path::PathBuf;
use async_std::process::Child;
use async_std::process::Command;
use std::env;
use std::io::Error;
use std::process::ExitStatus;
use url::Url;

const NAME: &'static str = env!("CARGO_BIN_NAME");

#[derive(Debug)]
pub struct Plugin {
    name: String,
    url: Url,
    disabled: bool,
    config: Option<String>,
    repository_path: PathBuf,
    link_path: PathBuf,
    children: Vec<Plugin>,
}

impl Plugin {
    pub fn builder(name: &str, xdg: &Xdg) -> PluginBuilder {
        let path = format!("{}/{}", NAME, name);
        let repository_path = xdg.data.join(&path);

        let path = format!("kak/autoload/{}", path);
        let link_path = xdg.config.join(path);

        PluginBuilder {
            name: name.to_string(),
            url: None,
            disabled: false,
            config: None,
            repository_path,
            link_path,
            children: Vec::new(),
        }
    }

    async fn repository_path_exists(&self) -> bool {
        fs::metadata(&self.repository_path).await.is_ok()
    }

    pub async fn update(&self) -> Result<ExitStatus, Error> {
        if self.repository_path_exists().await {
            self.pull().await
        } else {
            self.clone_repo().await
        }
    }

    async fn clone_repo(&self) -> Result<ExitStatus, Error> {
        let url = format!("{}.git", self.url);
        Command::new("git")
            .arg("clone")
            .arg(url)
            .arg(&self.repository_path)
            .status()
            .await
    }

    async fn pull(&self) -> Result<ExitStatus, Error> {
        Command::new("git")
            .arg("pull")
            .arg(&self.repository_path)
            .status()
            .await
    }
}

pub struct PluginBuilder {
    name: String,
    url: Option<Url>,
    disabled: bool,
    config: Option<String>,
    repository_path: PathBuf,
    link_path: PathBuf,
    children: Vec<Plugin>,
}

impl PluginBuilder {
    pub fn set_url(mut self, url: Url) -> PluginBuilder {
        self.url = Some(url);
        self
    }

    pub fn set_config(mut self, config: String) -> PluginBuilder {
        self.config = Some(config);
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

    pub fn build(self) -> Result<Plugin> {
        let url = self
            .url
            .ok_or_else(|| anyhow!("Missing `url` field for plugin {}", self.name))?;

        Ok(Plugin {
            name: self.name,
            disabled: self.disabled,
            config: self.config,
            repository_path: self.repository_path,
            link_path: self.link_path,
            children: self.children,
            url,
        })
    }
}

pub struct Xdg {
    config: PathBuf,
    data: PathBuf,
}

impl Xdg {
    pub fn new() -> Xdg {
        let home = env::var("HOME").expect("Could not read HOME environment variable");
        let config = env::var("XDG_CONFIG_HOME").unwrap_or(format!("{home}/.config"));
        let config = PathBuf::from(config);
        let data = env::var("XDG_DATA_HOME").unwrap_or(format!("{home}/.local/share"));
        let data = PathBuf::from(data);

        Xdg { config, data }
    }
}

#[cfg(test)]
mod test {
    use async_std::{fs, prelude::FutureExt, task};
    use url::Url;

    use super::*;

    #[test]
    fn repository_path_exists() {
        let xdg = Xdg::new();
        let plugin = Plugin::builder("balaio", &xdg)
            .set_url(Url::parse("https://github.com/gustavo-hms/balaio").unwrap())
            .build()
            .unwrap();

        task::block_on(async {
            println!("{:?}", &plugin.repository_path);
            println!("{}", plugin.repository_path_exists().await);
        })
    }

    #[test]
    fn update() {
        let xdg = Xdg::new();
        let luar = Plugin::builder("luar", &xdg)
            .set_url(Url::parse("https://github.com/gustavo-hms/luar").unwrap())
            .build()
            .unwrap();

        let peneira = Plugin::builder("peneira", &xdg)
            .set_url(Url::parse("https://github.com/gustavo-hms/peneira").unwrap())
            .build()
            .unwrap();

        task::block_on(async {
            let luar = luar.update();
            let peneira = peneira.update();
            let result = luar.join(peneira).await;
            println!("{:?}", result);
        })
    }
}
