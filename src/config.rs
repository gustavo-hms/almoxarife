use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::os::unix;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::thread;
use std::time::Duration;

use crate::plugin::Plugin;
use crate::plugin::PluginTree;

const CONFIG_PRELUDE: &str = r"
hook global KakBegin .* %🧺

add-highlighter shared/almoxarife regions
add-highlighter shared/almoxarife/ region '^\s*config:\s+\|' '^\s*\w+:' ref kakrc
add-highlighter shared/almoxarife/ region '^\s*config:[^\n]' '\n' ref kakrc

hook -group almoxarife global WinCreate .*almoxarife[.]yaml %{
    add-highlighter window/almoxarife ref almoxarife
    hook -once -always window WinClose .* %{ remove-highlighter window/almoxarife }
}
";

pub struct Config {
    /// The directory where plugins' repos will be checked out (usually `~/.local/share/almoxarife`).
    pub almoxarife_data_dir: PathBuf,
    // The Almoxarife subdirectory inside `autoload`.
    pub autoload_plugins_dir: PathBuf,
    /// The path to `almoxarife.yaml`.
    pub file: PathBuf,
    /// The path to `almoxarife.kak`
    almoxarife_kak_file: PathBuf,
    // The Kakoune's autoload directory.
    autoload_dir: PathBuf,
}

impl Config {
    pub fn new() -> Config {
        let home = env::var("HOME").expect("could not read HOME environment variable");
        let home = Path::new(&home);

        let config_dir = if let Ok(config) = env::var("XDG_CONFIG_HOME") {
            PathBuf::from(&config)
        } else {
            home.join(".config")
        };

        let file = config_dir.join("almoxarife.yaml");

        let almoxarife_data_dir = if let Ok(data) = env::var("XDG_DATA_HOME") {
            PathBuf::from(&data).join("almoxarife")
        } else {
            home.join(".local/share/almoxarife")
        };

        let autoload_dir = config_dir.join("kak/autoload");
        let mut autoload_plugins_dir = autoload_dir.clone();
        autoload_plugins_dir.push("almoxarife");
        let almoxarife_kak_file = autoload_plugins_dir.join("almoxarife.kak");

        Config {
            file,
            almoxarife_kak_file,
            autoload_dir,
            autoload_plugins_dir,
            almoxarife_data_dir,
        }
    }

    pub fn parse(&self) -> Result<Vec<Plugin>> {
        let file = File::open(&self.file)?;
        let tree: HashMap<String, PluginTree> = serde_yaml::from_reader(&file)?;

        if tree.is_empty() {
            bail!("configuration file has no YAML element");
        }

        let plugins = tree
            .into_iter()
            .flat_map(|(name, tree)| tree.plugins(name, self))
            .collect();

        Ok(plugins)
    }

    pub fn create_dirs(&self) -> Result<()> {
        if self.autoload_dir.metadata().is_err() {
            fs::create_dir_all(&self.autoload_dir)?;

            self.link_runtime_dir()
                .context("unable to detect Kakoune's runtime directory")?;
        }

        if self.autoload_plugins_dir.metadata().is_ok() {
            fs::remove_dir_all(&self.autoload_plugins_dir)?;
        }

        fs::create_dir_all(&self.autoload_plugins_dir)?;

        if self.almoxarife_data_dir.metadata().is_err() {
            fs::create_dir_all(&self.almoxarife_data_dir)?;
        }

        Ok(())
    }

    fn link_runtime_dir(&self) -> Result<()> {
        let mut kakoune = Command::new("kak")
            .args(["-d", "-s", "almoxarife", "-E"])
            .arg("echo -to-file /dev/stdout %val[runtime]")
            .stdout(Stdio::piped())
            .spawn()?;

        thread::sleep(Duration::from_millis(100));

        kakoune.kill()?;
        let output = kakoune.wait_with_output()?;
        let runtime_dir = OsStr::from_bytes(&output.stdout);
        let runtime_dir = PathBuf::from(runtime_dir).join("rc");
        unix::fs::symlink(runtime_dir, self.autoload_dir.join("rc"))?;

        Ok(())
    }

    fn create_kak_file(&self) -> Result<Kak> {
        let file = File::create(&self.almoxarife_kak_file)
            .context("couldn't create almoxarife.kak file")?;

        Ok(Kak(file))
    }

    pub fn create_kak_file_with_prelude(&self) -> Result<Kak> {
        let mut kak = self.create_kak_file()?;
        kak.write(CONFIG_PRELUDE.as_bytes())?;
        Ok(kak)
    }
}

pub struct Kak(File);

impl Kak {
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.0.write_all(data).context("couldn't write kak file")
    }

    pub fn close(&mut self) -> Result<()> {
        self.0
            .write_all("🧺".as_bytes())
            .context("couldn't write kak file")
    }
}
