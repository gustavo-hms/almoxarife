use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

use crate::setup::Kak;
use crate::setup::Plugin;
use crate::setup::PluginError;
use crate::setup::Setup;
use crate::setup::Status;

#[test]
fn new_setup() {
    let setup = Setup::with_env(
        [
            ("HOME", "custom-home".to_string()),
            ("XDG_DATA_HOME", "custom-data".to_string()),
            ("XDG_CONFIG_HOME", "custom-config".to_string()),
        ]
        .into(),
    );

    assert_eq!(
        setup.almoxarife_data_dir,
        Path::new("custom-data/almoxarife")
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
fn create_dirs() {
    let temp_dir = TempDir::new().unwrap();
    let autoload_dir = temp_dir.path().join("autoload");
    let autoload_plugins_dir = autoload_dir.join("almoxarife");
    let almoxarife_data_dir = temp_dir.path().join("data");

    let setup = Setup {
        almoxarife_data_dir: almoxarife_data_dir.clone(),
        autoload_dir: autoload_dir.clone(),
        autoload_plugins_dir: autoload_plugins_dir.clone(),
        env: add_tests_executables_to_path(),
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

#[test]
fn write_kak_file() {
    let mut kak = Kak::with_buffer();
    kak.write_prelude().unwrap();
    kak.write(b"require-module a-plugin\n").unwrap();
    kak.write(b"set global an-option 19\n").unwrap();
    kak.close().unwrap();
    let expected = r"hook global KakBegin .* %🧺
add-highlighter shared/almoxarife regions
add-highlighter shared/almoxarife/ region '^\s*config:\s+\|' '^\s*\w+:' ref kakrc
add-highlighter shared/almoxarife/ region '^\s*config:[^\n]' '\n' ref kakrc
hook -group almoxarife global WinCreate .*almoxarife[.]yaml %{
    add-highlighter window/almoxarife ref almoxarife
    hook -once -always window WinClose .* %{ remove-highlighter window/almoxarife }
}
require-module a-plugin
set global an-option 19
🧺";
    assert_eq!(kak.bytes(), expected.as_bytes());
}

#[test]
fn parse_yaml() {
    let file = b"
            luar:
                location: https://github.com/gustavo-hms/luar
                config: set-option global luar_interpreter luajit

                peneira:
                    location: /home/gustavo-hms/peneira
                    disabled: false

                    peneira-filters:
                      location: https://codeberg.org/mbauhardt/peneira-filters
                      config: |
                        map global normal <c-p> ': peneira-filters-mode<ret>'

            auto-pairs:
                location: https://github.com/alexherbo2/auto-pairs.kak
            ";

    let setup = Setup::default();
    let config = setup.config_from_buffer(file.as_slice()).unwrap();
    let plugins: HashMap<_, _> = config
        .active_plugins()
        .into_iter()
        .map(|p| (p.name.clone(), p))
        .collect();

    let expected: HashMap<_, _> = [
        (
            "auto-pairs".to_string(),
            Plugin {
                name: "auto-pairs".into(),
                parent: None,
                has_children: false,
                location: "https://github.com/alexherbo2/auto-pairs.kak".into(),
                is_local: false,
                config: Default::default(),
                repository_path: "~/.local/share/almoxarife/auto-pairs".into(),
                link_path: "~/.config/kak/autoload/almoxarife/auto-pairs".into(),
                env: Default::default(),
            },
        ),
        (
            "luar".to_string(),
            Plugin {
                name: "luar".into(),
                parent: None,
                has_children: true,
                location: "https://github.com/gustavo-hms/luar".into(),
                is_local: false,
                config: "set-option global luar_interpreter luajit".into(),
                repository_path: "~/.local/share/almoxarife/luar".into(),
                link_path: "~/.config/kak/autoload/almoxarife/luar".into(),
                env: Default::default(),
            },
        ),
        (
            "peneira".to_string(),
            Plugin {
                name: "peneira".into(),
                parent: Some("luar".into()),
                has_children: true,
                location: "/home/gustavo-hms/peneira".into(),
                is_local: true,
                config: Default::default(),
                repository_path: "/home/gustavo-hms/peneira".into(),
                link_path: "~/.config/kak/autoload/almoxarife/peneira".into(),
                env: Default::default(),
            },
        ),
        (
            "peneira-filters".to_string(),
            Plugin {
                name: "peneira-filters".into(),
                parent: Some("peneira".into()),
                has_children: false,
                location: "https://codeberg.org/mbauhardt/peneira-filters".into(),
                is_local: false,
                config: "map global normal <c-p> ': peneira-filters-mode<ret>'\n".into(),
                repository_path: "~/.local/share/almoxarife/peneira-filters".into(),
                link_path: "~/.config/kak/autoload/almoxarife/peneira-filters".into(),
                env: Default::default(),
            },
        ),
    ]
    .into();

    assert_eq!(plugins, expected);
}

#[test]
fn parse_yaml_disabled_plugin() {
    let file = b"
            luar:
                location: https://github.com/gustavo-hms/luar
                config: set-option global luar_interpreter luajit

                peneira:
                    location: /home/gustavo-hms/peneira
                    disabled: true

                    peneira-filters:
                      location: https://codeberg.org/mbauhardt/peneira-filters
                      config: |
                        map global normal <c-p> ': peneira-filters-mode<ret>'

            auto-pairs:
                location: https://github.com/alexherbo2/auto-pairs.kak
            ";

    let setup = Setup::default();
    let config = setup.config_from_buffer(file.as_slice()).unwrap();

    let disabled = config.disabled_plugins();
    assert_eq!(disabled, ["peneira", "peneira-filters"]);

    let plugins: HashMap<_, _> = config
        .active_plugins()
        .into_iter()
        .map(|p| (p.name.clone(), p))
        .collect();

    let expected: HashMap<_, _> = [
        (
            "auto-pairs".to_string(),
            Plugin {
                name: "auto-pairs".into(),
                parent: None,
                has_children: false,
                location: "https://github.com/alexherbo2/auto-pairs.kak".into(),
                is_local: false,
                config: Default::default(),
                repository_path: "~/.local/share/almoxarife/auto-pairs".into(),
                link_path: "~/.config/kak/autoload/almoxarife/auto-pairs".into(),
                env: Default::default(),
            },
        ),
        (
            "luar".to_string(),
            Plugin {
                name: "luar".into(),
                parent: None,
                has_children: true,
                location: "https://github.com/gustavo-hms/luar".into(),
                is_local: false,
                config: "set-option global luar_interpreter luajit".into(),
                repository_path: "~/.local/share/almoxarife/luar".into(),
                link_path: "~/.config/kak/autoload/almoxarife/luar".into(),
                env: Default::default(),
            },
        ),
    ]
    .into();

    assert_eq!(plugins, expected);
}

fn add_tests_executables_to_path() -> HashMap<&'static str, String> {
    let project_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let project_dir = Path::new(&project_dir);
    let path = std::env::var("PATH").unwrap();

    [(
        "PATH",
        format!("{}:{path}", project_dir.join("tests").to_string_lossy()),
    )]
    .into()
}

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
    env.insert("ALMOXARIFE_TEST_LOCATION", url.to_string() + ".git");
    env.insert(
        "ALMOXARIFE_TEST_REPO_PATH",
        repository_path.to_string_lossy().into(),
    );

    let plugin = Plugin {
        name: "kakoune-phantom-selection".into(),
        parent: None,
        has_children: false,
        location: url.to_string(),
        is_local: false,
        config: "map global normal f ': phantom-selection-add-selection<ret>'".into(),
        repository_path,
        link_path: link_path.clone(),
        env,
    };

    let status = plugin.update().unwrap();
    assert_eq!(
        status,
        Status::Installed {
            name: "kakoune-phantom-selection".into(),
            config: r"try %[ require-module kakoune-phantom-selection ]
map global normal f ': phantom-selection-add-selection<ret>'
"
            .into()
        }
    );

    assert!(link_path.is_symlink());
    assert!(link_path.metadata().is_ok());
}

#[test]
fn plugin_update_clone_plugin_with_parent() {
    let temp_dir = tempfile::tempdir().unwrap();
    // Almoxarife should see the dir `repo/kakoune-phantom-selection` does not
    // exist and clone it.
    let repository_path = temp_dir.path().join("repo/peneira");

    let link_dir = temp_dir.path().join("link");
    fs::create_dir(&link_dir).unwrap();
    let link_path = link_dir.join("peneira");

    let url = "https://github.com/gustavo-hms/peneira";

    let mut env = add_tests_executables_to_path();
    env.insert("ALMOXARIFE_TEST_LOCATION", url.to_string() + ".git");
    env.insert(
        "ALMOXARIFE_TEST_REPO_PATH",
        repository_path.to_string_lossy().into(),
    );

    let plugin = Plugin {
        name: "peneira".into(),
        parent: Some("luar".into()),
        has_children: false,
        location: url.to_string(),
        is_local: false,
        config: "set-option global peneira_files_command 'rg --files'".into(),
        repository_path,
        link_path: link_path.clone(),
        env,
    };

    let status = plugin.update().unwrap();
    assert_eq!(
        status,
        Status::Installed {
            name: "peneira".into(),
            config: r"hook -once global ModuleLoaded luar %[
    try %[ require-module peneira ]
    set-option global peneira_files_command 'rg --files'
]
"
            .into()
        }
    );

    assert!(link_path.is_symlink());
    assert!(link_path.metadata().is_ok());
}

#[test]
fn plugin_update_clone_plugin_with_children() {
    let temp_dir = tempfile::tempdir().unwrap();
    // Almoxarife should see the dir `repo/kakoune-phantom-selection` does not
    // exist and clone it.
    let repository_path = temp_dir.path().join("repo/peneira");

    let link_dir = temp_dir.path().join("link");
    fs::create_dir(&link_dir).unwrap();
    let link_path = link_dir.join("peneira");

    let url = "https://github.com/gustavo-hms/peneira";

    let mut env = add_tests_executables_to_path();
    env.insert("ALMOXARIFE_TEST_LOCATION", url.to_string() + ".git");
    env.insert(
        "ALMOXARIFE_TEST_REPO_PATH",
        repository_path.to_string_lossy().into(),
    );

    let plugin = Plugin {
        name: "peneira".into(),
        parent: None,
        has_children: true,
        location: url.to_string(),
        is_local: false,
        config: "set-option global peneira_files_command 'rg --files'".into(),
        repository_path,
        link_path: link_path.clone(),
        env,
    };

    let status = plugin.update().unwrap();
    assert_eq!(
        status,
        Status::Installed {
            name: "peneira".into(),
            config: r"try %[ require-module peneira ] catch %[
    provide-module peneira ''
    require-module peneira
]
set-option global peneira_files_command 'rg --files'
"
            .into()
        }
    );

    assert!(link_path.is_symlink());
    assert!(link_path.metadata().is_ok());
}

