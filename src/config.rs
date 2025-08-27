use std::collections::HashMap;
use std::env;
use std::error;
use std::ffi::OsStr;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::fs::File;
use std::io;
use std::io::Write;
use std::os::unix;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::result;
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

#[derive(Debug)]
pub struct Error(String);

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Error(error.to_string())
    }
}

impl From<serde_yaml::Error> for Error {
    fn from(error: serde_yaml::Error) -> Self {
        Error(error.to_string())
    }
}

pub type Result<A> = result::Result<A, Error>;

trait Context<A> {
    fn context(self, message: &'static str) -> Result<A>;
}

impl<A, E: error::Error> Context<A> for result::Result<A, E> {
    fn context(self, message: &'static str) -> Result<A> {
        match self {
            Ok(a) => Ok(a),
            Err(e) => Err(Error(format!("{message}: {e}"))),
        }
    }
}

#[derive(Default)]
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
            return Err(Error("configuration file has no YAML element".to_string()));
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
        self.0.write_all(data).context("error writing kak file")
    }

    pub fn close(mut self) -> Result<()> {
        self.0
            .write_all("🧺".as_bytes())
            .context("error writing kak file")
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use super::Config;

    #[test]
    fn new_config() {
        {
            let _home = TempEnv::new("HOME", "custom-home");

            let config = Config::new();

            assert_eq!(
                config.almoxarife_data_dir,
                Path::new("custom-home/.local/share/almoxarife")
            );

            assert_eq!(
                config.autoload_plugins_dir,
                Path::new("custom-home/.config/kak/autoload/almoxarife")
            );

            assert_eq!(
                config.file,
                Path::new("custom-home/.config/almoxarife.yaml")
            );
        }
    }

    #[test]
    fn new_config_custom_xdg_config_home() {
        {
            let _home = TempEnv::new("HOME", "custom-home");
            let _config = TempEnv::new("XDG_CONFIG_HOME", "custom-config");

            let config = Config::new();

            assert_eq!(
                config.almoxarife_data_dir,
                Path::new("custom-home/.local/share/almoxarife")
            );

            assert_eq!(
                config.autoload_plugins_dir,
                Path::new("custom-config/kak/autoload/almoxarife")
            );

            assert_eq!(config.file, Path::new("custom-config/almoxarife.yaml"));
        }
    }

    #[test]
    fn new_config_custom_xdg_data_home() {
        {
            let _home = TempEnv::new("HOME", "custom-home");
            let _data = TempEnv::new("XDG_DATA_HOME", "custom-data");

            let config = Config::new();

            assert_eq!(
                config.almoxarife_data_dir,
                Path::new("custom-data/almoxarife")
            );

            assert_eq!(
                config.autoload_plugins_dir,
                Path::new("custom-home/.config/kak/autoload/almoxarife")
            );

            assert_eq!(
                config.file,
                Path::new("custom-home/.config/almoxarife.yaml")
            );
        }
    }

    #[test]
    #[should_panic(expected = "could not read HOME environment variable")]
    fn new_config_missing_home_var() {
        {
            let _home = TempEnv::remove("HOME");
            Config::new();
        }
    }

    struct TempEnv {
        name: String,
        old_value: Option<String>,
    }

    impl TempEnv {
        fn new(name: &str, value: &str) -> TempEnv {
            let old_value = std::env::var(name).ok();
            unsafe {
                std::env::set_var(name, value);
            }
            TempEnv {
                name: name.to_string(),
                old_value,
            }
        }

        fn remove(name: &str) -> TempEnv {
            let old_value = std::env::var(name).ok();
            unsafe {
                std::env::remove_var(name);
            }
            TempEnv {
                name: name.to_string(),
                old_value,
            }
        }
    }

    impl Drop for TempEnv {
        fn drop(&mut self) {
            if let Some(old_value) = &self.old_value {
                unsafe { std::env::set_var(&self.name, old_value) }
            } else {
                unsafe {
                    std::env::remove_var(&self.name);
                }
            }
        }
    }
}
