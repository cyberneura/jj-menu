//! End-to-end checks of configuration discovery and merging, driven through
//! the binary so that the documented behaviour is what is actually tested.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    // `cargo test` builds the binary next to the test executable.
    let mut path = std::env::current_exe().expect("test executable path");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join("jj-menu")
}

fn tempdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("jj-menu-it-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create the temporary directory");
    dir
}

/// Run `jj-menu --show-config` with the search starting at `dir`.
///
/// `HOME` and `XDG_CONFIG_HOME` are pointed at an empty directory so the
/// developer's own `~/.config/jj-menu/` cannot change the result.
fn show_config(dir: &Path) -> String {
    let empty_home = dir.join(".empty-home");
    fs::create_dir_all(&empty_home).expect("create the fake home");

    let output = Command::new(bin())
        .arg("--show-config")
        .arg("--cwd")
        .arg(dir)
        .env("HOME", &empty_home)
        .env("XDG_CONFIG_HOME", &empty_home)
        .output()
        .expect("run jj-menu");

    assert!(
        output.status.success(),
        "jj-menu failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn entry_count(report: &str) -> usize {
    report
        .lines()
        .find_map(|line| line.strip_prefix("menu entries from configuration: "))
        .expect("the report states the entry count")
        .trim()
        .parse()
        .expect("the entry count is a number")
}

fn one_entry(title: &str) -> String {
    format!("menu:\n  - title: {title}\n    shell: 'true'\n")
}

#[test]
fn merges_a_directory_with_its_ancestors() {
    let root = tempdir("merge-ancestors");
    let nested = root.join("a/b");
    fs::create_dir_all(&nested).unwrap();
    fs::write(root.join(".jj-menu.yaml"), one_entry("from root")).unwrap();
    fs::write(nested.join(".jj-menu.yaml"), one_entry("from nested")).unwrap();

    let report = show_config(&nested);
    assert_eq!(entry_count(&report), 2, "{report}");

    // Nearest first: the nested file is loaded before the root one.
    let nested_at = report.find("a/b/.jj-menu.yaml").expect("nested listed");
    let root_line = format!("{}/.jj-menu.yaml", root.display());
    let root_at = report.find(&root_line).expect("root listed");
    assert!(nested_at < root_at, "{report}");
}

#[test]
fn skips_a_fallback_file_when_a_nearer_one_exists() {
    let root = tempdir("fallback-skipped");
    let nested = root.join("a");
    fs::create_dir_all(&nested).unwrap();
    fs::write(
        root.join(".jj-menu.yaml"),
        format!("merge: false\n{}", one_entry("from root")),
    )
    .unwrap();
    fs::write(nested.join(".jj-menu.yaml"), one_entry("from nested")).unwrap();

    let report = show_config(&nested);
    assert_eq!(entry_count(&report), 1, "{report}");
    let root_line = format!("{}/.jj-menu.yaml", root.display());
    assert!(
        !report.contains(&root_line),
        "the fallback must not be listed as loaded: {report}"
    );
}

#[test]
fn uses_a_fallback_file_when_nothing_nearer_exists() {
    let root = tempdir("fallback-used");
    let nested = root.join("a");
    fs::create_dir_all(&nested).unwrap();
    fs::write(
        root.join(".jj-menu.yaml"),
        format!("merge: false\n{}", one_entry("from root")),
    )
    .unwrap();

    let report = show_config(&nested);
    assert_eq!(entry_count(&report), 1, "{report}");
}

#[test]
fn loads_the_shared_file_before_the_local_override() {
    let dir = tempdir("local-override");
    fs::write(dir.join(".jj-menu.yaml"), one_entry("shared")).unwrap();
    fs::write(dir.join(".jj-menu.local.yaml"), one_entry("personal")).unwrap();

    let report = show_config(&dir);
    assert_eq!(entry_count(&report), 2, "{report}");
    let shared = report.find(".jj-menu.yaml").expect("shared listed");
    let local = report.find(".jj-menu.local.yaml").expect("local listed");
    assert!(shared < local, "{report}");
}

#[test]
fn reports_no_configuration_when_none_exists() {
    let dir = tempdir("none");
    let report = show_config(&dir);
    assert!(report.contains("no configuration file found"), "{report}");
    assert_eq!(entry_count(&report), 0, "{report}");
}

#[test]
fn reads_yaml_toml_and_json_alike() {
    let cases = [
        (".jj-menu.yaml", "menu:\n  - title: t\n    shell: 'true'\n"),
        (
            ".jj-menu.toml",
            "[[menu]]\ntitle = \"t\"\nshell = \"true\"\n",
        ),
        (
            ".jj-menu.json",
            r#"{"menu": [{"title": "t", "shell": "true"}]}"#,
        ),
    ];

    for (file, body) in cases {
        let dir = tempdir(&format!("format-{file}"));
        fs::write(dir.join(file), body).unwrap();
        let report = show_config(&dir);
        assert_eq!(entry_count(&report), 1, "{file}: {report}");
    }
}

#[test]
fn fails_with_a_message_naming_the_broken_file() {
    let dir = tempdir("broken");
    let path = dir.join(".jj-menu.yaml");
    fs::write(&path, "menu:\n  - titel: typo\n").unwrap();
    let empty_home = dir.join(".empty-home");
    fs::create_dir_all(&empty_home).unwrap();

    let output = Command::new(bin())
        .arg("--show-config")
        .arg("--cwd")
        .arg(&dir)
        .env("HOME", &empty_home)
        .env("XDG_CONFIG_HOME", &empty_home)
        .output()
        .expect("run jj-menu");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(".jj-menu.yaml"), "{stderr}");
    assert!(stderr.contains("titel"), "{stderr}");
}
