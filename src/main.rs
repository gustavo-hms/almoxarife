use std::env;
use std::error;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::fs::File;
use std::mem;
use std::path::Path;
use std::path::PathBuf;
use std::process;
use std::process::Command;
use std::result;
use std::sync::mpsc;
use std::thread;

use colorized::Color;
use colorized::Colors;

use setup::Kak;
use setup::Plugin;
use setup::Setup;
use setup::Status;

use crate::setup::PluginError;

mod setup;
#[cfg(test)]
mod setup_test;

fn main() -> Result<()> {
    let setup = Setup::new();

    match env::args().nth(1) {
        Some(arg) if arg == "--config" => {
            let status = Command::new("kak")
                .arg(&setup.almoxarife_yaml_path)
                .status()
                .context("couldn't run Kakoune")?;

            match status.code() {
                None | Some(0) => (),
                Some(_) => process::exit(1),
            }
        }

        Some(arg) if arg == "--help" || arg == "-h" => {
            println!(
                "A plugin manager for the Kakoune editor.

Usage: al [OPTIONS]

Options:
 --config
        Open the configuration file before updating plugins.

 -h, --help
        Prints this help message.

Running al without any extra option will update your plugins according to the
configuration file."
            );
            return Ok(());
        }

        _ => (),
    }

    let config = setup
        .open_config_file()
        .context("couldn't open almoxarife.yaml")?;

    setup.create_dirs().context("couldn't setup Almoxarife")?;

    let kak = setup
        .create_kak_file_with_prelude()
        .context("couldn't configure plugins")?;

    let disabled_plugins = config.disabled_plugins();
    let removed_plugins = config
        .removed_plugins()
        .context("couldn't delete directories of removed plugins")?;

    manage_plugins(
        config.active_plugins(),
        disabled_plugins,
        removed_plugins,
        kak,
    )
}

fn manage_plugins(
    plugins: Vec<Plugin>,
    disabled_plugins: Vec<String>,
    removed_plugins: Vec<PathBuf>,
    mut kak: Kak<File>,
) -> Result<()> {
    for disabled in disabled_plugins {
        println!("{disabled:>20} {}", "disabled".color(Colors::BrightBlackFg))
    }

    let (sender, receiver) = mpsc::channel();
    let mut errors = Vec::new();
    let mut changes = Vec::new();

    thread::scope(|s| -> Result<()> {
        for plugin in plugins {
            let sender = sender.clone();

            s.spawn(move || {
                let result = plugin.update();
                sender.send(result)
            });
        }

        for removed in removed_plugins {
            let sender = sender.clone();

            s.spawn(move || {
                let result = remove_dir(&removed);
                sender.send(result)
            });
        }

        mem::drop(sender);

        while let Ok(result) = receiver.recv() {
            match result {
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

                    let message: String = log
                        .split("\n")
                        .map(|line| match line.split_once(" ") {
                            Some((revision, message)) => {
                                format!("{} {message}\n", revision.color(Colors::BrightBlackFg))
                            }

                            None => line.to_string(),
                        })
                        .collect();

                    changes.push(format!("{}:\n{message}", name.color(Colors::GreenFg)));
                }

                Ok(Status::Local { name, config }) => {
                    kak.write(config.as_bytes())?;
                    println!("{name:>20} {}", "local".color(Colors::YellowFg))
                }

                Ok(Status::Deleted { name }) => {
                    println!("{name:>20} {}", "removed".color(Colors::CyanFg))
                }

                Err(error) => {
                    println!("{:>20} {}", error.plugin(), "failed".color(Colors::RedFg));
                    errors.push(error);
                }
            }
        }

        Ok(())
    })?;

    kak.close()?;

    if !changes.is_empty() {
        println!("\nUpdates\n");
        println!("{}", changes.join("\n"));
    }

    if !errors.is_empty() {
        eprintln!();
        Err(Error::Plugins(errors))
    } else {
        Ok(())
    }
}

fn remove_dir(path: &Path) -> result::Result<Status, PluginError> {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into();

    match fs::remove_dir_all(path) {
        Ok(_) => Ok(Status::Deleted { name }),
        Err(e) => Err(PluginError::Delete(name, e.to_string())),
    }
}

enum Error {
    Context {
        error: Box<dyn error::Error>,
        context: String,
    },
    Plugins(Vec<setup::PluginError>),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Context { error, context } => write!(f, "{}: {}", context, error),

            Error::Plugins(errors) => {
                let messages: Vec<_> = errors.into_iter().map(|e| e.to_string()).collect();
                write!(f, "\n  {}", messages.join("\n  "))
            }
        }
    }
}

impl Debug for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

impl error::Error for Error {}

impl From<setup::SetupError> for Error {
    fn from(error: setup::SetupError) -> Self {
        Error::Context {
            error: Box::new(error),
            context: "couldn't setup plugins".to_string(),
        }
    }
}

type Result<A> = result::Result<A, Error>;

trait Context<A> {
    fn context(self, message: &str) -> Result<A>;
}

impl<A, E: error::Error + 'static> Context<A> for result::Result<A, E> {
    fn context(self, message: &str) -> Result<A> {
        match self {
            Ok(a) => Ok(a),
            Err(e) => Err(Error::Context {
                error: Box::new(e),
                context: message.to_string(),
            }),
        }
    }
}
