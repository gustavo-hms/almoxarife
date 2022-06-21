use anyhow::anyhow;
use anyhow::bail;
use anyhow::Context;
use anyhow::Error;
use anyhow::Result;
use async_std::fs::File;
use async_std::io::WriteExt;
use async_std::task;
use futures::stream::FuturesUnordered;
use futures::stream::StreamExt;
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
    create_dirs(&xdg)?;
    let mut got_error = false;

    task::block_on(async {
        let mut kak = File::create(xdg.autoload.join("balaio.kak"))
            .await
            .context("Couldn't create kak file")?;

        let mut updates: FuturesUnordered<_> = plugins
            .iter()
            .flat_map(Plugin::iter)
            .map(Plugin::update)
            .collect();

        while let Some(result) = updates.next().await {
            match result {
                Ok(config) => kak
                    .write_all(config.as_bytes())
                    .await
                    .context("Couldn't write kak file")?,

                Err(error) => {
                    println!("{}", error);
                    got_error = true;
                }
            }
        }

        Ok::<(), Error>(())
    })?;

    if got_error {
        Err(anyhow!("Some plugins could not be updated"))
    } else {
        Ok(())
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
                builder = builder.set_url(Url::parse(url.value())?);
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

fn create_dirs(xdg: &Xdg) -> Result<()> {
    if xdg.autoload.metadata().is_ok() {
        fs::remove_dir_all(&xdg.autoload)?;
    }

    fs::create_dir_all(&xdg.autoload)?;

    if xdg.data.metadata().is_err() {
        fs::create_dir_all(&xdg.data)?;
    }

    Ok(())
}
