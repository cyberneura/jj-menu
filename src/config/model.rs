//! Data model for the menu configuration.
//!
//! The same model is produced from YAML, TOML and JSON, so parsing lives in
//! [`super::loader`] and only the shape is described here.

use serde::Deserialize;

/// A whole configuration file.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    /// Menu entries defined by this file.
    #[serde(default)]
    pub menu: Vec<MenuItem>,

    /// When `false`, this file is skipped if another configuration file has
    /// already been loaded. Defaults to `true` (files are merged).
    #[serde(default = "default_true")]
    pub merge: bool,

    /// Controls the built-in launchers (`package.json`, `Makefile`, Cargo,
    /// Gradle). `None` means the file said nothing, which is different from
    /// spelling out the defaults: only a file that decides stops files further
    /// up from deciding.
    #[serde(default)]
    pub auto_launchers: Option<AutoLaunchers>,
}

/// A single menu entry.
///
/// `shell` accepts either a single command or a list of commands. A list is
/// joined with newlines and handed to the shell as one script, so the commands
/// run sequentially in the same shell (this is how `cd` followed by another
/// command keeps working).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct MenuItem {
    /// Label shown in the menu. Falls back to `shell` when omitted.
    #[serde(default)]
    pub title: Option<String>,

    /// Command(s) to run.
    #[serde(default)]
    pub shell: Option<Shell>,

    /// Long description shown in the detail view.
    #[serde(default)]
    pub help: Option<String>,

    /// Nested entries, opened from the detail view.
    #[serde(default)]
    pub submenu: Vec<MenuItem>,

    /// Placeholders that are prompted for before running `shell`.
    #[serde(default)]
    pub args: Vec<ArgSpec>,
}

/// One or many shell commands.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Shell {
    One(String),
    Many(Vec<String>),
}

impl Shell {
    /// Render as a single script.
    pub fn script(&self) -> String {
        match self {
            Shell::One(s) => s.clone(),
            Shell::Many(v) => v.join("\n"),
        }
    }
}

/// A value asked for interactively and substituted into `shell` as `{name}`.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ArgSpec {
    /// Placeholder name. `{name}` in `shell` is replaced with the input.
    pub name: String,

    /// Text shown while asking. Falls back to `name`.
    #[serde(default)]
    pub prompt: Option<String>,

    /// Value used when the input is left empty.
    #[serde(default)]
    pub default: Option<String>,
}

/// Which built-in launchers to scan for.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum AutoLaunchers {
    /// `auto_launchers: false` turns every launcher off at once.
    All(bool),
    /// Per-launcher switches.
    Each(AutoLauncherFlags),
}

impl Default for AutoLaunchers {
    fn default() -> Self {
        AutoLaunchers::All(true)
    }
}

impl AutoLaunchers {
    pub fn package_json(&self) -> bool {
        self.flag(|f| f.package_json)
    }

    pub fn makefile(&self) -> bool {
        self.flag(|f| f.makefile)
    }

    pub fn cargo(&self) -> bool {
        self.flag(|f| f.cargo)
    }

    pub fn gradle(&self) -> bool {
        self.flag(|f| f.gradle)
    }

    /// `true` when at least one launcher is enabled.
    pub fn any(&self) -> bool {
        self.package_json() || self.makefile() || self.cargo() || self.gradle()
    }

    fn flag(&self, pick: impl Fn(&AutoLauncherFlags) -> bool) -> bool {
        match self {
            AutoLaunchers::All(enabled) => *enabled,
            AutoLaunchers::Each(flags) => pick(flags),
        }
    }
}

/// Per-launcher switches. Every launcher defaults to enabled, so a file only
/// needs to name the ones it wants to turn off.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutoLauncherFlags {
    #[serde(default = "default_true")]
    pub package_json: bool,
    #[serde(default = "default_true")]
    pub makefile: bool,
    #[serde(default = "default_true")]
    pub cargo: bool,
    #[serde(default = "default_true")]
    pub gradle: bool,
}

impl Default for AutoLauncherFlags {
    fn default() -> Self {
        Self {
            package_json: true,
            makefile: true,
            cargo: true,
            gradle: true,
        }
    }
}

fn default_true() -> bool {
    true
}

impl MenuItem {
    /// Label to render in the menu.
    pub fn label(&self) -> String {
        if let Some(title) = &self.title
            && !title.is_empty()
        {
            return title.clone();
        }
        match &self.shell {
            Some(shell) => first_line(&shell.script()).to_string(),
            None => String::new(),
        }
    }

    /// The script this entry runs, if any.
    pub fn script(&self) -> Option<String> {
        self.shell.as_ref().map(|s| s.script())
    }

    /// Whether the detail view has anything to show for this entry.
    ///
    /// Arguments deliberately do not count. They are prompted for when the
    /// entry runs, so a detail view would add nothing — and the view puts the
    /// entry's own command in a `Run: ...` line that carries the same
    /// arguments, which would then be openable again, and again.
    pub fn has_detail(&self) -> bool {
        !self.submenu.is_empty() || self.help.is_some()
    }

    /// Build a plain entry, used by the built-in launchers.
    pub fn command(title: impl Into<String>, shell: impl Into<String>) -> Self {
        Self {
            title: Some(title.into()),
            shell: Some(Shell::One(shell.into())),
            ..Default::default()
        }
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}
