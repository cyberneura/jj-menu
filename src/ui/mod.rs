//! The interactive menu.
//!
//! The terminal is put into raw mode on the alternate screen while the menu is
//! open and fully restored before anything else happens, so the selected
//! command inherits a clean TTY (see [`crate::exec`]).

pub mod prompt;
pub mod state;

use std::collections::HashMap;
use std::io::{Write, stderr};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, style, terminal};

use crate::config::MenuItem;
use prompt::LineEditor;
use state::{Frame, MenuState};

/// What the user picked.
pub enum Outcome {
    /// Run this script.
    Run(String),
    /// Leave without running anything.
    Cancelled,
}

/// Restores the terminal when the menu ends, including on a panic or an early
/// return, so a crash cannot leave the shell in raw mode.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        // Armed before entering the alternate screen: if that fails, the guard
        // is dropped on the way out and raw mode is undone. Constructing it
        // afterwards would leave the shell in raw mode on such a failure.
        let guard = Self;
        execute!(stderr(), terminal::EnterAlternateScreen, cursor::Hide)?;
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(stderr(), cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

/// Open the menu and return what the user picked.
///
/// The UI is drawn on stderr so that stdout stays free for `--print`, which
/// lets `jj-menu --print | ...` work while the menu is on screen.
pub fn run(items: Vec<MenuItem>, title: &str) -> Result<Outcome> {
    let _guard = TerminalGuard::enter()?;
    let mut menu = MenuState::new(Frame::new(title, items));

    loop {
        draw(&mut menu)?;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        // Windows reports both press and release; only act on press.
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match classify(&key) {
            Action::Quit => return Ok(Outcome::Cancelled),
            Action::Down => menu.move_down(),
            Action::Up => menu.move_up(),
            Action::First => menu.move_first(),
            Action::Last => menu.move_last(),
            Action::Back => {
                if !menu.leave_detail() {
                    return Ok(Outcome::Cancelled);
                }
            }
            Action::Forward => {
                // The detail view is where help, submenus and arguments live.
                menu.enter_detail();
            }
            Action::Select => {
                let Some(item) = menu.selected().cloned() else {
                    continue;
                };
                // A container without a command opens instead of running, so
                // Enter never silently does nothing.
                if item.script().is_none() {
                    menu.enter_detail();
                    continue;
                }
                match resolve(&item)? {
                    Some(script) => return Ok(Outcome::Run(script)),
                    // Argument input was cancelled: stay in the menu.
                    None => continue,
                }
            }
            Action::None => {}
        }
    }
}

/// Fill in the arguments of `item` and return the script to run.
///
/// `None` means the user cancelled while entering arguments.
fn resolve(item: &MenuItem) -> Result<Option<String>> {
    let script = item.script().unwrap_or_default();
    if item.args.is_empty() {
        return Ok(Some(script));
    }

    let mut values: HashMap<String, String> = HashMap::new();
    for spec in &item.args {
        match ask(&prompt::label(spec), &prompt::initial_value(spec))? {
            Some(value) => {
                values.insert(spec.name.clone(), value);
            }
            None => return Ok(None),
        }
    }

    Ok(Some(prompt::substitute(&script, &values)))
}

/// Ask for one value. `None` means cancelled.
fn ask(label: &str, initial: &str) -> Result<Option<String>> {
    let mut editor = LineEditor::with_text(initial);

    loop {
        draw_prompt(label, &editor)?;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Enter => return Ok(Some(editor.text())),
            KeyCode::Esc => return Ok(None),
            KeyCode::Char('c' | 'd') if ctrl => return Ok(None),
            KeyCode::Char('u') if ctrl => editor.clear_to_start(),
            KeyCode::Char('a') if ctrl => editor.home(),
            KeyCode::Char('e') if ctrl => editor.end(),
            KeyCode::Backspace => editor.backspace(),
            KeyCode::Delete => editor.delete(),
            KeyCode::Left => editor.left(),
            KeyCode::Right => editor.right(),
            KeyCode::Home => editor.home(),
            KeyCode::End => editor.end(),
            // Any other modifier combination is a shortcut, not text.
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                editor.insert(c)
            }
            _ => {}
        }
    }
}

/// A key press, resolved to what it should do.
enum Action {
    Up,
    Down,
    First,
    Last,
    Forward,
    Back,
    Select,
    Quit,
    None,
}

