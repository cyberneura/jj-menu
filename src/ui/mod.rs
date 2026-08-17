//! The interactive menu.
//!
//! The terminal is put into raw mode on the alternate screen while the menu is
//! open and fully restored before anything else happens, so the selected
//! command inherits a clean TTY (see [`crate::exec`]).

pub mod prompt;
pub mod state;
pub mod theme;

use std::collections::HashMap;
use std::io::{Write, stderr};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, terminal};

use crate::config::{Job, Launch, MenuItem};
use prompt::LineEditor;
use state::{Frame, MenuState};
use theme::{Style, paint_with};

/// What the user picked.
pub enum Outcome {
    /// Run this, with any arguments already filled in.
    Run(Launch),
    /// Leave without running anything.
    Cancelled,
}

/// Restores the terminal when the menu ends, including on a panic or an early
/// return, so a crash cannot leave the shell in raw mode. A signal that kills
/// the process outright runs no destructor; [`crate::signal`] covers that.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        // Before raw mode, so that what the signal handler restores is the
        // state the shell had. Drop cannot run when the process is killed
        // outright, and this is the only cover for that (see crate::signal).
        crate::signal::arm_terminal_restore();
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
        // The terminal is the caller's again, so the handler must stop
        // touching it -- the selected command runs next and owns it.
        crate::signal::disarm_terminal_restore();
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
                if item.launch().is_none() {
                    menu.enter_detail();
                    continue;
                }
                match resolve(&item)? {
                    Some(launch) => return Ok(Outcome::Run(launch)),
                    // Argument input was cancelled: stay in the menu.
                    None => continue,
                }
            }
            Action::None => {}
        }
    }
}

