use anyhow::anyhow;
use anyhow::Result;
use async_std::path::PathBuf;
use std::env;
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

    // pub fn update(&self) -> Result<Child, Error> {}

    // fn clone_repo(&self) -> Result<Child, Error> {
    //     let url = format!("{}.git", self.url);
    //     Command::new("git").arg("clone").arg(url).spawn()
    // }

    // fn update(&self) -> Result<Child, Error> {
    //     Command::new("git").arg("pull").spawn()
    // }
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
        let config = env::var("XDG_CONFIG_HOME").unwrap_or(String::from("~/.config"));
        let config = PathBuf::from(config);
        let data = env::var("XDG_DATA_HOME").unwrap_or(String::from("~/.local/share"));
        let data = PathBuf::from(data);

        Xdg { config, data }
    }
}
