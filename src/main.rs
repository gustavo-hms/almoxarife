use std::env;
use std::fs;

use anyhow::anyhow;
use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use async_std::path::PathBuf;
use toml_edit::Document;
use toml_edit::Item;
use toml_edit::Table;
use toml_edit::Value;
use url::Url;

const NAME: &'static str = env!("CARGO_BIN_NAME");

#[derive(Clone)]
struct Xdg {
    config: PathBuf,
    data: PathBuf,
}

impl Xdg {
    fn new() -> Xdg {
        let config = env::var("XDG_CONFIG_HOME").unwrap_or(String::from("~/.config"));
        let config = PathBuf::from(config);
        let data = env::var("XDG_DATA_HOME").unwrap_or(String::from("~/.local/share"));
        let data = PathBuf::from(data);

        Xdg { config, data }
    }
}

#[derive(Debug)]
struct Plugin {
    name: String,
    url: Url,
    disabled: bool,
    config: Option<String>,
    repository_path: PathBuf,
    link_path: PathBuf,
    children: Vec<Plugin>,
}

impl Plugin {
    fn builder(name: &str, xdg: &Xdg) -> PluginBuilder {
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

    // fn clone_repo(&self) -> Result<Child, Error> {
    //     let url = format!("{}.git", self.url);
    //     Command::new("git").arg("clone").arg(url).spawn()
    // }

    // fn update(&self) -> Result<Child, Error> {
    //     Command::new("git").arg("pull").spawn()
    // }
}

struct PluginBuilder {
    name: String,
    url: Option<Url>,
    disabled: bool,
    config: Option<String>,
    repository_path: PathBuf,
    link_path: PathBuf,
    children: Vec<Plugin>,
}

impl PluginBuilder {
    fn set_url(mut self, url: Url) -> PluginBuilder {
        self.url = Some(url);
        self
    }

    fn set_config(mut self, config: String) -> PluginBuilder {
        self.config = Some(config);
        self
    }

    fn set_disabled(mut self, disabled: bool) -> PluginBuilder {
        self.disabled = disabled;
        self
    }

    fn add_child(mut self, child: Plugin) -> PluginBuilder {
        self.children.push(child);
        self
    }

    fn build(self) -> Result<Plugin> {
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

fn parse(file: &str, xdg: &Xdg) -> Result<Vec<Plugin>> {
    let toml = fs::read_to_string(file)?;
    let doc = toml.parse::<Document>()?;
    let mut plugins = Vec::new();

    for (key, value) in doc.iter() {
        if let Item::Table(table) = value {
            plugins.push(build_plugin(key, table, xdg)?);
        } else {
            bail!("Unexpected field {key}")
        }
    }

    Ok(plugins)
}

fn build_plugin(name: &str, table: &Table, xdg: &Xdg) -> Result<Plugin> {
    let mut builder = Plugin::builder(name, xdg);

    for element in table.iter() {
        match element {
            ("url", Item::Value(Value::String(url))) => {
                builder = builder.set_url(Url::parse(&url.value())?);
            }

            ("url", _) => bail!("Expecting a string for the `url` field of plugin {name}"),

            ("disabled", Item::Value(Value::Boolean(disabled))) => {
                builder = builder.set_disabled(*disabled.value());
            }

            ("disabled", _) => {
                bail!("Expecting a boolean for the `disabled` field of plugin {name}")
            }

            ("config", Item::Value(Value::String(config))) => {
                builder = builder.set_config(config.value().clone());
            }

            ("config", _) => {
                bail!("Expecting a string for the `config` field of plugin {name}")
            }

            (key, Item::Table(table)) => {
                let child = build_plugin(key, table, xdg)?;
                builder = builder.add_child(child);
            }

            (key, value) => bail!("Unexpected value: `{name}.{key} = {value}`"),
        }
    }

    builder.build()
}

fn main() -> Result<()> {
    let plugins = parse(
        "balaio.toml",
        &Xdg {
            config: PathBuf::from("~/.config"),
            data: PathBuf::from("~/.local/share"),
        },
    )
    .context("Couldn't parse balaio.toml")?;

    println!("{:#?}", plugins);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_std::path::PathBuf;

    #[test]
    fn check_urls() {
        let xdg = Xdg {
            config: PathBuf::from("~/.config"),
            data: PathBuf::from("~/.local/share"),
        };

        let plugin = Plugin::new(
            String::from("luar"),
            Url::parse("https://github.com/gustavo-hms/luar").unwrap(),
            false,
            String::new(),
            &xdg,
        );

        assert_eq!(
            plugin.repository_path.to_str().unwrap(),
            format!("~/.local/share/{}/luar", NAME)
        );

        assert_eq!(
            plugin.link_path.to_str().unwrap(),
            format!("~/.config/kak/autoload/{}/luar", NAME)
        );
    }
}