/// Map a key press to an action.
///
/// Both vi keys and arrows are accepted, plus the readline motions `Ctrl-N` /
/// `Ctrl-P`, which the original Python implementation also supported.
fn classify(key: &KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('c') if ctrl => Action::Quit,
        KeyCode::Char('n') if ctrl => Action::Down,
        KeyCode::Char('p') if ctrl => Action::Up,
        KeyCode::Char('j') | KeyCode::Down => Action::Down,
        KeyCode::Char('k') | KeyCode::Up => Action::Up,
        KeyCode::Char('l') | KeyCode::Right => Action::Forward,
        KeyCode::Char('h') | KeyCode::Left => Action::Back,
        KeyCode::Char('g') | KeyCode::Home => Action::First,
        KeyCode::Char('G') | KeyCode::End => Action::Last,
        KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
        KeyCode::Enter => Action::Select,
        _ => Action::None,
    }
}

/// Rows reserved around the list: title, blank line, and the footer.
const CHROME_ROWS: u16 = 3;

fn draw(menu: &mut MenuState) -> Result<()> {
    let (cols, rows) = terminal_size();
    let help_rows = menu.frame().help.as_ref().map_or(0, |_| 2);
    let list_height = rows.saturating_sub(CHROME_ROWS + help_rows).max(1) as usize;
    menu.scroll_into_view(list_height);

    let mut out = stderr();
    execute!(
        out,
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(0, 0)
    )?;

    let breadcrumb = if menu.depth() > 1 {
        format!("{} (h/← to go back)", menu.frame().title)
    } else {
        menu.frame().title.clone()
    };
    writeln!(out, "{}\r", truncate(&breadcrumb, cols))?;

    if let Some(help) = &menu.frame().help {
        writeln!(out, "{}\r", truncate(help, cols))?;
        writeln!(out, "\r")?;
    }
    writeln!(out, "\r")?;

    if menu.is_empty() {
        writeln!(out, "  (no entries)\r")?;
    }

    let offset = menu.offset();
    for (index, item) in menu
        .items()
        .iter()
        .enumerate()
        .skip(offset)
        .take(list_height)
    {
        let marker = if item.has_detail() { " >" } else { "  " };
        let line = format!(
            "{} {}{marker}",
            cursor_marker(index == menu.cursor()),
            item.label()
        );
        if index == menu.cursor() {
            execute!(out, style::SetAttribute(style::Attribute::Reverse))?;
            write!(out, "{}", truncate(&line, cols))?;
            execute!(out, style::SetAttribute(style::Attribute::Reset))?;
            writeln!(out, "\r")?;
        } else {
            writeln!(out, "{}\r", truncate(&line, cols))?;
        }
    }

    let footer = match menu.selected() {
        Some(item) => match item.script() {
            Some(script) => format!("$ {}", script.replace('\n', " ; ")),
            None => format!("> {} entries", item.submenu.len()),
        },
        None => String::new(),
    };
    execute!(out, cursor::MoveTo(0, rows.saturating_sub(1)))?;
    execute!(out, style::SetAttribute(style::Attribute::Reverse))?;
    write!(out, "{}", pad(&truncate(&footer, cols), cols))?;
    execute!(out, style::SetAttribute(style::Attribute::Reset))?;
    out.flush()?;
    Ok(())
}

fn draw_prompt(label: &str, editor: &LineEditor) -> Result<()> {
    let (cols, rows) = terminal_size();
    let mut out = stderr();

    let line = format!("{label}: {}", editor.text());
    execute!(out, cursor::MoveTo(0, rows.saturating_sub(1)))?;
    execute!(out, terminal::Clear(terminal::ClearType::CurrentLine))?;
    write!(out, "{}", truncate(&line, cols))?;

    // Put the real cursor where the text cursor is, so the terminal's own
    // caret marks the insertion point.
    let column = (label.chars().count() + 2 + editor.cursor()).min(cols as usize - 1);
    execute!(
        out,
        cursor::MoveTo(column as u16, rows.saturating_sub(1)),
        cursor::Show
    )?;
    out.flush()?;
    Ok(())
}

/// Terminal size, with a usable fallback.
///
/// A zero is treated the same as an error: a PTY that was never given a
/// winsize reports `0x0`, and every line would then be truncated to nothing.
fn terminal_size() -> (u16, u16) {
    usable_size(terminal::size().ok())
}

/// Split out so the fallback can be tested without a real terminal.
fn usable_size(reported: Option<(u16, u16)>) -> (u16, u16) {
    let (cols, rows) = reported.unwrap_or((0, 0));
    (
        if cols == 0 { 80 } else { cols },
        if rows == 0 { 24 } else { rows },
    )
}

