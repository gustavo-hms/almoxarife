use std::env;
use std::process;
use std::process::Command;

use anyhow::anyhow;
use anyhow::Context;
use anyhow::Result;
use colorized::Color;
use colorized::Colors;
use config::Config;
use plugin::Status;

mod config;
mod plugin;

use plugin::Plugin;
use tokio::runtime::Builder;
use tokio::task::JoinSet;

fn main() -> Result<()> {
    let config = Config::new();

    if matches!(env::args().nth(1), Some(arg) if arg == "config") {
        let status = Command::new("kak")
            .arg(&config.file)
            .status()
            .context("couldn't run Kakoune")?;

        match status.code() {
            None | Some(0) => (),
            Some(code) => process::exit(code),
        }
    }

    let plugins = config
        .parse()
        .context(format!("couldn't parse {}", config.file.to_str().unwrap()))?;

    config.create_dirs()?;

    let runtime = Builder::new_current_thread().enable_io().build()?;
    runtime.block_on(manage_plugins(plugins, &config))
}

async fn manage_plugins(plugins: Vec<Plugin>, config: &Config) -> Result<()> {
    let mut kak = config.create_kak_file_with_prelude()?;
    let mut set = JoinSet::new();

    for plugin in plugins {
        set.spawn(plugin.update());
    }

    let mut errors = Vec::new();
    let mut changes = Vec::new();

    while let Some(result) = set.join_next().await {
        match result? {
            Ok(Status::Installed { name, config }) => {
                kak.write(config.as_bytes())?;
                println!("{name:>20} {}", "installed".color(Colors::GreenFg))
            }

            Ok(Status::Unchanged { name, config }) => {
                kak.write(config.as_bytes())?;
                println!("{name:>20} {}", "unchanged".color(Colors::BlueFg))
            }

            Ok(Status::Updated { name, log, config }) => {
                kak.write(config.as_bytes())?;
                println!("{name:>20} {}", "updated".color(Colors::GreenFg));
                changes.push(format!("{}:\n\n{}\n", name, log));
            }

            Ok(Status::Local { name, config }) => {
                kak.write(config.as_bytes())?;
                println!("{name:>20} {}", "local".color(Colors::YellowFg))
            }

            Err(error) => {
                println!("{:>20} {}", error.plugin(), "failed".color(Colors::RedFg));
                errors.push(error.to_string());
            }
        }
    }

    kak.close()?;

    if !changes.is_empty() {
        println!("Updates");
        println!("-------\n");
        println!("{}", changes.join("\n"));
    }

    if !errors.is_empty() {
        eprintln!();
        Err(anyhow!("\n  {}", errors.join("\n  ")))
    } else {
        Ok(())
    }
}
