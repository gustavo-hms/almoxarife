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
use std::iter;
use std::os::unix;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::result;
use std::thread;
use std::time::Duration;

use colorized::Color;
use colorized::Colors;
use serde::Deserialize;

pub struct Setup {
    /// The path to `almoxarife.yaml`.
    pub almoxarife_yaml_path: PathBuf,
    /// The directory where plugins' repos will be checked out (usually
    /// `~/.local/share/almoxarife`).
    pub almoxarife_data_dir: PathBuf,
    // The Almoxarife subdirectory inside `autoload`.
    pub autoload_plugins_dir: PathBuf,
    /// The path to `almoxarife.kak`
    pub almoxarife_kak: PathBuf,
    // The Kakoune's autoload directory.
    pub autoload_dir: PathBuf,
    // Custom environment variables the setup process will consider.
    #[cfg(test)]
    pub env: HashMap<&'static str, String>,
}

impl Default for Setup {
    fn default() -> Self {
        Setup {
            almoxarife_yaml_path: "~/.config/almoxarife.yaml".into(),
            almoxarife_data_dir: "~/.local/share/almoxarife".into(),
            autoload_plugins_dir: "~/.config/kak/autoload/almoxarife".into(),
            almoxarife_kak: "~/.config/kak/autoload/almoxarife/almoxarife.kak".into(),
            autoload_dir: "~/.config/kak/autoload".into(),
            #[cfg(test)]
            env: HashMap::default(),
        }
    }
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
            #[cfg(test)]
            env,
        }
    }

    pub fn create_dirs(&self) -> Result<(), SetupError> {
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

    fn link_runtime_dir(&self) -> Result<(), SetupError> {
        let mut command = Command::new("kak");
        command
            .args(["-d", "-s", "almoxarife", "-E"])
            .arg("echo -to-file /dev/stdout %val[runtime]")
            .stdout(Stdio::piped());

        #[cfg(test)]
        command.envs(&self.env);

        let mut kakoune = command.spawn()?;
        thread::sleep(Duration::from_millis(100));
        kakoune.kill()?;
        let output = kakoune.wait_with_output()?;
        let runtime_dir = OsStr::from_bytes(&output.stdout);
        let runtime_dir = PathBuf::from(runtime_dir).join("rc");
        unix::fs::symlink(runtime_dir, self.autoload_dir.join("rc"))?;

        Ok(())
    }

    pub fn create_kak_file_with_prelude(&self) -> Result<Kak<File>, SetupError> {
        let mut kak = Kak::new(&self.almoxarife_kak)?;
        kak.write_prelude()?;
        Ok(kak)
    }

    pub fn open_config_file(&self) -> Result<Config<'_>, SetupError> {
        Config::new(self)
    }

    #[cfg(test)]
    pub fn config_from_buffer(&self, buffer: &[u8]) -> Result<Config<'_>, SetupError> {
        Config::from_reader(buffer, self)
    }
}

pub struct Config<'setup> {
    setup: &'setup Setup,
    plugins: HashMap<String, PluginTree>,
}

impl<'setup> Config<'setup> {
    fn new(setup: &Setup) -> Result<Config<'_>, SetupError> {
        let file = File::open(&setup.almoxarife_yaml_path)?;
        Config::from_reader(&file, setup)
    }

    fn from_reader<'r, R: 'r + ?Sized>(
        reader: &'r R,
        setup: &'setup Setup,
    ) -> Result<Config<'setup>, SetupError>
    where
        &'r R: Read,
    {
        let plugins: HashMap<String, PluginTree> =
            serde_yaml::from_reader(reader).context(&format!(
                "couldn't parse {}",
                setup.almoxarife_yaml_path.to_string_lossy()
            ))?;

        if plugins.is_empty() {
            return Err(SetupError(
                "configuration file has no YAML element".to_string(),
            ));
        }

        Ok(Config { setup, plugins })
    }

    pub fn list_plugins(&self) -> Vec<(&str, PluginStatus)> {
        self.plugins
            .iter()
            .flat_map(|(name, tree)| {
                iter::once((
                    name.as_str(),
                    if tree.disabled {
                        PluginStatus::Disabled
                    } else {
                        PluginStatus::Enabled
                    },
                ))
                .chain(tree.list_children())
            })
            .collect()
    }

    pub fn active_plugins(self) -> Vec<Plugin> {
        self.plugins
            .into_iter()
            .flat_map(|(name, tree)| tree.plugins(name, None, &self.setup))
            .collect()
    }
}

pub enum PluginStatus {
    Enabled,
    Disabled,
}

#[derive(Debug, Deserialize)]
struct PluginTree {
    location: String,
    #[serde(default)]
    config: String,
    #[serde(default)]
    disabled: bool,
    #[serde(flatten)]
    children: HashMap<String, PluginTree>,
}

impl PluginTree {
    fn plugins(&self, name: String, parent: Option<String>, setup: &Setup) -> Vec<Plugin> {
        if self.disabled {
            return Vec::new();
        }

        iter::once(Plugin::new(name.clone(), self, parent, setup))
            .chain(self.children.iter().flat_map(move |(child_name, child)| {
                child.plugins(child_name.clone(), Some(name.clone()), setup)
            }))
            .collect()
    }

