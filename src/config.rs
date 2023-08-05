use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use async_std::fs::File;
use async_std::io::WriteExt;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::os::unix;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::thread;
use std::time::Duration;
use yaml_rust::yaml::Hash;
use yaml_rust::Yaml;
use yaml_rust::YamlLoader;

use crate::plugin::Plugin;

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

pub struct Config {
    /// The directory where plugins' repos will be checked out (usually `~/.local/share/balaio`).
    pub balaio_data_dir: PathBuf,
    /// The path to `balaio.yaml`.
    pub file: PathBuf,
    // The Balaio subdectory inside `autoload`.
    pub autoload_plugins_dir: PathBuf,
    // Path of `XDG_CONFIG_HOME`
    dir: PathBuf,
    // The Kakoune's autoload directory.
    autoload_dir: PathBuf,
}

impl Config {
    pub fn new() -> Config {
        let home = env::var("HOME").expect("Could not read HOME environment variable");
        let home = Path::new(&home);

        let config_dir = if let Ok(config) = env::var("XDG_CONFIG_HOME") {
            PathBuf::from(&config)
        } else {
            home.join(".config")
        };

        let balaio_data_dir = if let Ok(data) = env::var("XDG_DATA_HOME") {
            PathBuf::from(&data).join("balaio")
        } else {
            home.join(".local/share/balaio")
        };

        let autoload_dir = config_dir.join("kak/autoload");
        let mut autoload_plugins_dir = autoload_dir.clone();
        autoload_plugins_dir.push("balaio");
        let file = autoload_plugins_dir.join("balaio.kak");

        Config {
            dir: config_dir,
            file,
            autoload_dir,
            autoload_plugins_dir,
            balaio_data_dir,
        }
    }

    pub fn parse(&self) -> Result<Vec<Plugin>> {
        let yaml = fs::read_to_string(&self.file)?;
        let doc = YamlLoader::load_from_str(&yaml)?;

        if doc.is_empty() {
            bail!("Configuration file has no YAML element");
        }

        let mut plugins = Vec::new();

        match &doc[0] {
            Yaml::Hash(hash) => {
                for element in hash.iter() {
                    if let (Yaml::String(key), Yaml::Hash(hash)) = element {
                        plugins.push(self.build_plugin(key, hash)?);
                    } else {
                        bail!("Unexpected field {element:?}")
                    }
                }
            }

            _ => bail!("Couldn't parse configuration file"),
        }

        Ok(plugins)
    }

    fn build_plugin(&self, name: &str, hash: &Hash) -> Result<Plugin> {
        let mut builder = Plugin::builder(name, self);

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
                    let child = self.build_plugin(key, hash)?;
                    builder = builder.add_child(child);
                }

                _ => bail!("Unexpected value: `{key:?}: {value:?}`"),
            }
        }

        builder.build()
    }

    pub fn create_dirs(&self) -> Result<()> {
        if self.autoload_dir.metadata().is_err() {
            fs::create_dir_all(&self.autoload_dir)?;

            self.link_runtime_dir()
                .context("Unable to detect Kakoune's runtime directory")?;
        }

        if self.autoload_plugins_dir.metadata().is_ok() {
            fs::remove_dir_all(&self.autoload_plugins_dir)?;
        }

        fs::create_dir_all(&self.autoload_plugins_dir)?;

        if self.balaio_data_dir.metadata().is_err() {
            fs::create_dir_all(&self.balaio_data_dir)?;
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

        kill.wait()?;

        let output = kakoune.wait_with_output()?;
        let runtime_dir = OsStr::from_bytes(&output.stdout);
        let mut runtime_dir = PathBuf::from(runtime_dir);
        runtime_dir.push("rc");
        unix::fs::symlink(&runtime_dir, &self.autoload_dir)?;
        Ok(())
    }

    async fn balaio_config(&self) -> Result<Kak> {
        let file = File::create(&self.file)
            .await
            .context("Couldn't create balaio.kak file")?;

        Ok(Kak(file))
    }

    pub async fn create_kak_file_with_prelude(&self) -> Result<Kak> {
        let mut kak = self.balaio_config().await?;
        kak.write(CONFIG_PRELUDE.as_bytes()).await?;
        Ok(kak)
    }
}

pub struct Kak(File);

impl Kak {
    pub async fn write(&mut self, data: &[u8]) -> Result<()> {
        self.0
            .write_all(data)
            .await
            .context("Couldn't write kak file")
    }

    pub async fn close(mut self) -> Result<()> {
        self.0
            .write_all("🧺".as_bytes())
            .await
            .context("Couldn't write kak file")
    }
}
