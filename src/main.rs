use anyhow::bail;
use anyhow::Context;
use anyhow::Error;
use anyhow::Result;
use async_std::fs as asyncfs;
use async_std::prelude::FutureExt;
use async_std::task;
use std::fs;
use toml_edit::Document;
use toml_edit::Item;
use toml_edit::Table;
use toml_edit::Value;
use url::Url;

mod plugin;

use plugin::Plugin;
use plugin::Xdg;

fn main() -> Result<()> {
    let xdg = Xdg::new();
    let plugins = parse("balaio.toml", &xdg).context("Couldn't parse balaio.toml")?;

    println!("{:#?}", plugins);

    task::block_on(async { create_dirs(&xdg).await })?;

    Ok(())
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

async fn create_dirs(xdg: &Xdg) -> Result<()> {
    let autoload = async {
        if xdg.autoload.metadata().await.is_ok() {
            asyncfs::remove_dir_all(&xdg.autoload).await?;
        }

        asyncfs::create_dir_all(&xdg.autoload).await?;
        Ok::<(), Error>(())
    };

    let data = async {
        if !xdg.data.metadata().await.is_ok() {
            asyncfs::create_dir_all(&xdg.data).await?;
        }

        Ok::<(), Error>(())
    };

    match autoload.join(data).await {
        (err @ Err(_), _) | (_, err @ Err(_)) => return err,
        _ => Ok(()),
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use async_std::path::PathBuf;

//     #[test]
//     fn check_urls() {
//         let xdg = Xdg {
//             config: PathBuf::from("~/.config"),
//             data: PathBuf::from("~/.local/share"),
//         };

//         let plugin = Plugin::new(
//             String::from("luar"),
//             Url::parse("https://github.com/gustavo-hms/luar").unwrap(),
//             false,
//             String::new(),
//             &xdg,
//         );

//         assert_eq!(
//             plugin.repository_path.to_str().unwrap(),
//             format!("~/.local/share/{}/luar", NAME)
//         );

//         assert_eq!(
//             plugin.link_path.to_str().unwrap(),
//             format!("~/.config/kak/autoload/{}/luar", NAME)
//         );
//     }
// }