fn cursor_marker(selected: bool) -> &'static str {
    if selected { "*>" } else { "  " }
}

/// Cut a line to the terminal width, counting characters rather than bytes.
///
/// Control characters are removed first: labels can come from a file in the
/// checkout (an npm script name, a make target), and a JSON string can carry a
/// real ESC or newline. Writing those straight out would let merely *opening*
/// the menu in an untrusted repository run terminal escape sequences — OSC
/// clipboard writes, cursor moves — without anything being selected.
///
/// This is not display-width aware: a wide character (CJK, emoji) counts as
/// one, so a line of wide characters is cut later than it ideally would be.
/// Erring on the side of cutting late keeps ASCII, the common case, exact.
fn truncate(text: &str, cols: u16) -> String {
    let max = cols.saturating_sub(1) as usize;
    sanitize(text).take(max).collect()
}

/// Replace every control character with `·` so it cannot reach the terminal
/// as a command. C1 (`U+0080`–`U+009F`) counts too: in a UTF-8 terminal those
/// are alternative forms of the C0 escape sequences.
fn sanitize(text: &str) -> impl Iterator<Item = char> + '_ {
    text.chars().map(|c| {
        if c.is_control() || ('\u{80}'..='\u{9f}').contains(&c) {
            '·'
        } else {
            c
        }
    })
}

fn pad(text: &str, cols: u16) -> String {
    let width = cols.saturating_sub(1) as usize;
    let len = text.chars().count();
    if len >= width {
        return text.to_string();
    }
    format!("{text}{}", " ".repeat(width - len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_a_usable_size_when_the_terminal_reports_zero() {
        assert_eq!(usable_size(None), (80, 24));
        assert_eq!(usable_size(Some((0, 0))), (80, 24));
        assert_eq!(usable_size(Some((0, 40))), (80, 40));
        assert_eq!(usable_size(Some((120, 0))), (120, 24));
        assert_eq!(usable_size(Some((120, 40))), (120, 40));
    }

    #[test]
    fn truncates_by_character_count() {
        assert_eq!(truncate("abcdef", 4), "abc");
        assert_eq!(truncate("abc", 80), "abc");
        assert_eq!(truncate("日本語です", 4), "日本語");
    }

    #[test]
    fn strips_control_characters_from_repository_controlled_text() {
        // An npm script name or make target can carry a real ESC; none of it
        // may reach the terminal as a command.
        assert_eq!(truncate("a\u{1b}[2Jb", 80), "a·[2Jb");
        assert_eq!(truncate("a\nb", 80), "a·b");
        assert_eq!(truncate("a\u{7}b", 80), "a·b");
        assert_eq!(truncate("a\u{9b}b", 80), "a·b", "C1 CSI must go too");
        assert_eq!(truncate("plain", 80), "plain", "ordinary text is untouched");
    }

    #[test]
    fn pads_to_the_terminal_width() {
        assert_eq!(pad("ab", 5), "ab  ");
        assert_eq!(pad("abcdef", 4), "abcdef");
    }

    #[test]
    fn resolves_an_entry_without_arguments_directly() {
        let item = MenuItem::command("t", "echo hi");
        assert_eq!(resolve(&item).unwrap().unwrap(), "echo hi");
    }

    #[test]
    fn maps_vi_keys_and_arrows_to_the_same_actions() {
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        assert!(matches!(classify(&key(KeyCode::Char('j'))), Action::Down));
        assert!(matches!(classify(&key(KeyCode::Down)), Action::Down));
        assert!(matches!(classify(&key(KeyCode::Char('k'))), Action::Up));
        assert!(matches!(classify(&key(KeyCode::Up)), Action::Up));
        assert!(matches!(
            classify(&key(KeyCode::Char('l'))),
            Action::Forward
        ));
        assert!(matches!(classify(&key(KeyCode::Right)), Action::Forward));
        assert!(matches!(classify(&key(KeyCode::Char('h'))), Action::Back));
        assert!(matches!(classify(&key(KeyCode::Left)), Action::Back));
        assert!(matches!(classify(&key(KeyCode::Char('q'))), Action::Quit));
        assert!(matches!(classify(&key(KeyCode::Esc)), Action::Quit));
        assert!(matches!(classify(&key(KeyCode::Enter)), Action::Select));
    }

    #[test]
    fn maps_readline_motions() {
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        assert!(matches!(classify(&ctrl('n')), Action::Down));
        assert!(matches!(classify(&ctrl('p')), Action::Up));
        assert!(matches!(classify(&ctrl('c')), Action::Quit));
    }
}
