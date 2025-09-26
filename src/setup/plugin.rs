use colorized::Color;
use colorized::Colors;
use serde::Deserialize;
use std::collections::HashMap;
use std::error;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::iter;
use std::os::unix;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

use crate::setup::Setup;

#[derive(Debug, Deserialize)]
pub struct PluginTree {
    location: String,
    #[serde(default)]
    config: String,
    #[serde(default)]
    disabled: bool,
    #[serde(flatten)]
    children: HashMap<String, PluginTree>,
}

impl PluginTree {
    pub fn plugins(&self, name: String, setup: &Setup) -> Vec<Plugin> {
        if self.disabled {
            return Vec::new();
        }

        iter::once(Plugin::new(name, self, setup))
            .chain(
                self.children
                    .iter()
                    .flat_map(|(child_name, child)| child.plugins(child_name.clone(), setup)),
            )
            .collect()
    }
}

#[derive(Debug, PartialEq)]
pub struct Plugin {
    pub(super) name: String,
    /// Where the plugin is located (the URL of a git repo or a local folder).
    pub(super) location: String,
    /// Whether the code is located in a local folder.
    pub(super) is_local: bool,
    /// User defined configuration for the plugin.
    pub(super) config: String,
    /// The path to the folder containing the plugin's code.
    pub(super) repository_path: PathBuf,
    /// The path inside `autoload` where a soft link of the plugin is.
    pub(super) link_path: PathBuf,
    // Custom environment variables the plugin setup will consider.
    #[cfg(test)]
    pub(super) env: HashMap<&'static str, String>,
}

fn is_local(location: &str) -> bool {
    !location.starts_with("https://")
        && !location.starts_with("http://")
        && !location.starts_with("git@")
}

