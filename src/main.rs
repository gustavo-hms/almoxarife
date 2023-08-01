use anyhow::anyhow;
use anyhow::bail;
use anyhow::Context;
use anyhow::Error;
use anyhow::Result;
use async_std::fs::File;
use async_std::io::WriteExt;
use async_std::task;
use config::Config;
use futures::stream::FuturesUnordered;
use futures::stream::StreamExt;
use kdam::term::Colorizer;
use kdam::tqdm;
use kdam::Column;
use kdam::RichProgress;
use plugin::Status;
use std::fs;
use std::path::Path;
use yaml_rust::yaml::Hash;
use yaml_rust::yaml::Yaml;
use yaml_rust::yaml::YamlLoader;

mod config;
mod plugin;

use plugin::Plugin;

fn main() -> Result<()> {
    let config = Config::new();
    let plugins = parse(&config.file, &config)
        .context(format!("Couldn't parse {}", config.file.to_str().unwrap()))?;
    config.create_dirs()?;

    task::block_on(async { manage_plugins(&plugins, &config).await })
}

async fn manage_plugins(plugins: &[Plugin], config: &Config) -> Result<()> {
    let mut kak = config.create_kak_file_with_prelude().await?;
    let mut errors = Vec::new();

    let mut updates: FuturesUnordered<_> = plugins
        .iter()
        .flat_map(Plugin::iter)
        .map(Plugin::update)
        .collect();

    let mut progress = RichProgress::new(
        tqdm!(total = updates.len()),
        vec![Column::Text("Updating".into(), None), Column::Bar],
    );

    while let Some(result) = updates.next().await {
        match result {
            Ok(Status::Installed { name, config }) => {
                kak.write(config.as_bytes()).await?;
                progress.write(format!("{name:>20} {}", "installed".colorize("green")))
            }

            Ok(Status::Updated { name, config }) => {
                kak.write(config.as_bytes()).await?;
                progress.write(format!("{name:>20} {}", "updated".colorize("green")))
            }

            Ok(Status::NoChange { name, config }) => {
                kak.write(config.as_bytes()).await?;
                progress.write(format!("{name:>20} {}", "unchanged".colorize("blue")))
            }

            Err(error) => {
                let message = format!("{:>20} {}", error.plugin(), "failed".colorize("red"));
                progress.write(message);
                errors.push(error.to_string());
            }
        }

        progress.update(1);
    }

    kak.close().await?;
    progress.clear();

    if !errors.is_empty() {
        eprintln!();
        Err(anyhow!(
            "some plugins could not be updated:\n  {}",
            errors.join("\n  ")
        ))
    } else {
        Ok(())
    }
}

fn parse<P: AsRef<Path>>(file: P, config: &Config) -> Result<Vec<Plugin>> {
    let yaml = fs::read_to_string(file)?;
    let doc = YamlLoader::load_from_str(&yaml)?;

    if doc.is_empty() {
        bail!("Configuration file has no YAML element");
    }

    let mut plugins = Vec::new();

    match &doc[0] {
        Yaml::Hash(hash) => {
            for element in hash.iter() {
                if let (Yaml::String(key), Yaml::Hash(hash)) = element {
                    plugins.push(build_plugin(key, hash, config)?);
                } else {
                    bail!("Unexpected field {element:?}")
                }
            }
        }

        _ => bail!("Couldn't parse configuration file"),
    }

    Ok(plugins)
}

fn build_plugin(name: &str, hash: &Hash, config: &Config) -> Result<Plugin> {
    let mut builder = Plugin::builder(name, config);

    for (key, value) in hash.iter() {
        match (key.as_str(), value) {
            (Some("location"), Yaml::String(location)) => {
                builder = builder.set_location(location);
            }

            (Some("location"), _) => {
                bail!("Expecting a string for the `location` field of plugin {name}")
            }

            (Some("disabled"), Yaml::Boolean(disabled)) => {
                builder = builder.set_disabled(*disabled);
            }

            (Some("disabled"), _) => {
                bail!("Expecting a boolean for the `disabled` field of plugin {name}")
            }

            (Some("config"), Yaml::String(config)) => {
                builder = builder.set_config(config.clone());
            }

            (Some("config"), _) => {
                bail!("Expecting a string for the `config` field of plugin {name}")
            }

            (Some(key), Yaml::Hash(hash)) => {
                let child = build_plugin(key, hash, config)?;
                builder = builder.add_child(child);
            }

            _ => bail!("Unexpected value: `{key:?}: {value:?}`"),
        }
    }

    builder.build()
}
