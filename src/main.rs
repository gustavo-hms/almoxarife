use std::env;
use std::process;
use std::process::Command;
use std::sync::mpsc;
use std::thread;

use anyhow::anyhow;
use anyhow::Context;
use anyhow::Result;
use config::Config;
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
    manage_plugins(&plugins, &config)
}

fn manage_plugins(plugins: &[Plugin], config: &Config) -> Result<()> {
    let mut kak = config.create_kak_file_with_prelude()?;
    let plugins: Vec<_> = plugins.iter().flat_map(Plugin::iter).collect();
    let number_of_plugins = plugins.len();

    thread::scope(|s| {
        let (tx, rx) = mpsc::channel();

        for plugin in plugins {
            let tx = tx.clone();

            s.spawn(move || {
                let result = plugin.update();
                tx.send(result)
            });
        }

        let mut errors = Vec::new();
        let mut changes = Vec::new();

        let mut progress = RichProgress::new(
            tqdm!(total = number_of_plugins),
            vec![Column::Text("Updating".into(), None), Column::Bar],
        );

        for _ in 0..number_of_plugins {
            match rx.recv()? {
                Ok(Status::Installed { name, config }) => {
                    kak.write(config.as_bytes())?;
                    progress.write(format!("{name:>20} {}", "installed".colorize("green")))
                }

                Ok(Status::Unchanged { name, config }) => {
                    kak.write(config.as_bytes())?;
                    progress.write(format!("{name:>20} {}", "unchanged".colorize("blue")))
                }

                Ok(Status::Updated { name, log, config }) => {
                    kak.write(config.as_bytes())?;
                    progress.write(format!("{name:>20} {}", "updated".colorize("green")));
                    changes.push(format!("{}:\n\n{}\n", name, log));
                }

                Ok(Status::Local { name, config }) => {
                    kak.write(config.as_bytes())?;
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

        kak.close()?;
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
    })
}
