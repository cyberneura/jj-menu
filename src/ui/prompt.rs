//! Argument prompting and placeholder substitution.
//!
//! The line editing logic is kept free of terminal handling so it can be
//! tested without a TTY.

use std::collections::HashMap;

use crate::config::ArgSpec;

/// A single-line text field.
#[derive(Debug, Clone, Default)]
pub struct LineEditor {
    /// The text, as characters, so cursor movement is not byte-based (a
    /// multi-byte character must move the cursor by one, not by its length).
    chars: Vec<char>,
    cursor: usize,
}

impl LineEditor {
    pub fn with_text(text: &str) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let cursor = chars.len();
        Self { chars, cursor }
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn insert(&mut self, c: char) {
        self.chars.insert(self.cursor, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        if self.cursor < self.chars.len() {
            self.cursor += 1;
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.chars.len();
    }

    /// Delete from the cursor to the start of the line (readline's `Ctrl-U`).
    pub fn clear_to_start(&mut self) {
        self.chars.drain(..self.cursor);
        self.cursor = 0;
    }
}

/// Replace `{name}` in `script` with the collected values.
///
/// Substitution is a single left-to-right pass over the template, so a value
/// that itself looks like a placeholder is never expanded again.
///
/// Values are inserted verbatim. They are typed by the person running the
/// menu and end up in a shell script, so quoting is deliberately left to the
/// template author (`grep {pattern}` vs `grep "{pattern}"`); quoting here
/// would make it impossible to pass flags or globs.
pub fn substitute(script: &str, values: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(script.len());
    let mut rest = script;

    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];

        match after.find('}') {
            Some(end) => {
                let name = &after[..end];
                match values.get(name) {
                    Some(value) => out.push_str(value),
                    // Unknown placeholders are left alone: the text may well
                    // be shell syntax such as `${VAR}` or a brace expansion.
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[end + 1..];
            }
            // No closing brace: the rest is literal text.
            None => {
                out.push('{');
                out.push_str(after);
                return out;
            }
        }
    }

    out.push_str(rest);
    out
}

/// Prompt label for an argument.
pub fn label(spec: &ArgSpec) -> String {
    spec.prompt.clone().unwrap_or_else(|| spec.name.clone())
}

/// The value to start the field with.
pub fn initial_value(spec: &ArgSpec) -> String {
    spec.default.clone().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn substitutes_a_placeholder() {
        assert_eq!(
            substitute("rg {pattern}", &values(&[("pattern", "TODO")])),
            "rg TODO"
        );
    }

    #[test]
    fn substitutes_several_placeholders_including_repeats() {
        assert_eq!(
            substitute("cp {a} {b} && echo {a}", &values(&[("a", "x"), ("b", "y")])),
            "cp x y && echo x"
        );
    }

    #[test]
    fn leaves_unknown_placeholders_alone_so_shell_syntax_survives() {
        assert_eq!(substitute("echo ${HOME}", &values(&[])), "echo ${HOME}");
        assert_eq!(substitute("touch a{1,2}", &values(&[])), "touch a{1,2}");
    }

    #[test]
    fn leaves_an_unclosed_brace_as_literal_text() {
        assert_eq!(substitute("echo {oops", &values(&[])), "echo {oops");
    }

    #[test]
    fn does_not_expand_a_substituted_value_again() {
        // The value looks like another placeholder; it must survive as text.
        assert_eq!(
            substitute("echo {a}", &values(&[("a", "{b}"), ("b", "boom")])),
            "echo {b}"
        );
    }

    #[test]
    fn edits_a_line_by_character_not_by_byte() {
        let mut editor = LineEditor::with_text("日本語");
        assert_eq!(editor.cursor(), 3);
        editor.backspace();
        assert_eq!(editor.text(), "日本");
        editor.home();
        editor.insert('あ');
        assert_eq!(editor.text(), "あ日本");
        assert_eq!(editor.cursor(), 1);
    }

    #[test]
    fn moves_the_cursor_within_bounds() {
        let mut editor = LineEditor::with_text("ab");
        editor.right();
        assert_eq!(editor.cursor(), 2, "must not move past the end");
        editor.home();
        editor.left();
        assert_eq!(editor.cursor(), 0, "must not move before the start");
    }

    #[test]
    fn deletes_forwards_and_backwards() {
        let mut editor = LineEditor::with_text("abc");
        editor.home();
        editor.delete();
        assert_eq!(editor.text(), "bc");
        editor.end();
        editor.backspace();
        assert_eq!(editor.text(), "b");
        editor.delete();
        assert_eq!(editor.text(), "b", "delete at the end is a no-op");
    }

    #[test]
    fn clears_to_the_start_of_the_line() {
        let mut editor = LineEditor::with_text("abcdef");
        editor.home();
        editor.right();
        editor.right();
        editor.clear_to_start();
        assert_eq!(editor.text(), "cdef");
        assert_eq!(editor.cursor(), 0);
    }

    #[test]
    fn falls_back_to_the_name_when_no_prompt_is_given() {
        let spec = ArgSpec {
            name: "pattern".into(),
            prompt: None,
            default: None,
        };
        assert_eq!(label(&spec), "pattern");
        assert_eq!(initial_value(&spec), "");
    }
}
