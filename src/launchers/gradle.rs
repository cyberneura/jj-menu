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
///
/// `wrapper` and `script` are looked up independently by the caller: in a
/// multi-project build the wrapper sits at the root while the subproject has
/// only its own build script, so the nearest of each can be in different
/// directories.
pub fn scan(wrapper: Option<&Path>, script: Option<&Path>) -> Option<LauncherGroup> {
    // The wrapper pins the Gradle version for the project, so use it when
    // available and fall back to a Gradle on PATH otherwise.
    let (runner, source) = match wrapper {
        Some(wrapper) => (super::quote(&wrapper.to_string_lossy()), "gradlew"),
        // No wrapper: there has to be a build script, or the caller would not
        // have asked.
        None => {
            script?;
            ("gradle".to_string(), "build.gradle")
        }
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
        let wrapper = dir.join("gradlew");
        fs::write(&wrapper, "#!/bin/sh\n").unwrap();
        let group = scan(Some(&wrapper), None).unwrap();
        assert_eq!(group.source, "gradlew");
        assert!(group.items[0].script().unwrap().ends_with("gradlew' build"));
    }

    #[test]
    fn falls_back_to_gradle_on_path_without_a_wrapper() {
        let dir = tempdir("no-wrapper");
        let script = dir.join("build.gradle");
        fs::write(&script, "").unwrap();
        let group = scan(None, Some(&script)).unwrap();
        assert_eq!(group.source, "build.gradle");
        assert_eq!(group.items[0].script().unwrap(), "gradle build");
    }

    #[test]
    fn uses_a_root_wrapper_with_a_subproject_build_script() {
        // The multi-project layout: gradlew at the root, build.gradle in the
        // subproject the menu was opened in.
        let root = tempdir("multi-project");
        let wrapper = root.join("gradlew");
        fs::write(&wrapper, "#!/bin/sh\n").unwrap();
        let sub = root.join("app");
        fs::create_dir_all(&sub).unwrap();
        let script = sub.join("build.gradle");
        fs::write(&script, "").unwrap();

        let group = scan(Some(&wrapper), Some(&script)).unwrap();
        assert_eq!(group.source, "gradlew");
        assert_eq!(
            group.items[0].script().unwrap(),
            format!("'{}' build", wrapper.display())
        );
    }

    #[test]
    fn produces_nothing_without_a_wrapper_or_a_build_script() {
        assert!(scan(None, None).is_none());
    }
}
