use std::env;
use std::fmt::Display;
use std::process;
use std::process::Command;

use anyhow::anyhow;
use anyhow::Context;
use anyhow::Result;
use async_std::task;
use config::Config;
use futures::stream::FuturesUnordered;
use futures::stream::StreamExt;
use kdam::term::Colorizer;
use kdam::tqdm;
use kdam::Column;
use kdam::RichProgress;
use plugin::Status;

mod config;
mod plugin;

use plugin::Plugin;

fn main() -> Result<()> {
    let config = Config::new();

    if let Some(arg) = env::args().nth(1) {
        if arg == "config" {
            let status = Command::new("kak")
                .arg(&config.file)
                .status()
                .context("couldn't run Kakoune")?;

            match status.code() {
                None | Some(0) => (),
                Some(code) => process::exit(code),
            }
        }
    }

    let plugins = config
        .parse()
        .context(format!("couldn't parse {}", config.file.to_str().unwrap()))?;
    config.create_dirs()?;

    task::block_on(async { manage_plugins(&plugins, &config).await })
}

async fn manage_plugins(plugins: &[Plugin], config: &Config) -> Result<()> {
    let mut kak = config.create_kak_file_with_prelude().await?;

    let mut updates: FuturesUnordered<_> = plugins
        .iter()
        .flat_map(Plugin::iter)
        .map(Plugin::update)
        .collect();

    let mut progress = RichProgress::new(
        tqdm!(total = updates.len()),
        vec![Column::Text("Updating".into(), None), Column::Bar],
    );

    let mut errors = Vec::new();
    let mut changes = Vec::new();

    while let Some(result) = updates.next().await {
        match result {
            Ok(Status::Installed { name, config }) => {
                kak.write(config.as_bytes()).await?;
                progress.write(format!("{name:>20} {}", "installed".colorize("green")))
            }

            Ok(Status::Unchanged { name, config }) => {
                kak.write(config.as_bytes()).await?;
                progress.write(format!("{name:>20} {}", "unchanged".colorize("blue")))
            }

            Ok(Status::Updated { name, log, config }) => {
                kak.write(config.as_bytes()).await?;
                progress.write(format!("{name:>20} {}", "updated".colorize("green")));

                changes.push(
                    Change {
                        plugin: name.to_string(),
                        log,
                    }
                    .to_string(),
                );
            }

            Ok(Status::Local { name, config }) => {
                kak.write(config.as_bytes()).await?;
                progress.write(format!("{name:>20} {}", "local".colorize("yellow")))
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

    if !changes.is_empty() {
        println!("Updates\n-------\n");
        println!("{}", changes.join("\n"));
    }

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

struct Change {
    plugin: String,
    log: String,
}

impl Display for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}:\n\n{}\n", self.plugin, self.log)
    }
}
