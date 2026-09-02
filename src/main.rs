//! `jj-menu` — a simple TUI menu launcher.
//!
//! See the README for the configuration format.

mod config;
mod exec;
mod launchers;
mod menu;
mod parallel;
mod shell_init;
mod signal;
mod ui;

use std::io::{IsTerminal, Write, stderr, stdout};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;

use config::Launch;
use shell_init::ShellKind;

/// Exit code used when the menu is dismissed without choosing anything.
///
/// 130 is the conventional "terminated by Ctrl-C" code, which keeps the shell
/// wrapper from evaluating an empty command.
const EXIT_CANCELLED: u8 = 130;

#[derive(Parser, Debug)]
#[command(
    name = "jj-menu",
    version,
    about = "A simple TUI menu launcher",
    long_about = None
)]
struct Args {
    /// Print the selected command to stdout instead of running it.
    ///
    /// Used by the shell wrapper so that `cd` and `export` affect the calling
    /// shell. See `jj-menu --shell-init <shell>`.
    #[arg(long)]
    print: bool,

    /// Print the shell function to add to your startup file.
    #[arg(long, value_name = "SHELL")]
    shell_init: Option<ShellKind>,

    /// Start the search for configuration files here instead of the working
    /// directory.
    #[arg(long, value_name = "DIR")]
    cwd: Option<PathBuf>,

    /// List the configuration files that were loaded, then exit.
    #[arg(long)]
    show_config: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            let mut out = stderr();
            // Unlike the menu, this can end up in a file or a pipe, so the
            // colour depends on where it is going.
            if out.is_terminal() {
                let _ = ui::theme::paint(
                    &mut out,
                    ui::theme::Style::fg(ui::theme::ERROR).bold(),
                    "jj-menu:",
                );
                let _ = writeln!(out, " {err:#}");
            } else {
                let _ = writeln!(out, "jj-menu: {err:#}");
            }
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let args = Args::parse();

    if let Some(kind) = args.shell_init {
        print!("{}", shell_init::snippet(kind));
        return Ok(ExitCode::SUCCESS);
    }

    let start_dir = match args.cwd {
        Some(dir) => dir,
        None => std::env::current_dir().context("failed to read the working directory")?,
    };
    // Absolute from here on. The launchers build `cd <dir> && ...` commands
    // for projects found in an ancestor, and those run with the working
    // directory already set to `start_dir` — a relative path would then be
    // resolved a second time and point somewhere else entirely.
    let start_dir = std::fs::canonicalize(&start_dir)
        .with_context(|| format!("failed to resolve {}", start_dir.display()))?;

    let config = config::load(&start_dir)?;

    if args.show_config {
        report_config(&config, &start_dir);
        return Ok(ExitCode::SUCCESS);
    }

    let items = menu::build(&config, &start_dir);
    if items.is_empty() {
        eprintln!("{}", no_entries_help());
        return Ok(ExitCode::FAILURE);
    }

    // The menu reads keys from stdin and draws on stderr, so both have to be
    // a terminal. Checking stdin alone would let `jj 2>menu.log` block on a
    // menu nobody can see, with the escape sequences going into the log.
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("no terminal available (stdin is not a TTY)");
    }
    if !stderr().is_terminal() {
        anyhow::bail!("no terminal available (stderr is not a TTY; the menu is drawn there)");
    }

    let title = format!("jj-menu — {}", start_dir.display());
    match ui::run(items, &title)? {
        ui::Outcome::Cancelled => Ok(ExitCode::from(EXIT_CANCELLED)),
        ui::Outcome::Run(Launch::Script(script), cwd) => {
            let cwd = cwd.as_deref().unwrap_or(&start_dir);
            if args.print {
                let mut out = stdout();
                writeln!(out, "{}", in_dir_script(cwd, &script, &start_dir))?;
                out.flush()?;
                return Ok(ExitCode::SUCCESS);
            }

            // Echoed with the `cd` the child is given as its working
            // directory, so what is on screen is what is being run. An entry
            // from an ancestor's file otherwise looks like it runs here.
            echo("$", &in_dir_script(cwd, &script, &start_dir))?;
            let status = exec::run(&script, cwd)?;
            // Pass the command's exit code through, so `jj && next` and `$?`
            // behave the way they would for a typed command.
            Ok(ExitCode::from(exec::exit_code(status)))
        }
        ui::Outcome::Run(Launch::Parallel(jobs), cwd) => {
            let cwd = cwd.as_deref().unwrap_or(&start_dir);
            // Run here even under `--print`. There is nothing to hand back to
            // the calling shell: the point of `--print` is that `cd` and
            // `export` reach the user's shell, and a group is several separate
            // processes, none of which could change it anyway. Printing them
            // as one script would mean writing the `&`-and-`wait` form of
            // whichever shell is calling — and fish, which the wrapper
            // supports, does not have it.
            for job in &jobs {
                echo("&", &in_dir_script(cwd, &job.script, &start_dir))?;
            }
            // With `--print` the wrapper is reading stdout through a command
            // substitution and evaluates whatever comes back; a job writing
            // there would be *run*, not shown. Send it to the terminal instead.
            let output = if args.print {
                parallel::Output::Stderr
            } else {
                parallel::Output::Inherit
            };
            Ok(ExitCode::from(parallel::run(&jobs, cwd, output)?))
        }
    }
}