#[test]
fn plugin_update_clone_plugin_with_parent_and_children() {
    let temp_dir = tempfile::tempdir().unwrap();
    // Almoxarife should see the dir `repo/kakoune-phantom-selection` does not
    // exist and clone it.
    let repository_path = temp_dir.path().join("repo/peneira");

    let link_dir = temp_dir.path().join("link");
    fs::create_dir(&link_dir).unwrap();
    let link_path = link_dir.join("peneira");

    let url = "https://github.com/gustavo-hms/peneira";

    let mut env = add_tests_executables_to_path();
    env.insert("ALMOXARIFE_TEST_LOCATION", url.to_string() + ".git");
    env.insert(
        "ALMOXARIFE_TEST_REPO_PATH",
        repository_path.to_string_lossy().into(),
    );

    let plugin = Plugin {
        name: "peneira".into(),
        parent: Some("luar".into()),
        has_children: true,
        location: url.to_string(),
        is_local: false,
        config: "set-option global peneira_files_command 'rg --files'".into(),
        repository_path,
        link_path: link_path.clone(),
        env,
    };

    let status = plugin.update().unwrap();
    assert_eq!(
        status,
        Status::Installed {
            name: "peneira".into(),
            config: r"hook -once global ModuleLoaded luar %[
    try %[ require-module peneira ] catch %[
        provide-module peneira ''
        require-module peneira
    ]
    set-option global peneira_files_command 'rg --files'
]
"
            .into()
        }
    );

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
    env.insert("ALMOXARIFE_TEST_LOCATION", url.to_string() + ".git");
    env.insert(
        "ALMOXARIFE_TEST_REPO_PATH",
        repository_path.to_string_lossy().into(),
    );

    let plugin = Plugin {
        name: "kakoune-phantom-selection".into(),
        parent: None,
        has_children: false,
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
        PluginError::Clone(
            "kakoune-phantom-selection".into(),
            "git exited with status 1: unexpected error!".into()
        )
    );
}

