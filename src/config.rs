use std::env;
use std::fs;
use std::os::unix;
use std::process::Command;
use std::process::Stdio;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use std::path::Path;
use std::path::PathBuf;

pub struct Config {
    /// The directory where plugins' repos will be checked out (usually `~/.local/share/balaio`).
    pub balaio_data_dir: PathBuf,
    // Directory of `XDG_CONFIG_HOME`
    dir: PathBuf,
    // The Kakoune's autoload directory.
    autoload_dir: PathBuf,
    // The Balaio subdectory inside `autoload`.
    autoload_plugins_dir: PathBuf,
}

impl Config {
    pub fn new() -> Config {
        let home = env::var("HOME").expect("Could not read HOME environment variable");
        let home = Path::new(&home);

        let dir = if let Ok(config) = env::var("XDG_CONFIG_HOME") {
            PathBuf::from(&config)
        } else {
            home.join(".config")
        };

        let balaio_data_dir = if let Ok(data) = env::var("XDG_DATA_HOME") {
            PathBuf::from(&data).join("balaio")
        } else {
            home.join(".local/share/balaio")
        };

        let autoload_dir = dir.join("kak/autoload");
        let mut autoload_plugins_dir = autoload_dir.clone();
        autoload_plugins_dir.push("balaio");

        Config {
            dir,
            autoload_dir,
            autoload_plugins_dir,
            balaio_data_dir,
        }
    }

    pub fn create_dirs(&self) -> Result<()> {
        if !self.autoload_dir.metadata().is_ok() {
            fs::create_dir_all(&self.autoload_dir)?;

            self.link_runtime_dir()
                .context("Unable to detect Kakoune' runtime directory")?;
        }

        if self.autoload_plugins_dir.metadata().is_ok() {
            fs::remove_dir_all(&self.autoload_plugins_dir)?;
        }

        fs::create_dir_all(&self.autoload_plugins_dir)?;

        if self.data_dir.metadata().is_err() {
            fs::create_dir_all(&self.data_dir)?;
        }

        Ok(())
    }

    fn link_runtime_dir(&self) -> Result<()> {
        let kakoune = Command::new("kak")
            .args(["-d", "-s", "balaio", "-E"])
            .arg("echo -to-file /dev/stdout %val[runtime]")
            .stdout(Stdio::piped())
            .spawn()?;

        thread::sleep(Duration::from_millis(100));

        let mut kill = Command::new("kill")
            .args(["-s", "TERM", &kakoune.id().to_string()])
            .spawn()?;

        kill.wait();

        let runtime_dir = kakoune.wait_with_output().unwrap();
        unix::fs::symlink(&runtime_dir, &self.autoload_dir)
    }
}
