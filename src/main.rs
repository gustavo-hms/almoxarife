use std::env;
use std::error;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs::File;
use std::mem;
use std::process;
use std::process::Command;
use std::result;
use std::sync::mpsc;
use std::thread;

use colorized::Color;
use colorized::Colors;
use setup::Kak;

use plugin::Plugin;
use plugin::Status;
use setup::Setup;

mod plugin;
mod setup;

fn main() -> Result<()> {
    let setup = Setup::new();

    if matches!(env::args().nth(1), Some(arg) if arg == "config") {
        let status = Command::new("kak")
            .arg(&setup.almoxarife_yaml_path)
            .status()
            .context("couldn't run Kakoune")?;

        match status.code() {
            None | Some(0) => (),
            Some(_) => process::exit(1),
        }
    }

    let config = setup
        .open_config_file()
        .context("couldn't open almoxarife.yaml")?;

    let plugins = config.parse_yaml().context(&format!(
        "couldn't parse {}",
        setup.almoxarife_yaml_path.to_str().unwrap()
    ))?;

    setup.create_dirs().context("couldn't setup Almoxarife")?;
    let kak = setup
        .create_kak_file_with_prelude()
        .context("couldn't configure plugins")?;

    manage_plugins(plugins, kak)
}

fn manage_plugins(plugins: Vec<Plugin>, mut kak: Kak<File>) -> Result<()> {
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

impl From<setup::Error> for Error {
    fn from(error: setup::Error) -> Self {
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
