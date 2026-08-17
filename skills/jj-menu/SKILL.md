---
name: jj-menu
description: Write and maintain jj-menu configuration files (.jj-menu.yaml / .toml / .json) — the per-project TUI menu reached by typing `jj`. Use when adding, editing or debugging menu entries, submenus, prompted arguments, merge behaviour or automatic launchers, or when the user mentions jj-menu, `.jj-menu.yaml`, or asks to put a command "in the jj menu".
---

# jj-menu configuration

`jj-menu` is a TUI launcher: the user types `jj`, picks an entry, and the entry's
shell script runs with the TTY attached. Entries come from configuration files
plus automatic launchers. Unix only.

Full documentation: https://github.com/cyberneura/jj-menu

## Where the configuration lives

Files are searched in the current directory and every ancestor, nearest first,
then the per-user `config.{yaml,yml,toml,json}` under the platform configuration
directory — `~/.config/jj-menu/` on Linux, **`~/Library/Application Support/jj-menu/`
on macOS**. Every file found is merged, in that order, unless one of them opts out
with `merge: false` (below).

Base names: `.jj-menu`, `_jj-menu`, `jj-menu`, and the `.local` variants
(`.jj-menu.local`, …). Extensions: `.yaml`, `.yml`, `.toml`, `.json`.
`.cson` is ignored, not an error.

Within one directory the shared file loads before the `.local` one, so a
`.jj-menu.local.yaml` holds personal entries and stays out of the repository.

**Adding an entry: edit the file that already exists** — find it with
`jj-menu --show-config`, which lists exactly the files that were loaded. Only
create a new file when there is none. For a repository, `.jj-menu.yaml` at the
root is the normal choice; put anything personal in `.jj-menu.local.yaml` and
check that it is gitignored.

## Format

```yaml
menu:
  - title: Run the dev server
    shell: pnpm dev

  - title: Clean and rebuild
    # A list is joined into one script and runs in a single shell,
    # so `cd` carries over to the next line.
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

A bare list with no `menu:` key is also valid. The same structure works in TOML
(`[[menu]]`) and JSON.

| Field | Meaning |
| --- | --- |
| `title` | Label. Defaults to `shell`. |
| `shell` | Command, or a list of commands run as one script. |
| `help` | Description shown in the detail view (`l` / `→`). |
| `submenu` | Nested entries. |
| `args` | Values prompted for and substituted into `shell`. |

### args

Each entry has `name` (the `{name}` placeholder in `shell`), optionally `prompt`
and `default`. Arguments are prompted when the entry runs, not in the detail
view.

Substitution is verbatim — the value is pasted into the script before the shell
parses it, and jj-menu never escapes it. **Quoting is the template's job**:
`rg {pattern}` lets the input carry flags and its own quoting, `rg "{pattern}"`
keeps whitespace in a single argument. Double quotes do **not** make it literal —
`$(...)`, backticks and `$VAR` still expand inside them, and a `"` in the input
ends the quoted section. Treat the value as code the user typed, not as data.

A placeholder with no matching argument is left alone, so `${HOME}` and `a{1,2}`
survive.

### merge: false

```yaml
merge: false
menu:
  - title: Only this
    shell: echo hi
```

A file marked this way is skipped when another file has already been loaded.
Since loading is nearest-first, it makes the file a **fallback**: it contributes
only when nothing closer to the working directory exists. Use it on a
repository-wide or per-user file whose entries would be noise inside a directory
that has its own menu. It never affects the nearest file, which is always loaded
first.

## Automatic launchers

Entries are also derived from `package.json` scripts (run with the package
manager implied by the nearest lock file), `Makefile` targets that need no extra
arguments, Cargo (`build`/`test`/`check`/`run`, plus `fmt` and `clippy` when
installed) and Gradle. **This happens whether or not a configuration file
exists** — writing one does not turn the launchers off. Configured entries come
first, then one group per source; a lone group is flattened into the top level
when nothing is configured.

Only entries that can run as-is are listed. Make pattern rules, targets built
from variables, and anything inside `define` or a conditional are skipped; so
are npm scripts whose name starts with `-`. Tools taking positional arguments
(`fab`, `cap`, …) are not scanned at all — **define those explicitly with
`args`**.

Turning them off:

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

The nearest configuration file decides; files further up only fill in values not
already set.

## Checking a change

**Do not run bare `jj-menu` yourself.** It needs a terminal on both stdin and
stderr and refuses without one — `jj-menu: no terminal available (stdin is not a
TTY)`, exit 1. That message means your shell has no TTY, not that the
configuration is broken. Given a TTY it blocks until a key is pressed. Use:

```shell
jj-menu --show-config   # lists the configuration files that were loaded, then exits
```

A file that fails to parse is reported here, including one with an unknown key:
every configuration struct rejects fields it does not know, so a typo is an error
rather than a silently ignored line. Do not invent fields. To see the menu, ask
the user to run `jj`.

`--print` prints the selected command instead of running it, but still opens the
menu, so it is for the user, not for you.

## Installing jj-menu

Only if it is missing (`command -v jj-menu`):

```shell
brew install cyberneura/tap/jj-menu    # or: cargo install jj-menu
```

The short `jj` command is a shell function — without it, `cd` or `export` in an
entry cannot affect the user's shell:

```shell
eval "$(jj-menu --shell-init bash)"    # ~/.bashrc
eval "$(jj-menu --shell-init zsh)"     # ~/.zshrc
jj-menu --shell-init fish | source     # ~/.config/fish/config.fish, fish 3.4+
```

The three snippets differ, so pass the shell that is actually being configured —
the zsh one pushes to history with `print -s`, which bash does not have.