/// Fill in the arguments of `item` and return what to run.
///
/// `None` means the user cancelled while entering arguments.
fn resolve(item: &MenuItem) -> Result<Option<Launch>> {
    let Some(launch) = item.launch() else {
        return Ok(None);
    };
    if item.args.is_empty() {
        return Ok(Some(launch));
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

    // The arguments belong to the entry, so every command of a parallel group
    // gets the same values: one prompt, however many shells it starts.
    let launch = match launch {
        Launch::Script(script) => Launch::Script(prompt::substitute(&script, &values)),
        Launch::Parallel(jobs) => Launch::Parallel(
            jobs.into_iter()
                .map(|job| Job {
                    title: job.title,
                    script: prompt::substitute(&job.script, &values),
                })
                .collect(),
        ),
    };
    Ok(Some(launch))
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
    let frame = render(menu, cols, rows, theme::enabled())?;
    flush(&frame)
}

/// Build one frame. Separate from [`draw`] so it can be rendered and inspected
/// without a terminal.
///
/// `color` is passed in rather than read inside: [`theme::enabled`] is decided
/// once per process, so a test could otherwise only ever see one of the two
/// modes — and the fallback for the other one is exactly what needs guarding.
fn render(menu: &mut MenuState, cols: u16, rows: u16, color: bool) -> Result<Vec<u8>> {
    let help_rows = menu.frame().help.as_ref().map_or(0, |_| 2);
    let list_height = rows.saturating_sub(CHROME_ROWS + help_rows).max(1) as usize;
    menu.scroll_into_view(list_height);

    // The whole frame is built in memory and written once. `stderr` is
    // unbuffered, so painting straight to it would turn every colour change
    // into its own write, and a redraw of a full screen into hundreds of them
    // — visibly torn over a slow link.
    let mut out = Vec::new();
    execute!(
        out,
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(0, 0)
    )?;

    let title = truncate(&menu.frame().title, cols);
    paint_with(color, &mut out, Style::fg(theme::TITLE).bold(), &title)?;
    if menu.depth() > 1 {
        // The hint only fits in whatever the title left of the line.
        let hint = truncate(
            " (h/← to go back)",
            cols.saturating_sub(title.chars().count() as u16),
        );
        paint_with(color, &mut out, Style::fg(theme::MUTED), &hint)?;
    }
    writeln!(out, "\r")?;

    if let Some(help) = &menu.frame().help {
        paint_with(
            color,
            &mut out,
            Style::fg(theme::MUTED),
            &truncate(help, cols),
        )?;
        writeln!(out, "\r")?;
        writeln!(out, "\r")?;
    }
    writeln!(out, "\r")?;

    if menu.is_empty() {
        paint_with(color, &mut out, Style::fg(theme::MUTED), "  (no entries)")?;
        writeln!(out, "\r")?;
    }

    let offset = menu.offset();
    for (index, item) in menu
        .items()
        .iter()
        .enumerate()
        .skip(offset)
        .take(list_height)
    {
        let selected = index == menu.cursor();
        let detail_marker = if item.has_detail() { " >" } else { "  " };
        let line = format!(
            "{} {}{detail_marker}",
            cursor_marker(selected),
            item.label()
        );

        if selected {
            // Padded first, so the highlight is a bar across the whole width
            // rather than a patch the length of the label.
            let line = pad(&truncate(&line, cols), cols);
            let (marker, label) = split_marker(&line);
            // The two runs share a background, so the bar reads as one block
            // with the marker picked out in front of it.
            paint_with(
                color,
                &mut out,
                Style::fg(theme::CURSOR)
                    .on(theme::SELECTED_BG)
                    .bold()
                    .highlight(),
                marker,
            )?;
            paint_with(
                color,
                &mut out,
                Style::fg(theme::SELECTED_FG)
                    .on(theme::SELECTED_BG)
                    .bold()
                    .highlight(),
                label,
            )?;
        } else {
            let style = if item.has_detail() {
                Style::fg(theme::CONTAINER)
            } else {
                Style::default()
            };
            paint_with(color, &mut out, style, &truncate(&line, cols))?;
        }
        writeln!(out, "\r")?;
    }

    let footer = match menu.selected() {
        Some(item) => match item.launch() {
            Some(Launch::Script(script)) => format!("$ {}", one_line(&script)),
            // `&` between the commands, the way a shell would be told to run
            // them at once, so the status line says which of the two an entry
            // is without spending a word on it.
            Some(Launch::Parallel(jobs)) => format!(
                "& {}",
                jobs.iter()
                    .map(|job| one_line(&job.script))
                    .collect::<Vec<_>>()
                    .join(" & ")
            ),
            None => format!("> {} entries", item.submenu.len()),
        },
        None => String::new(),
    };
    let footer = pad(&truncate(&footer, cols), cols);
    let (sigil, rest) = split_marker(&footer);
    execute!(out, cursor::MoveTo(0, rows.saturating_sub(1)))?;
    paint_with(
        color,
        &mut out,
        Style::fg(theme::COMMAND)
            .on(theme::FOOTER_BG)
            .bold()
            .highlight(),
        sigil,
    )?;
    paint_with(
        color,
        &mut out,
        Style::fg(theme::FOOTER_FG).on(theme::FOOTER_BG).highlight(),
        rest,
    )?;

    Ok(out)
}

/// Send a frame built by [`render`] or [`draw_prompt`] to the terminal in one
/// write.
fn flush(frame: &[u8]) -> Result<()> {
    let mut out = stderr();
    out.write_all(frame)?;
    out.flush()?;
    Ok(())
}

/// A multi-line script as one status-line-sized preview.
fn one_line(script: &str) -> String {
    script.replace('\n', " ; ")
}

/// Split the two-character sigil (`*>`, `$ `, `> `, `& `) off the front of a line so
/// it can be coloured on its own. Both halves may be empty on a very narrow
/// terminal.
fn split_marker(line: &str) -> (&str, &str) {
    match line.char_indices().nth(2) {
        Some((at, _)) => line.split_at(at),
        None => (line, ""),
    }
}

fn draw_prompt(label: &str, editor: &LineEditor) -> Result<()> {
    let (cols, rows) = terminal_size();
    let mut out = Vec::new();

    let line = format!("{label}: {}", editor.text());
    let line = truncate(&line, cols);
    let (prompt, value) = match line.char_indices().nth(label.chars().count() + 1) {
        Some((at, _)) => line.split_at(at),
        None => (line.as_str(), ""),
    };
    execute!(out, cursor::MoveTo(0, rows.saturating_sub(1)))?;
    execute!(out, terminal::Clear(terminal::ClearType::CurrentLine))?;
    let color = theme::enabled();
    paint_with(color, &mut out, Style::fg(theme::TITLE).bold(), prompt)?;
    paint_with(color, &mut out, Style::default(), value)?;

    // Put the real cursor where the text cursor is, so the terminal's own
    // caret marks the insertion point.
    let column = (label.chars().count() + 2 + editor.cursor()).min(cols as usize - 1);
    execute!(
        out,
        cursor::MoveTo(column as u16, rows.saturating_sub(1)),
        cursor::Show
    )?;

    flush(&out)
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

    /// The scripts a resolved entry would run, whichever kind it is.
    fn scripts(launch: Launch) -> Vec<String> {
        match launch {
            Launch::Script(script) => vec![script],
            Launch::Parallel(jobs) => jobs.into_iter().map(|job| job.script).collect(),
        }
    }

    fn parallel_item(scripts: &[&str]) -> MenuItem {
        MenuItem {
            title: Some("group".into()),
            parallel: scripts
                .iter()
                .map(|script| crate::config::model::ParallelCommand {
                    title: None,
                    shell: crate::config::model::Shell::One((*script).to_string()),
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn resolves_an_entry_without_arguments_directly() {
        let item = MenuItem::command("t", "echo hi");
        assert_eq!(scripts(resolve(&item).unwrap().unwrap()), ["echo hi"]);
    }

    #[test]
    fn resolves_a_parallel_entry_into_one_job_per_command() {
        let item = parallel_item(&["npm run dev", "npm run api"]);
        assert_eq!(
            scripts(resolve(&item).unwrap().unwrap()),
            ["npm run dev", "npm run api"]
        );
    }

    #[test]
    fn a_parallel_entry_is_runnable_rather_than_openable() {
        // Enter must run the group; the menu opens an entry only when there is
        // nothing to run, and a `parallel` entry has no `shell`.
        let item = parallel_item(&["true"]);
        assert!(item.launch().is_some());
        assert!(!item.has_detail());
    }

    #[test]
    fn the_status_line_tells_a_parallel_entry_apart_from_a_single_command() {
        let mut menu = MenuState::new(Frame::new(
            "title",
            vec![parallel_item(&["npm run dev", "npm run api"])],
        ));
        let frame = String::from_utf8(render(&mut menu, 60, 10, false).unwrap()).unwrap();
        let text = without_escapes(&frame);
        assert!(
            text.contains("& npm run dev & npm run api"),
            "the status line should show both commands: {text:?}"
        );
        assert!(
            !text.contains("> 0 entries"),
            "a parallel entry is not an empty container: {text:?}"
        );
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

    /// Render a small menu and return the frame as text, escapes included.
    ///
    /// This is the only look at the drawing code that does not need a
    /// terminal. `color` is passed to [`render`] rather than taken from the
    /// environment, so one test run covers both modes.
    fn rendered(color: bool) -> String {
        let items = vec![
            MenuItem::command("plain", "echo hi"),
            MenuItem {
                title: Some("with help".into()),
                help: Some("some help".into()),
                ..Default::default()
            },
        ];
        let mut menu = MenuState::new(Frame::new("title", items));
        menu.move_down();
        String::from_utf8(render(&mut menu, 40, 10, color).unwrap()).unwrap()
    }

    /// Drop the escape sequences, leaving what the user actually sees. A row is
    /// painted in several runs, so its text is not contiguous in the frame.
    fn without_escapes(frame: &str) -> String {
        let mut text = String::new();
        let mut chars = frame.chars();
        while let Some(c) = chars.next() {
            if c != '\u{1b}' {
                text.push(c);
                continue;
            }
            // Past the `[` that opens a CSI sequence; it ends at the first
            // byte in `@`..=`~`, which the `[` itself would otherwise match.
            let mut chars = chars.by_ref().skip(1);
            for c in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    break;
                }
            }
        }
        text
    }

    /// The selected row of a rendered frame, escapes still in place.
    ///
    /// Rows are matched on their visible text, so the assertions cannot be
    /// satisfied by some other part of the frame — the status line is
    /// highlighted too, and would otherwise stand in for a selected row that
    /// had lost its highlight entirely.
    fn selected_row(frame: &str) -> &str {
        frame
            .split("\r\n")
            .find(|row| without_escapes(row).contains("*> with help"))
            .unwrap_or_else(|| panic!("the selected entry is drawn: {frame:?}"))
    }

    #[test]
    fn the_selected_row_is_a_full_width_bar() {
        for color in [true, false] {
            let frame = rendered(color);
            let bar = selected_row(&frame);
            // Padded to the width, so the highlight covers the whole row.
            assert_eq!(
                without_escapes(bar).chars().count(),
                39,
                "the bar should run to the edge, got {bar:?}"
            );
        }
    }

    #[test]
    fn the_selected_row_is_marked_out_whether_or_not_color_is_on() {
        for color in [true, false] {
            let frame = rendered(color);
            let bar = selected_row(&frame);
            // The row is drawn as two runs — the marker and the rest — and
            // the highlight has to cover both, or the bar has a hole in it.
            // Without colour that means reverse video, which is the whole
            // point of `Style::highlight`: drop it and this fails.
            let sequence = if color {
                "\u{1b}[48;5;12m"
            } else {
                "\u{1b}[7m"
            };
            assert_eq!(
                bar.matches(sequence).count(),
                2,
                "{sequence:?} does not cover {bar:?}"
            );
        }
    }

    #[test]
    fn an_entry_with_a_detail_view_is_told_apart_from_a_plain_command() {
        let frame = rendered(true);
        let plain = frame
            .lines()
            .find(|line| line.contains("plain"))
            .expect("the unselected entry is drawn");
        // An unselected plain command is drawn as-is, with no escapes at all.
        // (`lines` has already taken the `\r` of the raw-mode line ending.)
        assert_eq!(plain, "   plain  ", "got {plain:?}");

        let detail = without_escapes(&frame);
        let detail = detail
            .lines()
            .find(|line| line.contains("with help"))
            .expect("the entry with a detail view is drawn");
        assert!(
            detail.contains("with help >"),
            "the detail marker is missing from {detail:?}"
        );
    }

    #[test]
    fn maps_readline_motions() {
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        assert!(matches!(classify(&ctrl('n')), Action::Down));
        assert!(matches!(classify(&ctrl('p')), Action::Up));
        assert!(matches!(classify(&ctrl('c')), Action::Quit));
    }
}
