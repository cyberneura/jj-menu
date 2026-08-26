# AGENTS.md

Notes for AI agents working on `jj-menu`. See `README.md` for what the tool is
and how a user configures it.

## What this is

A TUI menu launcher in Rust (edition 2024, MSRV 1.85). The user types `jj`,
picks an entry, and the selected shell script runs with the TTY fully attached.
Unix-like systems only.

Dependencies: `crossterm` (terminal), `clap` (CLI), `serde` +
`serde_yaml_ng` / `toml` / `serde_json` (configuration), `anyhow` (errors),
`dirs`, and `libc` — the last one only for restoring the terminal from a signal
handler, where nothing that locks may be called.

## Layout

| Path | Contents |
|---|---|
| `src/main.rs` | CLI, wiring, the `$ <script>` echo before a command runs |
| `src/config/` | Discovery of configuration files, loading, merging, the data model |
| `src/launchers/` | Entries derived from `package.json`, `Makefile`, Cargo and Gradle projects |
| `src/menu.rs` | Builds the menu from configuration plus launchers |
| `src/ui/` | The interactive menu: `mod.rs` drawing, `state.rs` navigation and the incremental search, `prompt.rs` line editing, `theme.rs` colours |
| `src/exec.rs` | Running the selected script |
| `src/parallel.rs` | Running a `parallel:` group: one shell per member, and passing Ctrl-C on to all of them |
| `src/signal.rs` | Restoring the terminal when a signal kills the process |
| `src/shell_init.rs` | The `jj` wrapper function for bash / zsh / fish |
| `tests/` | Integration tests for configuration merging |
| `skills/` | The agent skill published from this repository (`npx skills add cyberneura/jj-menu`) |

## Commands

Run these before pushing — they are what CI (`.github/workflows/ci.yml`)
checks, in this order, plus `cargo build --release`:

```shell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

For looking at the thing by hand, when you have a terminal:

```shell
cargo run                  # opens the menu in the current directory
cargo run -- --show-config # which configuration files were loaded, then exits
```

`cargo run` opens the interactive menu and waits for a key, so it is not part
of any automated check.

## Working on the terminal code

- **The menu is drawn on stderr**, so stdout stays free for `--print`. Keep it
  that way: the shell wrapper pipes stdout.
- **Everything the terminal sees goes through `ui::theme::paint_with`.** It
  takes the colour mode as an argument rather than reading the environment,
  because `theme::enabled()` is decided once per process — passing it in is what
  lets a test cover both modes in one run.
- **A frame is built in a `Vec<u8>` and written once.** `stderr` is unbuffered,
  so painting straight to it turns every colour change into its own write.
- **Text that comes from a file in the checkout is untrusted.** An npm script
  name or a make target can carry a real ESC; `ui::truncate` strips control
  characters so that merely opening the menu in a hostile repository cannot run
  escape sequences.
- **Colours may be off** (`NO_COLOR`, `TERM=dumb`). Anything that has to stand
  out then needs `Style::highlight()`, which falls back to reverse video.
- **The cursor is an index into what the search left, not into the entries.**
  `Frame` keeps the full `items` plus `visible`, the indices that match the
  query; `cursor` walks `visible`. Reading `items[cursor]` anywhere would pick
  the wrong entry as soon as a filter is on — go through `MenuState::selected`.
- **The search string lives on the frame, not on the menu.** That is what makes
  a submenu open unfiltered while going back restores the parent's filter. The
  `searching` flag in `run` is only about where typing goes.
- **The search row is drawn in place of the blank line under the title**, so
  turning the search on does not take a row off the list. `CHROME_ROWS` counts
  that blank line; if the row ever moves, the height arithmetic moves with it.
- **The selected entry's help is an overlay, not a row of its own.** It starts
  on the selected row after the label and is drawn *over* the entries below
  (`inline_help_runs`, `covered` in `ui::render`). Giving it rows would move
  every entry under the cursor on each keystroke, which is the bug it was
  written for (CYBERNEURA-DEV-582). The list height must not depend on it.
  What does not fit is dropped; `detail_help` wraps the same text over as much
  as half the screen in the detail view, which is where a long help is read.
- **`Ctrl-C` leaves from everywhere, including the search.** It is the one key
  that always stops what is going on; routing it to "cancel the search" would
  make the search the single place where it does not.
- **`Esc` is not simply quit.** The status row offers it as the way to drop a
  filter, so outside the search it clears one when there is one and only leaves
  the menu when there is not (`escape`). `q` and `Ctrl-C` always leave.
- **Letters are text while the search is open**, which is why `classify_search`
  maps movement to the arrows and to `Ctrl-N` / `Ctrl-P` only, and why the
  search string has no cursor of its own (that would need ← and →).

## Verifying a change

`cargo test` is the whole of it in a sandbox — a TUI cannot be driven without a
pty, and agents often cannot open one (`openpty: Operation not permitted`).
Rather than skipping verification, render a frame and assert on the bytes:
`ui::render(menu, cols, rows, color)` returns the frame as a `Vec<u8>` without
touching a terminal, and the tests in `src/ui/mod.rs` show the pattern.

When a test claims to guard behaviour, check that it actually fails when the
behaviour is removed. Several assertions here look convincing while passing on
mutated code.

Ask the user to run `cargo run` when the change is visual; they have a terminal.

## Working on the skill

`skills/jj-menu/SKILL.md` restates the configuration format for an agent that has
not read this repository. Nothing compiles it, so it rots silently: check any
claim against the implementation rather than against `README.md`, which itself
carries hedges (`dirs::config_dir()` is not `~/.config` on macOS) that are easy to
drop when condensing.

That it is still discoverable is checkable:

```shell
npx skills add . --list   # must report the skill with its name and description
```

## Releasing

Changing the version in `Cargo.toml` on `main` is what releases.
`.github/workflows/tag-on-version-change.yml` tags the merge commit when that
version has not been released -- it asks the releases API rather than reading
the diff, so a squash, a rebase and a direct push all behave the same -- and
`cargo-dist` (`dist-workspace.toml`, `.github/workflows/release.yml`) builds the
macOS and Linux binaries from that tag. Windows is deliberately not a target: a
menu entry is a shell script run through `$SHELL -c`.

Nothing is pushed to `cyberneura/homebrew-tap` from here. The tap updates its
own cask and formula files hourly from each project's latest release, which is
what keeps a token that can write to it out of this repository.
