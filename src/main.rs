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
use kdam::tqdm;
use std::fs;
use std::path::Path;
use yaml_rust::yaml::Hash;
use yaml_rust::yaml::Yaml;
use yaml_rust::yaml::YamlLoader;

mod plugin;

use plugin::Plugin;
use plugin::Xdg;

fn main() -> Result<()> {
    let xdg = Xdg::new();
    let balaio = xdg.config.join("balaio.yaml");
    let plugins =
        parse(&balaio, &xdg).context(format!("Couldn't parse {}", balaio.to_str().unwrap()))?;

    create_dirs(&xdg)?;

    let mut got_error = false;

    task::block_on(async {
        let mut kak = File::create(xdg.autoload.join("balaio.kak"))
            .await
            .context("Couldn't create kak file")?;

        kak.write_all(CONFIG_PRELUDE.as_bytes())
            .await
            .context("Couldn't write kak file")?;

        let mut updates: FuturesUnordered<_> = plugins
            .iter()
            .flat_map(Plugin::iter)
            .map(Plugin::update)
            .collect();

        let mut progress = tqdm!(total = updates.len());

        while let Some(result) = updates.next().await {
            match result {
                Ok((name, config)) => {
                    kak.write_all(config.as_bytes())
                        .await
                        .context("Couldn't write kak file")?;
                    progress.write(format!("  {name} updated"))
                }

                Err(error) => {
                    eprintln!("{}", error);
                    got_error = true;
                }
            }

            progress.update(1);
        }

        // Close top level block
        kak.write_all("🧺".as_bytes())
            .await
            .context("Couldn't write kak file")?;
        Ok::<(), Error>(())
    })?;

    if got_error {
        Err(anyhow!("Some plugins could not be updated"))
    } else {
        Ok(())
    }
}

const CONFIG_PRELUDE: &str = r#"
hook global KakBegin .* %🧺

add-highlighter shared/balaio regions
add-highlighter shared/balaio/ region '^\s*config:\s+\|' '^\s*\w+:' ref kakrc
add-highlighter shared/balaio/ region '^\s*config:[^\n]' '\n' ref kakrc

hook -group balaio global WinCreate .*balaio[.]yaml %{
    add-highlighter window/balaio ref balaio
    hook -once -always window WinClose .* %{ remove-highlighter window/balaio }
}
"#;

fn parse<P: AsRef<Path>>(file: P, xdg: &Xdg) -> Result<Vec<Plugin>> {
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
                    plugins.push(build_plugin(key, hash, xdg)?);
                } else {
                    bail!("Unexpected field {element:?}")
                }
            }
        }

        _ => bail!("Couldn't parse configuration file"),
    }

    Ok(plugins)
}

fn build_plugin(name: &str, hash: &Hash, xdg: &Xdg) -> Result<Plugin> {
    let mut builder = Plugin::builder(name, xdg);

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
                let child = build_plugin(key, hash, xdg)?;
                builder = builder.add_child(child);
            }

            _ => bail!("Unexpected value: `{key:?}: {value:?}`"),
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
