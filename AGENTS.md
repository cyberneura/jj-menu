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
| `src/ui/` | The interactive menu: `mod.rs` drawing, `state.rs` navigation, `prompt.rs` line editing, `theme.rs` colours |
| `src/exec.rs` | Running the selected script |
| `src/signal.rs` | Restoring the terminal when a signal kills the process |
| `src/shell_init.rs` | The `jj` wrapper function for bash / zsh / fish |
| `tests/` | Integration tests for configuration merging |

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