#[test]
fn plugin_update_clone_link_error() {
    let temp_dir = tempfile::tempdir().unwrap();

    let repository_path = temp_dir.path().join("repo/kakoune-phantom-selection");

    // By not creating the subdirectory `link`, we should trigger a linking
    // error. If the error is not triggered, then we are not really executing
    // the linking phase.
    let link_dir = temp_dir.path().join("link");
    let link_path = link_dir.join("kakoune-phantom-selection");

    let url = "https://github.com/occivink/kakoune-phantom-selection";

    let mut env = add_tests_executables_to_path();
    env.insert("ALMOXARIFE_TEST_LOCATION", url.to_string() + ".git");
    env.insert(
        "ALMOXARIFE_TEST_REPO_PATH",
        repository_path.to_string_lossy().into(),
    );

    let plugin = Plugin {
        name: "kakoune-phantom-selection".into(),
        parent: None,
        has_children: false,
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
        PluginError::Link(
            "kakoune-phantom-selection".into(),
            format!(
                "No such file or directory (os error 2): {}",
                link_path.to_string_lossy()
            )
        )
    );
}

#[test]
fn plugin_update_pull_no_changes() {
    let temp_dir = tempfile::tempdir().unwrap();

    let repository_path = temp_dir.path().join("repo/kakoune-phantom-selection");
    // Almoxarife should see the dir `repo/kakoune-phantom-selection` already
    // exists and pull changes.
    fs::create_dir_all(&repository_path).unwrap();

    let link_dir = temp_dir.path().join("link");
    fs::create_dir(&link_dir).unwrap();
    let link_path = link_dir.join("kakoune-phantom-selection");

    let mut env = add_tests_executables_to_path();
    // Test we are calling `git pull` from the right directory.
    env.insert(
        "ALMOXARIFE_TEST_CWD",
        repository_path.to_string_lossy().into(),
    );

    let plugin = Plugin {
        name: "kakoune-phantom-selection".into(),
        parent: None,
        has_children: false,
        location: String::new(),
        is_local: false,
        config: "map global normal f ': phantom-selection-add-selection<ret>'".into(),
        repository_path: repository_path.into(),
        link_path: link_path.into(),
        env,
    };

    let status = plugin.update().unwrap();
    assert_eq!(
        status,
        Status::Unchanged {
            name: "kakoune-phantom-selection".into(),
            config: r"try %[ require-module kakoune-phantom-selection ]
map global normal f ': phantom-selection-add-selection<ret>'
"
            .into()
        }
    );
}

