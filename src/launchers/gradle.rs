//! Entries for a Gradle project.
//!
//! Gradle can list a project's real tasks, but only by running
//! `./gradlew tasks`, which starts a JVM and evaluates the build script. That
//! is far too slow and too side-effecting to do while opening a menu, so a
//! fixed set of lifecycle tasks is offered instead, plus a `tasks` entry for
//! discovering the rest interactively.

use std::path::Path;

use super::LauncherGroup;
use crate::config::MenuItem;

/// Lifecycle tasks present in essentially every Gradle build.
const TASKS: &[&str] = &["build", "test", "clean", "assemble", "check", "tasks"];

/// Produce the Gradle entries, preferring the wrapper when it is present.
pub fn scan(path: &Path) -> Option<LauncherGroup> {
    let dir = path.parent()?;
    let wrapper = dir.join("gradlew");

    // The wrapper pins the Gradle version for the project, so use it when
    // available and fall back to a Gradle on PATH otherwise.
    let (runner, source) = if wrapper.is_file() {
        (
            format!("{}/gradlew", super::quote(&dir.to_string_lossy())),
            "gradlew",
        )
    } else {
        ("gradle".to_string(), "build.gradle")
    };

    let items = TASKS
        .iter()
        .map(|task| {
            let label = if source == "gradlew" {
                format!("./gradlew {task}")
            } else {
                format!("gradle {task}")
            };
            MenuItem::command(label, format!("{runner} {task}"))
        })
        .collect();

    Some(LauncherGroup {
        source: source.to_string(),
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("jj-menu-gradle-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn prefers_the_wrapper_when_present() {
        let dir = tempdir("wrapper");
        fs::write(dir.join("gradlew"), "#!/bin/sh\n").unwrap();
        let group = scan(&dir.join("gradlew")).unwrap();
        assert_eq!(group.source, "gradlew");
        assert!(group.items[0].script().unwrap().ends_with("/gradlew build"));
    }

    #[test]
    fn falls_back_to_gradle_on_path_without_a_wrapper() {
        let dir = tempdir("no-wrapper");
        fs::write(dir.join("build.gradle"), "").unwrap();
        let group = scan(&dir.join("build.gradle")).unwrap();
        assert_eq!(group.source, "build.gradle");
        assert_eq!(group.items[0].script().unwrap(), "gradle build");
    }
}
