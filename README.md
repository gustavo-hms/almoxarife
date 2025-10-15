# Almoxarife

Almoxarife is a fast plugin manager for the [Kakoune editor](https://kakoune.org/). Different
from other plugin managers (like [kak-bundle](https://codeberg.org/jdugan6240/kak-bundle)),
it's not itself a Kakoune's plugin but an external executable. You describe your
plugins in a configuration file and Almoxarife installs and updates them in
parallel.

## Features

Almoxarife has some interesting features listed bellow.

### Parallel installs and updates

Each plugin is managed in its own thread: installs, updates, and deletes are all
handled in parallel.

### Minimal runtime overhead

Almoxarife is not a plugin itself and doesn't load your plugins using `kak` scripts.
Instead it relies on Kakoune's builtin `autoload` functionality to load them.

It only generates a minimal `kak` file to include any configuration scripts you
write on its `almoxarife.yaml` file, and to ensure dependencies are loaded in the
right order and asynchronously.

This design ensures Kakoune loads as fast as it gets.

### Changelogs

Almoxarife lists the changelog of every updated plugin.

### Error handling

Every error it encounters while installing or updating the plugins is shown in a
comprehensive list, much like the changelog list.

### Dependency handling

You can specify whether a plugin depends on another one, building a dependency
tree of plugins. This has two advantages:

- If you disable a parent plugin, all its dependants are also disabled, recursively;
- Almoxarife takes care of only loading children plugins after the parent is already
  loaded, avoiding the situation at which a child plugin `requires` a parent but the
  parent is not yet loaded.

### Local plugins

Almoxarife can handle scripts present in local directories. This way, you can
mantain a clean `kakrc` by putting more complex scripts elsewhere.

### Automatic cleanup

When you remove a plugin from your configuration file, Almoxarife automatically
deletes the cloned repo (unless the removed plugin was a local directory, in which
case no removal takes place).

### Zero-friction initial setup

Kakoune has an odd behaviour regarding its `autoload` functionality: if it detects an
`autoload` subdirectory inside your `kak` directory (usually `~/.config/kak/autoload`),
*it stops loading its standard scripts*. That means that, whenever you start
manually installing plugins or writing a custom script, you *have* to make a symlink
to Kakoune's standard scripts into `~/.config/kak/autoload`, otherwise previously
working functionality suddenly stops working. That's very confusing, specially for
new users.

Fortunatelly, Almoxarife automatically handles that for you. So start using
Almoxarife is as simple as running `al --config`, no matter what your previous
setup is.

### Syntax highlighting of the configuration file

Even though the configuration file is an yaml file, you can put kakscript code on
it, and this code is properly highlighted.

## Usage

For a first setup, run
```
al --config
```
It will open the configuration file in an instance of Kakoune. Edit the file at
will. When you leave Kakoune, Almoxarife will install all the plugins described
in the just edited config file.

To update previously installed plugins, just run
```
al
```

Finally, every time you want to edit your configuration, run `al --config` again
and Almoxarife will take care of the details.

### Configuration format

The configuration file consists of a yaml document in the following simple format:
```yaml
# The key is the name of the plugin. If the plugin defines a module, this should
# be the module name, because Almoxarife will `require` the module automatically.
plugin-name:
  # May be a repository URL or a path for a local directory. It's the only required
  # field.
  location: https://github.com/user/plugin
  # Kakscript code to configure your plugin (optional).
  config: set buffer my-plugin-option true
  # Whether this plugin should be disabled (optional; defaults to false).
  disabled: true
```

Example:

```yaml
kakoune-gdb:
  location: https://github.com/occivink/kakoune-gdb

search:
  location: https://github.com/1g0rb0hm/search.kak
  config: set-option global search_context 3 # number of context lines
  disabled: true

state-save:
  location: https://gitlab.com/Screwtapello/kakoune-state-save
  config: |
    state-save-reg-load colon
    state-save-reg-load pipe
    state-save-reg-load slash

    hook global KakEnd .* %{
        state-save-reg-save colon
        state-save-reg-save pipe
        state-save-reg-save slash
    }

custom-scripts:
  location: ~/code/my-kak-scripts
```

#### Dependencies

You can specify dependencies between plugins by making a plugin configuration a child of another one:

```yaml
luar:
  location: https://github.com/gustavo-hms/luar
  config: set-option global luar_interpreter luajit

  peneira:
    location: https://github.com/gustavo-hms/peneira

    peneira-filters:
      location: https://codeberg.org/mbauhardt/peneira-filters
      config: |
        map global normal <c-p> ': peneira-filters-mode<ret>'

  enluarada:
    location: https://github.com/gustavo-hms/enluarada
    config: |
      require-module enluarada-search-tools
      require-module enluarada-selections

  objetiva:
    location: https://github.com/gustavo-hms/objetiva
    config: |
      map global object x '<a-;>objetiva-line<ret>' -docstring line
      map global object m '<a-;>objetiva-matching<ret>' -docstring matching
      map global object h '<a-;>objetiva-case<ret>' -docstring case
      map global normal h ': objetiva-case-move<ret>'
      map global normal H ': objetiva-case-expand<ret>'
      map global normal <a-h> ': objetiva-case-move-previous<ret>'
      map global normal <a-H> ': objetiva-case-expand-previous<ret>'
```

## Installation

Almoxarife consists of a single statically-linked binary called `al`. So, you can
just download it from the releases page and put it in your PATH.

If you prefer, you can install it via cargo: `cargo install almoxarife`.

### Comparison to kak-bundle

kak-bundle is a very well written and featurefull polugin manager. It uses a very
different approach though. Here I try to summarize the main differences.

While kak-bundle is implemented as a Kakoune plugin, Almoxarife is an external
statically linked binary, what means you don't have to change anything inside your
`~/.config/kak` directory.

Both Almoxarife and kak-bundle imposes very low runtime overhead, offering comparable
Kakoune's loading times.

Almoxarife has built-in support for plugin dependencies, ensuring they are loaded
in the right order.

Almoxarife focuses on offering a good usabillity, with the following properties:
- installing, updating, and cleaning up is all a single two-letters command: `al`;
- opening the configuration file is just a matter of running `al --config` (or
  just `al -c`);
- no other command is necessary or even provided;
- the command output is minimal, visually pleasant, and informative, telling the
  plugin status, showing changelogs and listing eventual errors.

That comes at a price, though: less common workflows present on kak-bundle are not
implemented:

- It doesn't support running installation scripts, like building binaries after
  cloning or updating a repository.
- It doesn't support selectively loading a subset of the scripts in a plugin
  repository, like kak-bundle does.

If you rely on any of this functionality, I recommend you give kak-bundle a try.

## Name

Almoxarife is a portuguese word for a warehouseman.