#[test]
fn plugin_update_pull_updates_available() {
    let temp_dir = tempfile::tempdir().unwrap();

    let repository_path = temp_dir.path().join("repo/kakoune-phantom-selection");
    // Almoxarife should see the dir `repo/kakoune-phantom-selection` already
    // exists and pull changes.
    fs::create_dir_all(&repository_path).unwrap();

    let link_dir = temp_dir.path().join("link");
    fs::create_dir(&link_dir).unwrap();
    let link_path = link_dir.join("kakoune-phantom-selection");

    let mut env = add_tests_executables_to_path();
    // Test we are calling `git pull` from the right directory.
    env.insert(
        "ALMOXARIFE_TEST_CWD",
        repository_path.to_string_lossy().into(),
    );
    env.insert("ALMOXARIFE_TEST_PLUGIN_UPDATE", "1".into());

    let plugin = Plugin {
        name: "kakoune-phantom-selection".into(),
        parent: None,
        has_children: false,
        location: String::new(),
        is_local: false,
        config: "map global normal f ': phantom-selection-add-selection<ret>'".into(),
        repository_path: repository_path.into(),
        link_path: link_path.into(),
        env,
    };

    let status = plugin.update().unwrap();
    assert_eq!(
        status,
        Status::Updated {
            name: "kakoune-phantom-selection".into(),
            config: r"try %[ require-module kakoune-phantom-selection ]
map global normal f ': phantom-selection-add-selection<ret>'
"
            .into(),
            log: "abcdef Some change\nghijk Other change\n".into()
        }
    );
}

#[test]
fn plugin_update_pull_unexpected_git_pull_fail() {
    let temp_dir = tempfile::tempdir().unwrap();

    let repository_path = temp_dir.path().join("repo/kakoune-phantom-selection");
    fs::create_dir_all(&repository_path).unwrap();

    let link_dir = temp_dir.path().join("link");
    fs::create_dir(&link_dir).unwrap();
    let link_path = link_dir.join("kakoune-phantom-selection");

    let mut env = add_tests_executables_to_path();
    env.insert("ALMOXARIFE_TEST_PULL_FAIL", "unexpected error!".to_string());
    env.insert(
        "ALMOXARIFE_TEST_CWD",
        repository_path.to_string_lossy().into(),
    );

    let plugin = Plugin {
        name: "kakoune-phantom-selection".into(),
        parent: None,
        has_children: false,
        location: String::new(),
        is_local: false,
        config: "map global normal f ': phantom-selection-add-selection<ret>'".into(),
        repository_path: repository_path.into(),
        link_path: link_path.into(),
        env,
    };

    let error = plugin.update().unwrap_err();
    assert_eq!(
        error,
        PluginError::Pull(
            "kakoune-phantom-selection".into(),
            "git exited with status 5: can't pull changes".into()
        )
    );
}