impl Plugin {
    fn new(name: String, node: &PluginTree, setup: &Setup) -> Plugin {
        let link_path = setup.autoload_plugins_dir.join(&name);

        let (is_local, repository_path) = if is_local(&node.location) {
            (true, PathBuf::from(&node.location))
        } else {
            (false, setup.almoxarife_data_dir.join(&name))
        };

        Plugin {
            name,
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

    pub fn update(self) -> Result<Status, Error> {
        let config = self.config();
        let name = self.name.clone();

        let status = match (self.is_local, self.repository_path_exists()) {
            (true, true) => Status::Local { name, config },

            (true, false) => {
                return Err(Error::Link(
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

    fn symlink(&self) -> Result<(), Error> {
        unix::fs::symlink(&self.repository_path, &self.link_path)
            .map_err(|e| Error::Link(self.name.clone(), e.to_string()))
    }

    fn clone_repo(&self, url: &str) -> Result<(), Error> {
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
            .map_err(|e| Error::Clone(self.name.clone(), e.to_string()))?;

        match output.status.code() {
            None | Some(0) => Ok(()),
            Some(code) => Err(Error::Clone(
                self.name.clone(),
                format!(
                    "git exited with status {}: {}",
                    code,
                    String::from_utf8_lossy(&output.stderr)
                ),
            )),
        }
    }

    fn pull(&self) -> Result<Option<String>, Error> {
        let old_revision = self.current_revision();

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
            .map_err(|e| Error::Pull(self.name.clone(), e.to_string()))?;

        if let Some(code) = output.status.code() {
            if code != 0 {
                return Err(Error::Pull(
                    self.name.clone(),
                    format!(
                        "git exited with status {}: {}",
                        code,
                        String::from_utf8_lossy(&output.stderr)
                    ),
                ));
            }
        }

        if let Some(old) = old_revision {
            if let Some(new) = self.current_revision() {
                return Ok(self.log(old, new));
            }
        }

        Ok(None)
    }

    pub fn config(&self) -> String {
        format!("try %[ require-module {} ]\n{}\n", self.name, self.config)
    }

    fn current_revision(&self) -> Option<String> {
        let mut command = Command::new("git");
        command
            .current_dir(&self.repository_path)
            .args(["rev-parse", "HEAD"]);

        #[cfg(test)]
        command.envs(&self.env);

        let output = command.output().ok()?;
        let mut revision = String::from_utf8_lossy(&output.stdout).to_string();
        revision.pop(); // Remove \n
        Some(revision)
    }

    fn log(&self, old_revision: String, new_revision: String) -> Option<String> {
        if old_revision == new_revision {
            return None;
        }

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

        let output = command.output().ok()?;
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

type Name = String;
type Message = String;

#[derive(Debug, PartialEq)]
pub enum Error {
    Clone(Name, Message),
    Pull(Name, Message),
    Link(Name, Message),
}

impl Error {
    pub fn plugin(&self) -> &str {
        match self {
            Error::Clone(name, _) => name,
            Error::Pull(name, _) => name,
            Error::Link(name, _) => name,
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Clone(name, message) => {
                write!(
                    f,
                    "{}: could not clone: {message}",
                    name.color(Colors::RedFg)
                )
            }

            Error::Pull(name, message) => {
                write!(
                    f,
                    "{}: could not update: {message}",
                    name.color(Colors::RedFg)
                )
            }

            Error::Link(name, message) => {
                write!(
                    f,
                    "{}: could not activate: {message}",
                    name.color(Colors::RedFg)
                )
            }
        }
    }
}

impl error::Error for Error {}

#[derive(Debug)]
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

#[cfg(test)]
mod test {
    use super::*;
    use crate::setup::test::add_tests_executables_to_path;

    #[test]
    fn plugin_update_clone() {
        let temp_dir = tempfile::tempdir().unwrap();
        // Almoxarife should see the dir `repo/kakoune-phantom-selection` does not
        // exist and clone it.
        let repository_path = temp_dir.path().join("repo/kakoune-phantom-selection");

        let link_dir = temp_dir.path().join("link");
        fs::create_dir(&link_dir).unwrap();
        let link_path = link_dir.join("kakoune-phantom-selection");

        let url = "https://github.com/occivink/kakoune-phantom-selection";

        let mut env = add_tests_executables_to_path();
        env.insert("ALMOXARIFE_TEST_SUBCOMMAND", "clone".to_string());
        env.insert("ALMOXARIFE_TEST_LOCATION", url.to_string() + ".git");
        env.insert(
            "ALMOXARIFE_TEST_REPO_PATH",
            repository_path.to_string_lossy().into(),
        );

        let plugin = Plugin {
            name: "kakoune-phantom-selection".into(),
            location: url.to_string(),
            is_local: false,
            config: "map global normal f ': phantom-selection-add-selection<ret>'".into(),
            repository_path,
            link_path: link_path.clone(),
            env,
        };

        plugin.update().unwrap();
        assert!(link_path.is_symlink());
        assert!(link_path.metadata().is_ok());
    }

    #[test]
    fn plugin_update_clone_unexpected_git_fail() {
        let temp_dir = tempfile::tempdir().unwrap();

        let repository_path = temp_dir.path().join("repo/kakoune-phantom-selection");

        let link_dir = temp_dir.path().join("link");
        fs::create_dir(&link_dir).unwrap();
        let link_path = link_dir.join("kakoune-phantom-selection");

        let url = "https://github.com/occivink/kakoune-phantom-selection";

        let mut env = add_tests_executables_to_path();
        env.insert("ALMOXARIFE_TEST_FAIL", "unexpected error!".to_string());
        env.insert("ALMOXARIFE_TEST_SUBCOMMAND", "clone".to_string());
        env.insert("ALMOXARIFE_TEST_LOCATION", url.to_string() + ".git");
        env.insert(
            "ALMOXARIFE_TEST_REPO_PATH",
            repository_path.to_string_lossy().into(),
        );

        let plugin = Plugin {
            name: "kakoune-phantom-selection".into(),
            location: url.to_string(),
            is_local: false,
            config: "map global normal f ': phantom-selection-add-selection<ret>'".into(),
            repository_path,
            link_path: link_path.clone(),
            env,
        };

        let error = plugin.update().unwrap_err();
        assert_eq!(
            error,
            Error::Clone(
                "kakoune-phantom-selection".into(),
                "git exited with status 1: unexpected error!".into()
            )
        );
    }

    #[test]
    fn plugin_update_pull() {
        let temp_dir = tempfile::tempdir().unwrap();

        let repository_path = temp_dir.path().join("repo/kakoune-phantom-selection");
        // Almoxarife should see the dir `repo/kakoune-phantom-selection` already
        // exists and pull changes.
        fs::create_dir_all(&repository_path).unwrap();

        let link_dir = temp_dir.path().join("link");
        fs::create_dir(&link_dir).unwrap();
        let link_path = link_dir.join("kakoune-phantom-selection");

        let mut env = add_tests_executables_to_path();
        env.insert("ALMOXARIFE_TEST_SUBCOMMAND", "pull".to_string());
        env.insert(
            "ALMOXARIFE_TEST_CWD",
            repository_path.to_string_lossy().into(),
        );

        let plugin = Plugin {
            name: "kakoune-phantom-selection".into(),
            location: String::new(),
            is_local: false,
            config: "map global normal f ': phantom-selection-add-selection<ret>'".into(),
            repository_path: repository_path.into(),
            link_path: link_path.into(),
            env,
        };

        plugin.update().unwrap();
    }
}
