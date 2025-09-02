use std::collections::HashMap;
use std::env;
use std::error;
use std::ffi::OsStr;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::fs::File;
use std::io;
use std::io::Read;
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
pub struct Setup {
    /// The path to `almoxarife.yaml`.
    pub almoxarife_yaml_path: PathBuf,
    /// The directory where plugins' repos will be checked out (usually
    /// `~/.local/share/almoxarife`).
    pub almoxarife_data_dir: PathBuf,
    // The Almoxarife subdirectory inside `autoload`.
    pub autoload_plugins_dir: PathBuf,
    /// The path to `almoxarife.kak`
    almoxarife_kak: PathBuf,
    // The Kakoune's autoload directory.
    autoload_dir: PathBuf,
    // Custom environment variables tue setup process will consider.
    env: HashMap<&'static str, String>,
}

fn get_var(environment: &HashMap<&str, String>, var: &str) -> Option<String> {
    environment.get(var).cloned().or_else(|| env::var(var).ok())
}

impl Setup {
    pub fn new() -> Setup {
        Setup::with_env(HashMap::new())
    }

    pub fn with_env(env: HashMap<&'static str, String>) -> Setup {
        let home = get_var(&env, "HOME").expect("could not read HOME environment variable");

        let home = Path::new(&home);

        let config_dir = if let Some(config) = get_var(&env, "XDG_CONFIG_HOME") {
            PathBuf::from(&config)
        } else {
            home.join(".config")
        };

        let almoxarife_yaml_path = config_dir.join("almoxarife.yaml");

        let almoxarife_data_dir = if let Some(data) = get_var(&env, "XDG_DATA_HOME") {
            PathBuf::from(&data).join("almoxarife")
        } else {
            home.join(".local/share/almoxarife")
        };

        let autoload_dir = config_dir.join("kak/autoload");
        let mut autoload_plugins_dir = autoload_dir.clone();
        autoload_plugins_dir.push("almoxarife");
        let almoxarife_kak = autoload_plugins_dir.join("almoxarife.kak");

        Setup {
            almoxarife_yaml_path,
            almoxarife_kak,
            autoload_dir,
            autoload_plugins_dir,
            almoxarife_data_dir,
            env,
        }
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
            .envs(&self.env)
            .spawn()?;

        thread::sleep(Duration::from_millis(100));

        kakoune.kill()?;
        let output = kakoune.wait_with_output()?;
        let runtime_dir = OsStr::from_bytes(&output.stdout);
        let runtime_dir = PathBuf::from(runtime_dir).join("rc");
        unix::fs::symlink(runtime_dir, self.autoload_dir.join("rc"))?;

        Ok(())
    }

    pub fn create_kak_file_with_prelude(&self) -> Result<Kak<File>> {
        let mut kak = Kak::new(&self.almoxarife_kak)?;
        kak.write_prelude()?;
        Ok(kak)
    }

    pub fn open_config_file(&'_ self) -> Result<Config<'_, File>> {
        Config::new(self)
    }
}

pub struct Config<'setup, R> {
    file: R,
    setup: &'setup Setup,
}

impl<'setup> Config<'setup, File> {
    fn new(setup: &Setup) -> Result<Config<'_, File>> {
        let path = &setup.almoxarife_yaml_path;
        let file = File::open(path)?;
        Ok(Config { file, setup })
    }
}

impl<'setup, R: 'setup> Config<'setup, R>
where
    &'setup R: Read,
{
    pub fn parse_yaml(&'setup self) -> Result<Vec<Plugin>> {
        let tree: HashMap<String, PluginTree> = serde_yaml::from_reader(&self.file)?;

        if tree.is_empty() {
            return Err(Error("configuration file has no YAML element".to_string()));
        }

        let plugins = tree
            .into_iter()
            .flat_map(|(name, tree)| tree.plugins(name, self.setup))
            .collect();

        Ok(plugins)
    }
}

pub struct Kak<W: Write>(W);

impl Kak<File> {
    fn new(path: &Path) -> Result<Kak<File>> {
        let file = File::create(path).context("couldn't create almoxarife.kak file")?;
        Ok(Kak(file))
    }
}

