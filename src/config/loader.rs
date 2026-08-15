//! Reading configuration files and merging them into one menu.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::discovery;
use super::model::{AutoLaunchers, ConfigFile, MenuItem};

/// The merged configuration used to build the menu.
#[derive(Debug, Default)]
pub struct Config {
    pub menu: Vec<MenuItem>,
    pub auto_launchers: AutoLaunchers,
    /// Files that were actually merged, in load order. Reported by `--debug`.
    pub sources: Vec<PathBuf>,
}

/// Parse one configuration file, dispatching on the extension.
///
/// A bare top-level list is accepted as a shorthand for `menu:`, so the
/// shortest possible config is just a list of entries.
pub fn parse(path: &Path, text: &str) -> Result<ConfigFile> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match ext.as_str() {
        "yaml" | "yml" => parse_yaml(text),
        "toml" => parse_toml(text),
        "json" => parse_json(text),
        other => anyhow::bail!("unsupported configuration format: .{other}"),
    }
}

fn parse_yaml(text: &str) -> Result<ConfigFile> {
    match serde_yaml_ng::from_str::<ConfigFile>(text) {
        Ok(config) => Ok(config),
        Err(err) => match serde_yaml_ng::from_str::<Vec<MenuItem>>(text) {
            Ok(menu) => Ok(ConfigFile {
                menu,
                ..Default::default()
            }),
            // Report the error from the documented shape, not from the
            // shorthand: it is much more likely to point at the real mistake.
            Err(_) => Err(err.into()),
        },
    }
}

fn parse_toml(text: &str) -> Result<ConfigFile> {
    // TOML has no top-level array, so only the documented shape applies.
    Ok(toml::from_str::<ConfigFile>(text)?)
}

fn parse_json(text: &str) -> Result<ConfigFile> {
    match serde_json::from_str::<ConfigFile>(text) {
        Ok(config) => Ok(config),
        Err(err) => match serde_json::from_str::<Vec<MenuItem>>(text) {
            Ok(menu) => Ok(ConfigFile {
                menu,
                ..Default::default()
            }),
            Err(_) => Err(err.into()),
        },
    }
}

/// Load and merge every configuration file that applies to `start_dir`.
///
/// A file whose `merge` is `false` is skipped once anything has been loaded.
/// Because files arrive nearest-first, that makes such a file a fallback: it
/// contributes only when nothing closer to `start_dir` was found.
pub fn load(start_dir: &Path) -> Result<Config> {
    let mut config = Config::default();
    let mut auto_launchers: Option<AutoLaunchers> = None;

    for path in discovery::all_config_paths(start_dir) {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let file =
            parse(&path, &text).with_context(|| format!("failed to parse {}", path.display()))?;

        if !file.merge && !config.sources.is_empty() {
            continue;
        }

        // The nearest file that actually sets `auto_launchers` wins; files
        // further up only fill in a value nobody has decided yet.
        if auto_launchers.is_none() {
            auto_launchers = file.auto_launchers;
        }

        config.menu.extend(file.menu);
        config.sources.push(path);
    }

    config.auto_launchers = auto_launchers.unwrap_or_default();
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(name: &str, text: &str) -> Result<ConfigFile> {
        parse(Path::new(name), text)
    }

    #[test]
    fn parses_the_documented_yaml_shape() {
        let config =
            parse_str("a.yaml", "menu:\n  - title: hello\n    shell: echo hello\n").unwrap();
        assert_eq!(config.menu.len(), 1);
        assert_eq!(config.menu[0].label(), "hello");
        assert_eq!(config.menu[0].script().unwrap(), "echo hello");
        assert!(config.merge);
    }

    #[test]
    fn parses_a_bare_top_level_list_as_the_menu() {
        let config = parse_str("a.yaml", "- title: hello\n  shell: echo hello\n").unwrap();
        assert_eq!(config.menu.len(), 1);
        assert_eq!(config.menu[0].label(), "hello");
    }

    #[test]
    fn joins_a_shell_list_into_one_script() {
        let config = parse_str(
            "a.yaml",
            "menu:\n  - title: t\n    shell:\n      - cd /tmp\n      - ls\n",
        )
        .unwrap();
        assert_eq!(config.menu[0].script().unwrap(), "cd /tmp\nls");
    }

    #[test]
    fn falls_back_to_the_command_when_no_title_is_given() {
        let config = parse_str("a.yaml", "menu:\n  - shell: echo hello\n").unwrap();
        assert_eq!(config.menu[0].label(), "echo hello");
    }

    #[test]
    fn parses_toml() {
        let config = parse_str(
            "a.toml",
            "[[menu]]\ntitle = \"hello\"\nshell = \"echo hello\"\n",
        )
        .unwrap();
        assert_eq!(config.menu[0].label(), "hello");
    }

    #[test]
    fn parses_json() {
        let config = parse_str(
            "a.json",
            r#"{"menu": [{"title": "hello", "shell": "echo hello"}]}"#,
        )
        .unwrap();
        assert_eq!(config.menu[0].label(), "hello");
    }

    #[test]
    fn parses_a_bare_json_array_as_the_menu() {
        let config = parse_str("a.json", r#"[{"title": "hello", "shell": "echo hello"}]"#).unwrap();
        assert_eq!(config.menu[0].label(), "hello");
    }

    #[test]
    fn rejects_unknown_extensions() {
        assert!(parse_str("a.cson", "menu: []").is_err());
    }

    #[test]
    fn reports_the_error_of_the_documented_shape_not_the_shorthand() {
        // `shell` is a mapping, which neither shape accepts. The message must
        // point at the offending entry, not say "expected a sequence" (which
        // is all the shorthand attempt can report about a top-level mapping).
        let err = parse_str("a.yaml", "menu:\n  - title: t\n    shell:\n      a: b\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("menu[0]"), "unexpected message: {err}");
        assert!(err.contains("Shell"), "unexpected message: {err}");
    }

    #[test]
    fn a_file_that_says_nothing_about_auto_launchers_leaves_it_undecided() {
        let config = parse_str("a.yaml", "menu: []").unwrap();
        assert!(
            config.auto_launchers.is_none(),
            "otherwise this file would override a parent that did decide"
        );
        assert!(AutoLaunchers::default().any(), "the default is all enabled");
    }

    #[test]
    fn auto_launchers_can_be_turned_off_at_once() {
        let config = parse_str("a.yaml", "auto_launchers: false\nmenu: []").unwrap();
        assert!(!config.auto_launchers.unwrap().any());
    }

    #[test]
    fn auto_launchers_can_be_turned_off_individually() {
        let config = parse_str("a.yaml", "auto_launchers:\n  makefile: false\nmenu: []").unwrap();
        let flags = config.auto_launchers.unwrap();
        assert!(!flags.makefile());
        assert!(flags.package_json());
    }

    #[test]
    fn rejects_unknown_keys_so_typos_are_not_silently_ignored() {
        assert!(parse_str("a.yaml", "menuu: []").is_err());
        assert!(parse_str("a.yaml", "menu:\n  - titel: t\n").is_err());
    }
}