    fn list_children(&self) -> Vec<(&str, PluginStatus)> {
        self.children
            .iter()
            .flat_map(|(name, subtree)| {
                iter::once((
                    name.as_str(),
                    if subtree.disabled {
                        PluginStatus::Disabled
                    } else {
                        PluginStatus::Enabled
                    },
                ))
                .chain(subtree.list_children())
            })
            .collect()
    }
}

#[derive(Debug, PartialEq)]
pub struct Plugin {
    pub name: String,
    /// The parent of this plugin, if any.
    pub parent: Option<String>,
    /// Whether this plugin has children.
    pub has_children: bool,
    /// Where the plugin is located (the URL of a git repo or a local folder).
    pub location: String,
    /// Whether the code is located in a local folder.
    pub is_local: bool,
    /// User defined configuration for the plugin.
    pub config: String,
    /// The path to the folder containing the plugin's code.
    pub repository_path: PathBuf,
    /// The path inside `autoload` where a soft link of the plugin is.
    pub link_path: PathBuf,
    // Custom environment variables the plugin setup will consider.
    #[cfg(test)]
    pub env: HashMap<&'static str, String>,
}

fn is_local(location: &str) -> bool {
    !location.starts_with("https://")
        && !location.starts_with("http://")
        && !location.starts_with("git@")
}

impl Plugin {
    fn new(name: String, node: &PluginTree, parent: Option<String>, setup: &Setup) -> Plugin {
        let link_path = setup.autoload_plugins_dir.join(&name);

        let (is_local, repository_path) = if is_local(&node.location) {
            (true, PathBuf::from(&node.location))
        } else {
            (false, setup.almoxarife_data_dir.join(&name))
        };

        Plugin {
            name,
            parent,
            has_children: !node.children.is_empty(),
            config: node.config.clone(),
            location: node.location.clone(),
            is_local,
            repository_path,
            link_path,
            #[cfg(test)]
            env: setup.env.clone(),
        }
    }

    fn repository_path_exists(&self) -> bool {
        fs::metadata(&self.repository_path).is_ok()
    }

    pub fn update(self) -> Result<Status, PluginError> {
        let config = self.config();
        let name = self.name.clone();

        let status = match (self.is_local, self.repository_path_exists()) {
            (true, true) => Status::Local { name, config },

            (true, false) => {
                return Err(PluginError::Link(
                    name,
                    format!("the path {} is empty", self.location),
                ))
            }

            (false, true) => match self.pull()? {
                None => Status::Unchanged { name, config },
                Some(log) => Status::Updated { name, log, config },
            },

            (false, false) => {
                self.clone_repo(&self.location)?;
                Status::Installed { name, config }
            }
        };

        self.symlink()?;
        Ok(status)
    }

    fn symlink(&self) -> Result<(), PluginError> {
        unix::fs::symlink(&self.repository_path, &self.link_path).map_err(|e| {
            PluginError::Link(
                self.name.clone(),
                format!("{}: {}", e, self.link_path.to_string_lossy()),
            )
        })
    }

    fn clone_repo(&self, url: &str) -> Result<(), PluginError> {
        let location = format!("{url}.git");

        let mut command = Command::new("git");
        command
            .arg("clone")
            .arg(location)
            .arg(&self.repository_path)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        #[cfg(test)]
        command.envs(&self.env);

        let output = command
            .output()
            .map_err(|e| PluginError::Clone(self.name.clone(), e.to_string()))?;

        match output.status.code() {
            None | Some(0) => Ok(()),
            Some(code) => Err(PluginError::Clone(
                self.name.clone(),
                format!(
                    "git exited with status {}: {}",
                    code,
                    String::from_utf8_lossy(&output.stderr)
                ),
            )),
        }
    }

    fn pull(&self) -> Result<Option<String>, PluginError> {
        let old_revision = self.current_revision()?;

        let mut command = Command::new("git");
        command
            .arg("pull")
            .current_dir(&self.repository_path)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        #[cfg(test)]
        command.envs(&self.env);

        let output = command
            .output()
            .map_err(|e| PluginError::Pull(self.name.clone(), e.to_string()))?;

        if let Some(code) = output.status.code() {
            if code != 0 {
                return Err(PluginError::Pull(
                    self.name.clone(),
                    format!(
                        "git exited with status {}: {}",
                        code,
                        String::from_utf8_lossy(&output.stderr)
                    ),
                ));
            }
        }

        let new_revision = self.current_revision()?;

        if old_revision == new_revision {
            return Ok(None);
        }

        self.log(old_revision, new_revision).map(|log| Some(log))
    }

    pub fn config(&self) -> String {
        match (&self.parent, self.has_children) {
            (None, false) => {
                format!(
                    "try %[ require-module {plugin} ]
{config}
",
                    plugin = self.name,
                    config = self.config
                )
            }

            (None, true) => format!(
                "try %[ require-module {plugin} ] catch %[
    provide-module {plugin} ''
    require-module {plugin}
]
{config}
",
                plugin = self.name,
                config = self.config
            ),

            (Some(parent), false) => format!(
                "hook -once global ModuleLoaded {parent} %[
    try %[ require-module {plugin} ]
    {config}
]
",
                plugin = self.name,
                parent = parent,
                config = self.config
            ),