impl<W: Write> Kak<W> {
    pub fn write_prelude(&mut self) -> Result<()> {
        let prelude = r"
        hook global KakBegin .* %🧺

        add-highlighter shared/almoxarife regions
        add-highlighter shared/almoxarife/ region '^\s*config:\s+\|' '^\s*\w+:' ref kakrc
        add-highlighter shared/almoxarife/ region '^\s*config:[^\n]' '\n' ref kakrc

        hook -group almoxarife global WinCreate .*almoxarife[.]yaml %{
            add-highlighter window/almoxarife ref almoxarife
            hook -once -always window WinClose .* %{ remove-highlighter window/almoxarife }
        }
        ";
        self.write(prelude.as_bytes())
    }

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
    use std::env;
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::Setup;

    #[test]
    fn new_setup() {
        let setup = Setup::with_env([("HOME", "custom-home".to_string())].into());

        assert_eq!(
            setup.almoxarife_data_dir,
            Path::new("custom-home/.local/share/almoxarife")
        );

        assert_eq!(
            setup.autoload_plugins_dir,
            Path::new("custom-home/.config/kak/autoload/almoxarife")
        );

        assert_eq!(
            setup.almoxarife_yaml_path,
            Path::new("custom-home/.config/almoxarife.yaml")
        );
    }

    #[test]
    fn new_setup_custom_xdg_config_home() {
        let setup = Setup::with_env(
            [
                ("HOME", "custom-home".to_string()),
                ("XDG_CONFIG_HOME", "custom-config".to_string()),
            ]
            .into(),
        );

        assert_eq!(
            setup.almoxarife_data_dir,
            Path::new("custom-home/.local/share/almoxarife")
        );

        assert_eq!(
            setup.autoload_plugins_dir,
            Path::new("custom-config/kak/autoload/almoxarife")
        );

        assert_eq!(
            setup.almoxarife_yaml_path,
            Path::new("custom-config/almoxarife.yaml")
        );
    }

    #[test]
    fn new_setup_custom_xdg_data_home() {
        let setup = Setup::with_env(
            [
                ("HOME", "custom-home".to_string()),
                ("XDG_DATA_HOME", "custom-data".to_string()),
            ]
            .into(),
        );

        assert_eq!(
            setup.almoxarife_data_dir,
            Path::new("custom-data/almoxarife")
        );

        assert_eq!(
            setup.autoload_plugins_dir,
            Path::new("custom-home/.config/kak/autoload/almoxarife")
        );

        assert_eq!(
            setup.almoxarife_yaml_path,
            Path::new("custom-home/.config/almoxarife.yaml")
        );
    }

    #[test]
    fn create_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let autoload_dir = temp_dir.path().join("autoload");
        let autoload_plugins_dir = autoload_dir.join("almoxarife");
        let almoxarife_data_dir = temp_dir.path().join("data");

        let mut executables_dir = project_path();
        executables_dir.push("tests");

        let path = std::env::var("PATH").unwrap();

        let setup = Setup {
            almoxarife_data_dir: almoxarife_data_dir.clone(),
            autoload_dir: autoload_dir.clone(),
            autoload_plugins_dir: autoload_plugins_dir.clone(),
            env: [(
                "PATH",
                format!("{}:{path}", executables_dir.to_string_lossy()),
            )]
            .into(),
            ..Default::default()
        };

        setup.create_dirs().unwrap();

        assert!(autoload_dir.is_dir());
        assert!(autoload_plugins_dir.is_dir());
        assert!(almoxarife_data_dir.is_dir());

        let mut runtime_dir = autoload_dir.clone();
        runtime_dir.push("rc");

        assert!(runtime_dir.is_symlink());
        assert!(runtime_dir.metadata().is_ok());
    }

    fn project_path() -> PathBuf {
        let path = env::current_dir().unwrap();
        let mut path_ancestors = path.as_path().ancestors();

        while let Some(p) = path_ancestors.next() {
            if fs::read_dir(p)
                .unwrap()
                .into_iter()
                .any(|p| p.unwrap().file_name() == "Cargo.toml")
            {
                return p.into();
            }
        }

        panic!("could not find project path");
    }
}
