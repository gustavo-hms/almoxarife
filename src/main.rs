use std::env;
use std::error;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::process;
use std::process::Command;
use std::result;

use colorized::Color;
use colorized::Colors;
use config::Kak;
use tokio::runtime::Builder;
use tokio::task::JoinError;
use tokio::task::JoinSet;

use config::Config;
use plugin::Plugin;
use plugin::Status;

mod config;
mod plugin;

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
        .context(&format!("couldn't parse {}", config.file.to_str().unwrap()))?;

    config.create_dirs().context("couldn't setup Almoxarife")?;
    let kak = config
        .create_kak_file_with_prelude()
        .context("couldn't configure plugins")?;

    let runtime = Builder::new_current_thread()
        .enable_io()
        .build()
        .context("runtime didn't start")?;

    runtime.block_on(manage_plugins(plugins, kak))
}

async fn manage_plugins(plugins: Vec<Plugin>, mut kak: Kak) -> Result<()> {
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

            Err(error) => {
                println!("{:>20} {}", error.plugin(), "failed".color(Colors::RedFg));
                errors.push(error);
            }
        }
    }

    kak.close()?;

    if !changes.is_empty() {
        println!("\n{}\n", "Updates".color(Colors::BrightGreenFg));
        println!("{}", changes.join("\n"));
    }

    if !errors.is_empty() {
        eprintln!();
        Err(Error::Plugins(errors))
    } else {
        Ok(())
    }
}

enum Error {
    Context {
        error: Box<dyn error::Error>,
        context: String,
    },
    Plugins(Vec<plugin::Error>),
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

impl From<JoinError> for Error {
    fn from(error: JoinError) -> Self {
        Error::Context {
            error: Box::new(error),
            context: "jobs couldn't be collected".to_string(),
        }
    }
}

impl From<config::Error> for Error {
    fn from(error: config::Error) -> Self {
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
