//! Entries for a Cargo project.
//!
//! Cargo finds the manifest itself, so the commands need no `cd` prefix.
//! Only subcommands that ship with a normal Rust toolchain are offered; a
//! `cargo clippy` entry that fails because the component is missing would be
//! worse than no entry at all, so it is included only when the manifest is a
//! real package (workspace roots without a package still get the basics).

use std::path::Path;

use super::{LauncherGroup, quote};
use crate::config::MenuItem;

/// Subcommands that work for any manifest, package or virtual workspace.
const COMMANDS: &[&str] = &["build", "test", "check", "fmt", "clippy"];

/// Produce the standard Cargo entries, plus one `cargo run --bin` per extra
/// binary target declared in the manifest.
pub fn scan(path: &Path) -> Option<LauncherGroup> {
    let text = std::fs::read_to_string(path).ok()?;
    let manifest: toml::Value = toml::from_str(&text).ok()?;

    let mut items: Vec<MenuItem> = COMMANDS
        .iter()
        .map(|c| MenuItem::command(format!("cargo {c}"), format!("cargo {c}")))
        .collect();

    // `cargo run` is only offered when it can actually resolve a target:
    //
    // - a virtual workspace (no `[package]`) has nothing to run
    // - several `[[bin]]` targets make a bare `cargo run` ambiguous, so name
    //   each one instead
    let bins = binary_names(&manifest);
    if manifest.get("package").is_some() {
        if bins.len() > 1 {
            for bin in bins {
                // The name comes from a manifest in the repository, so quote
                // it for the same reason npm script names are quoted.
                items.push(MenuItem::command(
                    format!("cargo run --bin {bin}"),
                    format!("cargo run --bin {}", quote(&bin)),
                ));
            }
        } else {
            items.push(MenuItem::command("cargo run", "cargo run"));
        }
    }

    Some(LauncherGroup {
        source: "Cargo.toml".to_string(),
        items,
    })
}

/// Names of the `[[bin]]` targets declared in the manifest.
fn binary_names(manifest: &toml::Value) -> Vec<String> {
    manifest
        .get("bin")
        .and_then(|b| b.as_array())
        .map(|bins| {
            bins.iter()
                .filter_map(|b| b.get("name")?.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_manifest(name: &str, body: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("jj-menu-cargo-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Cargo.toml");
        fs::write(&path, body).unwrap();
        path
    }

    fn labels(path: &std::path::Path) -> Vec<String> {
        scan(path)
            .unwrap()
            .items
            .iter()
            .map(|i| i.label())
            .collect()
    }

    #[test]
    fn offers_the_standard_subcommands() {
        let path = write_manifest("basic", "[package]\nname = \"x\"\nversion = \"0.1.0\"\n");
        let labels = labels(&path);
        assert!(labels.contains(&"cargo build".to_string()));
        assert!(labels.contains(&"cargo test".to_string()));
        assert!(labels.contains(&"cargo run".to_string()));
    }

    #[test]
    fn omits_run_for_a_virtual_workspace_where_it_cannot_resolve_a_target() {
        let path = write_manifest("virtual", "[workspace]\nmembers = [\"a\"]\n");
        assert!(!labels(&path).iter().any(|l| l.starts_with("cargo run")));
    }

    #[test]
    fn names_binaries_explicitly_only_when_there_are_several() {
        let one = write_manifest(
            "one-bin",
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[[bin]]\nname = \"a\"\npath = \"a.rs\"\n",
        );
        let one_labels = labels(&one);
        assert!(!one_labels.iter().any(|l| l.contains("--bin")));
        assert!(one_labels.contains(&"cargo run".to_string()));

        let two = write_manifest(
            "two-bins",
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[[bin]]\nname = \"a\"\npath = \"a.rs\"\n\n[[bin]]\nname = \"b\"\npath = \"b.rs\"\n",
        );
        let two_labels = labels(&two);
        assert!(two_labels.contains(&"cargo run --bin a".to_string()));
        assert!(two_labels.contains(&"cargo run --bin b".to_string()));
        assert!(
            !two_labels.contains(&"cargo run".to_string()),
            "a bare `cargo run` is ambiguous with several binaries"
        );
    }

    #[test]
    fn quotes_a_binary_name_that_carries_shell_metacharacters() {
        let path = write_manifest(
            "bin-injection",
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n             [[bin]]\nname = \"a; touch /tmp/pwned\"\npath = \"a.rs\"\n\n             [[bin]]\nname = \"b\"\npath = \"b.rs\"\n",
        );
        let scripts: Vec<String> = scan(&path)
            .unwrap()
            .items
            .iter()
            .filter_map(|i| i.script())
            .collect();
        assert!(
            scripts.contains(&"cargo run --bin 'a; touch /tmp/pwned'".to_string()),
            "{scripts:?}"
        );
    }

    #[test]
    fn ignores_a_broken_manifest_instead_of_failing() {
        let path = write_manifest("broken", "this is not toml =");
        assert!(scan(&path).is_none());
    }
}
