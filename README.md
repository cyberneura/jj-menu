# jj-menu

A simple TUI menu launcher. Type `jj`, press Enter, pick a command with `j`/`k`,
run it with Enter.

This is a Rust rewrite of the Python [ytyng/jj-menu](https://github.com/ytyng/jj-menu).
The idea is the same — a per-project list of commands you can reach in two
keystrokes — with a YAML/TOML/JSON configuration, submenus, prompted arguments,
and entries picked up automatically from `package.json`, `Makefile`, Cargo and
Gradle projects.

The selected command runs with the TTY fully attached, so `ssh`, `vim`, `top`
and anything else interactive behaves exactly as if you had typed it.

**Unix-like systems only** (macOS, Linux). A menu entry is a shell script run
through `$SHELL -c`, which has no meaningful Windows equivalent, so no Windows
binaries are published.

## Install

### Homebrew

```shell
brew install cyberneura/tap/jj-menu
```

### Cargo

```shell
cargo install jj-menu
```

### Shell function

The binary is `jj-menu`. Add the wrapper function to get the short `jj` command,
and so that entries like `cd /tmp` or `export FOO=1` affect *your* shell rather
than a child process:

```shell
# ~/.zshrc
eval "$(jj-menu --shell-init zsh)"

# ~/.bashrc
eval "$(jj-menu --shell-init bash)"

# ~/.config/fish/config.fish
jj-menu --shell-init fish | source
```

The fish function needs **fish 3.4 or newer**. It relies on quoted command
substitution (`"$(...)"`), which is what keeps a multi-line entry in one piece;
older versions split it on newlines.

The wrapper also pushes the command into your shell history, so the usual
recall-and-edit workflow keeps working.

Without the wrapper, `jj-menu` runs the command itself; everything works except
changes to the shell's own state.

## Usage

```
jj-menu [OPTIONS]

  --print              Print the selected command instead of running it
  --shell-init <SHELL> Print the shell function (bash, zsh, fish)
  --cwd <DIR>          Search for configuration from here instead of $PWD
  --show-config        List the configuration files that were loaded
  -h, --help           Show help
  -V, --version        Show the version
```

### Keys

| Key | Action |
| --- | --- |
| `j`, `↓`, `Ctrl-N` | Down |
| `k`, `↑`, `Ctrl-P` | Up |
| `g`, `Home` | First entry |
| `G`, `End` | Last entry |
| `l`, `→` | Open the detail view (help, submenu) |
| `h`, `←` | Back (leaves the menu at the top level) |
| `Enter` | Run the entry |
| `q`, `Esc`, `Ctrl-C` | Quit |

Entries with a detail view are marked with `>`: those are the ones with a
`submenu`, a `help` text, or both. The view shows the help, the entry's own
command as a `Run: ...` line, and the submenu.

Arguments are not part of the detail view — they are prompted for when the entry
runs, whichever level you run it from.

While entering an argument: `Enter` accepts, `Esc` cancels, and the usual
`Ctrl-A` / `Ctrl-E` / `Ctrl-U` motions work.

### Exit codes

`jj-menu` exits with the exit code of the command it ran, so `jj && something`
behaves as expected. A command killed by a signal reports `128 + signal`, the
same as in bash and zsh. Dismissing the menu without choosing anything exits
`130` (the conventional "interrupted" code), which the shell wrapper treats as
"do nothing".

## Configuration

Configuration files are searched for in the current directory and every ancestor,
nearest first, and then in `~/.config/jj-menu/`. Every file that is found is
merged, in that order.

### File names

Any of these base names:

```
.jj-menu   _jj-menu   jj-menu
.jj-menu.local   _jj-menu.local   jj-menu.local
```

with any of these extensions:

```
.yaml   .yml   .toml   .json
```

The per-user file is `~/.config/jj-menu/config.{yaml,yml,toml,json}` (or the
platform equivalent of `$XDG_CONFIG_HOME`).

Within one directory the shared file is loaded before the `.local` one, so a
`.jj-menu.local.yaml` can add personal entries without touching the file that is
committed to the repository.

### Format

```yaml
menu:
  - title: Run the dev server
    shell: pnpm dev

  - title: Clean and rebuild
    # A list is joined into one script and runs in a single shell, so `cd`
    # carries over to the next line.
    shell:
      - cd build
      - make clean
      - make -j

  - title: Deploy
    help: Pick the environment to deploy to.
    submenu:
      - title: staging
        shell: ./deploy.sh staging
      - title: production
        shell: ./deploy.sh production

  - title: Search the repository
    shell: rg {pattern}
    args:
      - name: pattern
        prompt: Pattern
        default: TODO
```

The shortest possible file is a bare list, with no `menu:` key:

```yaml
- title: List files
  shell: ls -la
```

`title` may be omitted, in which case the command itself is the label.

The same structure works in TOML and JSON:

```toml
[[menu]]
title = "Run the dev server"
shell = "pnpm dev"
```

### Entry fields

| Field | Meaning |
| --- | --- |
| `title` | Label shown in the menu. Defaults to `shell`. |
| `shell` | Command, or a list of commands run as one script. |
| `help` | Description shown in the detail view. |
| `submenu` | Nested entries, opened with `l` / `→`. |
| `args` | Values prompted for and substituted into `shell`. |

Each `args` entry has a `name` (the `{name}` placeholder in `shell`), and
optionally a `prompt` and a `default`.

Values are substituted verbatim, so quoting is up to the template: write
`rg {pattern}` if you want the input to be able to carry flags, and
`rg "{pattern}"` if you want it treated as one literal word. A placeholder with
no matching argument is left alone, so `${HOME}` and `a{1,2}` survive unharmed.

### Not merging

A file marked `merge: false` is skipped when another configuration file has
already been loaded:

```yaml
merge: false
menu:
  - title: Only this
    shell: echo hi
```

Since files are loaded nearest-first, this turns a file into a **fallback**: it
contributes only when nothing closer to the working directory was found. That is
what you want on a repository-wide or per-user file whose entries would be noise
once a directory has its own menu.

It has no effect on the nearest file itself, which is always the one loaded
first.

## Automatic launchers

With no configuration at all, `jj-menu` still has something to show: it looks
for project files in the current directory and its ancestors.

| Source | Entries |
| --- | --- |
| `package.json` | Every `scripts` entry, run with the package manager implied by the nearest lock file — searched upwards, stopping at the repository root or your home directory (pnpm, yarn, bun, npm; npm if none is found) |
| `Makefile` | Every target that can run without extra arguments |
| `Cargo.toml` | `build`, `test`, `check`, and `run` — named per binary when there are several, omitted when there is none, and carrying `--features` for a target with `required-features` — plus `fmt` and `clippy` when those components are installed |
| Gradle | `tasks`, plus the lifecycle tasks (`build`, `clean`, `assemble`, `check`, and `test`) that the plugins declared by the build script define, using `./gradlew` when present — the wrapper is looked up separately from the build script, so a root wrapper is used for a subproject |

In a Node project with no jj-menu file, `jj` is therefore just a list of npm
scripts.

When several sources are found, each becomes a submenu so the top level stays
short. Configured entries always come first.

### What is deliberately left out

Only entries that can run as-is are listed, because a menu entry that always
fails is worse than a missing one:

- **Makefile**: pattern rules (`%.o: %.c`) and targets built from variables
  (`$(BIN):`) are skipped — they need a concrete file name that only you know.
  So is anything inside a `define ... endef`, which make does not read as
  makefile syntax until the variable is expanded, and anything inside a
  conditional (`ifeq` … `endif`), since which branch make takes depends on
  variables that cannot be expanded here. Recipe lines are recognised by the
  active recipe prefix, so a `.RECIPEPREFIX` other than tab is honoured, and a
  target that looks like a flag is run as `make -- '-n'`.
- **package.json**: a script whose name starts with `-` is skipped. Every one
  of the runners parses it as an option (`npm run --silent` exits without
  running anything) and none has a way to say "this is a script name".
- **Gradle**: the real task list can only be obtained by running
  `./gradlew tasks`, which starts a JVM and evaluates the build script. That is
  far too slow to do while opening a menu, so the tasks are inferred from the
  plugins the build script applies — for a subproject, the root's
  `allprojects` and `subprojects` blocks count as well, since that is where a
  multi-project build usually applies them. A plugin listed with `apply false`
  does not, and neither does one referenced through a version catalog
  (`alias(libs.plugins.foo)`), since the id behind the alias lives in
  `libs.versions.toml`. A build with nothing recognisable has no lifecycle
  tasks to offer — `build` there silently runs the unrelated
  `buildEnvironment` — so it gets only the `tasks` entry to discover the rest.
- **Cargo**: `cargo run` is omitted when the package has no binary at all (a
  virtual workspace, or a library-only package) and replaced by one
  `cargo run --bin ...` per target when there are several, because a bare
  `cargo run` would be ambiguous. Binaries auto-discovered from `src/main.rs`
  and `src/bin/` are counted, and `required-features` is passed through as
  `--features`. `fmt` and `clippy` are offered only once the component behind
  them answers, because rustup installs the shim for both whether or not the
  component is there.
- **Gradle**: commands are run from the directory holding the build script.
  Gradle takes the project directory from the working directory and rejects one
  that is not part of the build, so opening the menu in, say, `src/main/java`
  would otherwise fail.
- Tools that take positional arguments (`fab`, `cap`, ...) are not scanned at
  all. Define those explicitly with `args`, which is exactly what `args` is for.

Names taken from a project file (npm script names, make targets, Cargo binary
names) are quoted before being put in a command, and every string drawn on
screen has its control characters replaced with `·`. They come from the
repository rather than from whoever wrote the menu, so a name like
`build; rm -rf /` must not be able to add a second command, and one containing
an ESC must not be able to drive the terminal just by being displayed.

### Turning them off

```yaml
auto_launchers: false
```

or per launcher:

```yaml
auto_launchers:
  package_json: true
  makefile: false
  cargo: true
  gradle: false
```

The nearest configuration file decides; files further up only fill in a value
that has not been set yet.

## Unsupported formats

`.cson` (CoffeeScript Object Notation) configuration files are **not** read.
There is no maintained CSON parser for Rust, and writing one is out of
proportion to the format's use. A `.jj-menu.cson` is ignored, not reported as an
error. Use YAML, which CSON was modelled on.

## Development

```shell
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo run
```

Releases are built by [dist](https://opensource.axo.dev/cargo-dist/) from a
version tag, which also publishes the Homebrew formula to
`cyberneura/homebrew-tap`.

## License

MIT
