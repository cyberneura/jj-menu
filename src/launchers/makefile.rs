//! Entries from the targets of a `Makefile`.
//!
//! The Makefile is scanned line by line rather than handed to `make -qp`,
//! because running make just to list targets can execute `$(shell ...)` in the
//! file. Reading is cheap and cannot have side effects.

use std::path::Path;

use super::{LauncherGroup, in_dir, quote};
use crate::config::MenuItem;

/// Read a Makefile and turn its targets into menu entries.
///
/// `start_dir` is the directory the menu was opened in; when the Makefile
/// lives in an ancestor, the command is prefixed with `cd` (unlike npm or
/// cargo, `make` does not search upwards).
pub fn scan(path: &Path, start_dir: &Path) -> Option<LauncherGroup> {
    let text = std::fs::read_to_string(path).ok()?;
    let dir = path.parent()?;

    let mut targets = Vec::new();
    for name in parse_targets(&text) {
        if !targets.contains(&name) {
            targets.push(name);
        }
    }
    if targets.is_empty() {
        return None;
    }

    let items = targets
        .into_iter()
        .map(|target| {
            // A target name can hold shell metacharacters, and this one came
            // from a file in the repository rather than from the menu author.
            let command = in_dir(dir, &format!("make {}", quote(&target)), start_dir);
            MenuItem::command(format!("make {target}"), command)
        })
        .collect();

    Some(LauncherGroup {
        source: "Makefile".to_string(),
        items,
    })
}

/// Target names that can be run without extra arguments.
///
/// Skipped on purpose:
///
/// - special targets (`.PHONY`, `.DEFAULT_GOAL`, ...) — not runnable
/// - pattern rules (`%.o: %.c`) — need a concrete file name
/// - names built from variables (`$(BIN): ...`) — the value is unknown here
/// - variable assignments (`CC := gcc`) and recipe lines (indented with a tab)
/// - the body of a `define`, which make does not read as makefile syntax
///   until the variable is expanded, so a `fake:` line in there is text
fn parse_targets(text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut in_define = false;

    for line in text.lines() {
        let start = line.trim_start();
        if in_define {
            in_define = !start.starts_with("endef");
            continue;
        }
        let start = start
            .strip_prefix("override ")
            .unwrap_or(start)
            .trim_start();
        if start.starts_with("define ") || start == "define" {
            in_define = true;
            continue;
        }
        if line.starts_with('\t') || start.starts_with('#') {
            continue;
        }
        let Some((left, right)) = line.split_once(':') else {
            continue;
        };
        // `::` is a double-colon rule; `:=` and friends are assignments.
        let right = right.strip_prefix(':').unwrap_or(right);
        if right.starts_with('=') {
            continue;
        }
        // A `=` before the colon means this is an assignment such as `A ?= b`.
        if left.contains('=') {
            continue;
        }

        for name in left.split_whitespace() {
            if name.starts_with('.') || name.contains('%') || name.contains('$') {
                continue;
            }
            targets.push(name.to_string());
        }
    }

    targets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_plain_targets() {
        let targets = parse_targets("build:\n\tcargo build\n\ntest: build\n\tcargo test\n");
        assert_eq!(targets, vec!["build", "test"]);
    }

    #[test]
    fn collects_every_name_of_a_multi_target_rule() {
        assert_eq!(parse_targets("a b c:\n\techo hi\n"), vec!["a", "b", "c"]);
    }

    #[test]
    fn skips_special_targets() {
        assert_eq!(
            parse_targets(".PHONY: build\nbuild:\n\ttrue\n"),
            vec!["build"]
        );
    }

    #[test]
    fn skips_the_body_of_a_define() {
        // make does not read a `define` body as makefile syntax until the
        // variable is expanded, so a colon in there is not a rule.
        let targets = parse_targets("define recipe\nfake:\n\techo hi\nendef\n\nbuild:\n\ttrue\n");
        assert_eq!(targets, vec!["build"]);
    }

    #[test]
    fn skips_the_body_of_an_assigning_or_overridden_define() {
        let targets = parse_targets("override define recipe :=\nfake:\nendef\nbuild:\n\ttrue\n");
        assert_eq!(targets, vec!["build"]);
    }

    #[test]
    fn skips_pattern_rules_because_they_need_a_file_name() {
        assert!(parse_targets("%.o: %.c\n\t$(CC) -c $<\n").is_empty());
    }

    #[test]
    fn skips_targets_built_from_variables() {
        assert!(parse_targets("$(BIN): main.o\n\ttrue\n").is_empty());
    }

    #[test]
    fn skips_variable_assignments() {
        assert!(parse_targets("CC := gcc\nFLAGS ?= -O2\nX = 1\n").is_empty());
    }

    #[test]
    fn skips_recipe_lines_that_contain_a_colon() {
        assert!(parse_targets("build:\n\techo a:b\n").contains(&"build".to_string()));
        assert_eq!(parse_targets("build:\n\techo a:b\n").len(), 1);
    }

    #[test]
    fn skips_comments() {
        assert!(parse_targets("# note: this is a comment\n").is_empty());
    }

    #[test]
    fn quotes_the_target_name_in_the_generated_command() {
        let dir = std::env::temp_dir().join("jj-menu-make-quote");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Makefile");
        std::fs::write(&path, "build:\n\ttrue\n").unwrap();

        let group = scan(&path, &dir).unwrap();
        assert_eq!(group.items[0].script().unwrap(), "make 'build'");
        assert_eq!(group.items[0].label(), "make build");
    }

    #[test]
    fn accepts_double_colon_rules() {
        assert_eq!(parse_targets("build::\n\ttrue\n"), vec!["build"]);
    }
}