            (Some(parent), true) => format!(
                "hook -once global ModuleLoaded {parent} %[
    try %[ require-module {plugin} ] catch %[
        provide-module {plugin} ''
        require-module {plugin}
    ]
    {config}
]
",
                plugin = self.name,
                parent = parent,
                config = self.config
            ),
        }
    }

    fn current_revision(&self) -> Result<String, PluginError> {
        let mut command = Command::new("git");
        command
            .current_dir(&self.repository_path)
            .args(["rev-parse", "HEAD"]);

        #[cfg(test)]
        command.envs(&self.env);

        let output = command
            .output()
            .map_err(|e| PluginError::Pull(self.name.clone(), e.to_string()))?;

        if let Some(code) = output.status.code() {
            if code != 0 {
                return Err(PluginError::Pull(
                    self.name.clone(),
                    format!(
                        "git exited with status {}: {}",
                        code,
                        String::from_utf8_lossy(&output.stderr)
                    ),
                ));
            }
        }

        let mut revision = String::from_utf8_lossy(&output.stdout).to_string();
        revision.pop(); // Remove \n
        Ok(revision)
    }

    fn log(&self, old_revision: String, new_revision: String) -> Result<String, PluginError> {
        let range = format!("{old_revision}..{new_revision}");

        let mut command = Command::new("git");
        command.current_dir(&self.repository_path).args([
            "log",
            &range,
            "--oneline",
            "--no-decorate",
            "--reverse",
        ]);

        #[cfg(test)]
        command.envs(&self.env);

        let output = command
            .output()
            .map_err(|e| PluginError::Pull(self.name.clone(), e.to_string()))?;

        if let Some(code) = output.status.code() {
            if code != 0 {
                return Err(PluginError::Pull(
                    self.name.clone(),
                    format!(
                        "git exited with status {}: {}",
                        code,
                        String::from_utf8_lossy(&output.stderr)
                    ),
                ));
            }
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

#[derive(Debug, PartialEq)]
pub enum Status {
    Installed {
        name: String,
        config: String,
    },
    Updated {
        name: String,
        log: String,
        config: String,
    },
    Unchanged {
        name: String,
        config: String,
    },
    Local {
        name: String,
        config: String,
    },
}

pub struct Kak<W: Write>(W);

impl Kak<File> {
    fn new(path: &Path) -> Result<Kak<File>, SetupError> {
        let file = File::create(path).context("couldn't create almoxarife.kak file")?;
        Ok(Kak(file))
    }
}

#[cfg(test)]
impl Kak<Vec<u8>> {
    pub fn with_buffer() -> Self {
        Kak(Vec::new())
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

impl<W: Write> Kak<W> {
    pub fn write_prelude(&mut self) -> Result<(), SetupError> {
        let prelude = r"hook global KakBegin .* %🧺
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

    pub fn write(&mut self, data: &[u8]) -> Result<(), SetupError> {
        self.0.write_all(data).context("error writing kak file")
    }

    pub fn close(&mut self) -> Result<(), SetupError> {
        self.0
            .write_all("🧺".as_bytes())
            .context("error writing kak file")
    }
}

#[derive(Debug, PartialEq)]
pub struct SetupError(String);

impl Display for SetupError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl error::Error for SetupError {}

impl From<io::Error> for SetupError {
    fn from(error: io::Error) -> Self {
        SetupError(error.to_string())
    }
}

impl From<serde_yaml::Error> for SetupError {
    fn from(error: serde_yaml::Error) -> Self {
        SetupError(error.to_string())
    }
}

trait Context<A> {
    fn context(self, message: &str) -> Result<A, SetupError>;
}

impl<A, E: error::Error> Context<A> for result::Result<A, E> {
    fn context(self, message: &str) -> Result<A, SetupError> {
        match self {
            Ok(a) => Ok(a),
            Err(e) => Err(SetupError(format!("{message}: {e}"))),
        }
    }
}

type Name = String;
type Message = String;

#[derive(Debug, PartialEq)]
pub enum PluginError {
    Clone(Name, Message),
    Pull(Name, Message),
    Link(Name, Message),
}

impl PluginError {
    pub fn plugin(&self) -> &str {
        match self {
            PluginError::Clone(name, _) => name,
            PluginError::Pull(name, _) => name,
            PluginError::Link(name, _) => name,
        }
    }
}

impl Display for PluginError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginError::Clone(name, message) => {
                write!(
                    f,
                    "{}: could not clone: {message}",
                    name.color(Colors::RedFg)
                )
            }

            PluginError::Pull(name, message) => {
                write!(
                    f,
                    "{}: could not update: {message}",
                    name.color(Colors::RedFg)
                )
            }

            PluginError::Link(name, message) => {
                write!(
                    f,
                    "{}: could not activate: {message}",
                    name.color(Colors::RedFg)
                )
            }
        }
    }
}