/// Prefix a whole script with the `cd` that puts it in `dir`.
///
/// `--print` hands the command to the calling shell, which is the one process
/// whose working directory `exec::run` cannot set, so the directory has to be
/// part of the command itself. That `cd` stays in effect afterwards: a
/// directory change reaching your own shell is what the wrapper is for, and an
/// entry that would rather not move you says `run_in_current_directory`.
///
/// **`;` rather than `&&`.** `&&` binds tighter than `&`, so
/// `cd dir && server &` would put the `cd` in the background along with the
/// command and leave the rest of the script running where the shell already
/// was. There is nothing lost by dropping the guard: `dir` is where a
/// configuration file was read from moments earlier.
///
/// A subshell would be the tidier tool -- it would leave the calling shell
/// where it was -- but there is no form of one that bash, zsh *and* fish all
/// accept, and it would swallow the `cd` and `export` effects that are the
/// whole point of `--print`.
fn in_dir_script(dir: &std::path::Path, script: &str, start_dir: &std::path::Path) -> String {
    if dir == start_dir {
        return script.to_string();
    }
    format!("cd {}; {script}", launchers::quote(&dir.to_string_lossy()))
}

/// Echo a command before it runs, the way a shell shows what it is doing.
///
/// `sigil` is `$` for a single command and `&` for one member of a parallel
/// group. stderr is known to be a terminal here: the menu was just drawn on it.
fn echo(sigil: &str, script: &str) -> Result<()> {
    let mut out = stderr();
    ui::theme::paint(
        &mut out,
        ui::theme::Style::fg(ui::theme::COMMAND).bold(),
        sigil,
    )?;
    writeln!(out, " {script}")?;
    out.flush()?;
    Ok(())
}

fn report_config(config: &config::Config, start_dir: &std::path::Path) {
    if config.sources.is_empty() {
        println!(
            "no configuration file found (searched from {})",
            start_dir.display()
        );
    } else {
        println!("configuration files, in load order:");
        for source in &config.sources {
            // "by default", because this is the file's setting: an entry
            // carrying `run_in_current_directory` of its own overrides it and
            // does not show up here, which a bare "runs in ..." would deny.
            match &source.cwd {
                Some(dir) => println!(
                    "  {} (entries run in {} by default)",
                    source.path.display(),
                    dir.display()
                ),
                None => println!(
                    "  {} (entries run in the working directory by default)",
                    source.path.display()
                ),
            }
        }
    }
    println!("menu entries from configuration: {}", config.menu.len());
    println!(
        "built-in launchers enabled: {}",
        config.auto_launchers.any()
    );
}

fn no_entries_help() -> String {
    "\
jj-menu: nothing to show.

Create a configuration file in this directory or an ancestor, for example
.jj-menu.yaml:

  menu:
    - title: List files
      shell: ls -la
    - title: Git log
      shell: git log --oneline --graph --decorate --all
    - title: Dev servers
      # Each entry gets its own shell and they all run at once; Ctrl-C
      # stops the lot.
      parallel:
        - shell: npm run dev
        - shell: npm run api

    - title: Deploy
      help: Pick the environment to deploy to.
      submenu:
        - title: staging
          shell: ./deploy.sh staging
        - title: production
          shell: ./deploy.sh production
    - title: Search
      shell: rg {pattern}
      args:
        - name: pattern
          prompt: Pattern
          default: TODO

Entries are also picked up automatically from package.json, Makefile,
Cargo.toml and Gradle builds.

To make `cd` inside a menu entry affect your shell, add the wrapper function:

  # ~/.zshrc
  eval \"$(jj-menu --shell-init zsh)\"
"
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_help_text_shows_a_usable_example() {
        let help = no_entries_help();
        assert!(help.contains("menu:"));
        assert!(help.contains("shell:"));
        assert!(help.contains("parallel:"));
        assert!(help.contains("--shell-init"));
    }

    #[test]
    fn the_example_in_the_help_text_parses() {
        // Keep the sample honest: extract the YAML block and load it.
        let help = no_entries_help();
        let start = help.find("  menu:").expect("the sample starts with menu:");
        let end = help
            .find("\nEntries are")
            .expect("the sample ends before the prose");
        let sample: String = help[start..end]
            .lines()
            .map(|line| line.strip_prefix("  ").unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n");

        let parsed = config::loader::parse(std::path::Path::new("sample.yaml"), &sample)
            .expect("the sample in the help text must parse");
        assert_eq!(parsed.menu.len(), 5);
    }

    #[test]
    fn a_script_is_printed_unchanged_when_it_already_runs_here() {
        let here = std::path::Path::new("/tmp/project");
        assert_eq!(in_dir_script(here, "make build", here), "make build");
    }

    #[test]
    fn a_script_from_another_directory_is_printed_with_a_cd() {
        assert_eq!(
            in_dir_script(
                std::path::Path::new("/tmp/pro'ject"),
                "make build",
                std::path::Path::new("/tmp/pro'ject/sub"),
            ),
            r"cd '/tmp/pro'\''ject'; make build",
            "the directory is quoted for the shell that will evaluate this"
        );
    }

    #[test]
    fn the_cd_covers_the_whole_script_not_just_its_first_command() {
        // `&&` would bind tighter than the `&`, backgrounding the `cd` along
        // with the server and leaving the next line where the shell already
        // was. Every command of the script has to end up in the directory.
        let printed = in_dir_script(
            std::path::Path::new("/tmp/project"),
            "server &\nnext",
            std::path::Path::new("/tmp/project/sub"),
        );
        assert_eq!(printed, "cd '/tmp/project'; server &\nnext");
        assert!(!printed.contains("&&"), "{printed}");
    }

    #[test]
    fn cancelling_uses_the_conventional_interrupt_code() {
        assert_eq!(EXIT_CANCELLED, 130);
    }
}
