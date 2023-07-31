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

        let runtime_dir = kakoune.wait_with_output()?;
        let runtime_dir = OsStr::from_bytes(&runtime_dir.stdout);
        unix::fs::symlink(runtime_dir, &self.autoload_dir)?;
        Ok(())
    }

    async fn balaio_config(&self) -> Result<File> {
        File::create(&self.file)
            .await
            .context("Couldn't create balaio.kak file")
    }

    pub async fn create_kak_file_with_prelude(&self) -> Result<File> {
        let mut kak = self.balaio_config().await?;

        kak.write_all(CONFIG_PRELUDE.as_bytes())
            .await
            .context("Couldn't write balaio.kak file");

        Ok(kak)
    }

    pub async fn close_kak_file(mut kak: File) -> Result<()> {
        kak.write_all("🧺".as_bytes())
            .await
            .context("Couldn't write kak file")
    }

    pub async fn write_to_kak(kak: &mut File, data: &[u8]) -> Result<()> {
        kak.write_all(data).await.context("Couldn't write kak file")
    }
}