#[test]
fn plugin_update_pull_unexpected_git_rev_parse_fail() {
    let temp_dir = tempfile::tempdir().unwrap();

    let repository_path = temp_dir.path().join("repo/kakoune-phantom-selection");
    fs::create_dir_all(&repository_path).unwrap();

    let link_dir = temp_dir.path().join("link");
    fs::create_dir(&link_dir).unwrap();
    let link_path = link_dir.join("kakoune-phantom-selection");

    let mut env = add_tests_executables_to_path();
    env.insert("ALMOXARIFE_TEST_PLUGIN_UPDATE", "1".into());
    env.insert(
        "ALMOXARIFE_TEST_REV_PARSE_FAIL",
        "unexpected error!".to_string(),
    );
    env.insert(
        "ALMOXARIFE_TEST_CWD",
        repository_path.to_string_lossy().into(),
    );

    let plugin = Plugin {
        name: "kakoune-phantom-selection".into(),
        parent: None,
        has_children: false,
        location: String::new(),
        is_local: false,
        config: "map global normal f ': phantom-selection-add-selection<ret>'".into(),
        repository_path: repository_path.into(),
        link_path: link_path.into(),
        env,
    };

    let error = plugin.update().unwrap_err();
    assert_eq!(
        error,
        PluginError::Pull(
            "kakoune-phantom-selection".into(),
            "git exited with status 7: can't retrieve commit SHA".into()
        )
    );
}

#[test]
fn plugin_update_pull_unexpected_git_log_fail() {
    let temp_dir = tempfile::tempdir().unwrap();

    let repository_path = temp_dir.path().join("repo/kakoune-phantom-selection");
    fs::create_dir_all(&repository_path).unwrap();

    let link_dir = temp_dir.path().join("link");
    fs::create_dir(&link_dir).unwrap();
    let link_path = link_dir.join("kakoune-phantom-selection");

    let mut env = add_tests_executables_to_path();
    env.insert("ALMOXARIFE_TEST_PLUGIN_UPDATE", "1".into());
    env.insert("ALMOXARIFE_TEST_LOG_FAIL", "unexpected error!".to_string());
    env.insert(
        "ALMOXARIFE_TEST_CWD",
        repository_path.to_string_lossy().into(),
    );

    let plugin = Plugin {
        name: "kakoune-phantom-selection".into(),
        parent: None,
        has_children: false,
        location: String::new(),
        is_local: false,
        config: "map global normal f ': phantom-selection-add-selection<ret>'".into(),
        repository_path: repository_path.into(),
        link_path: link_path.into(),
        env,
    };

    let error = plugin.update().unwrap_err();
    assert_eq!(
        error,
        PluginError::Pull(
            "kakoune-phantom-selection".into(),
            "git exited with status 8: can't get log of changes".into()
        )
    );
}

#[test]
fn plugin_update_pull_link_error() {
    let temp_dir = tempfile::tempdir().unwrap();

    let repository_path = temp_dir.path().join("repo/kakoune-phantom-selection");
    fs::create_dir_all(&repository_path).unwrap();

    // By not creating the subdirectory `link`, we should trigger a linking
    // error. If the error is not triggered, then we are not really executing
    // the linking phase.
    let link_dir = temp_dir.path().join("link");
    let link_path = link_dir.join("kakoune-phantom-selection");

    let mut env = add_tests_executables_to_path();
    env.insert(
        "ALMOXARIFE_TEST_CWD",
        repository_path.to_string_lossy().into(),
    );

    let plugin = Plugin {
        name: "kakoune-phantom-selection".into(),
        parent: None,
        has_children: false,
        location: String::new(),
        is_local: false,
        config: "map global normal f ': phantom-selection-add-selection<ret>'".into(),
        repository_path: repository_path.into(),
        link_path: link_path.clone(),
        env,
    };

    let error = plugin.update().unwrap_err();
    assert_eq!(
        error,
        PluginError::Link(
            "kakoune-phantom-selection".into(),
            format!(
                "No such file or directory (os error 2): {}",
                link_path.to_string_lossy()
            )
        )
    );
}
