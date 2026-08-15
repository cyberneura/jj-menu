//! Navigation state of the menu.
//!
//! Kept free of any terminal handling so it can be tested without a TTY.

use crate::config::MenuItem;

/// One level of the menu. Opening a submenu pushes a frame; going back pops
/// it, which is what keeps the cursor position of the parent level.
#[derive(Debug, Clone)]
pub struct Frame {
    pub title: String,
    /// Help text of the entry this frame was opened from.
    pub help: Option<String>,
    pub items: Vec<MenuItem>,
    pub cursor: usize,
}

impl Frame {
    pub fn new(title: impl Into<String>, items: Vec<MenuItem>) -> Self {
        Self {
            title: title.into(),
            help: None,
            items,
            cursor: 0,
        }
    }
}

/// The whole menu, as a stack of frames.
#[derive(Debug, Clone)]
pub struct MenuState {
    frames: Vec<Frame>,
    /// Index of the first visible row, updated as the cursor moves.
    offset: usize,
}

impl MenuState {
    pub fn new(root: Frame) -> Self {
        Self {
            frames: vec![root],
            offset: 0,
        }
    }

    pub fn frame(&self) -> &Frame {
        self.frames.last().expect("the root frame is never popped")
    }

    fn frame_mut(&mut self) -> &mut Frame {
        self.frames
            .last_mut()
            .expect("the root frame is never popped")
    }

    pub fn items(&self) -> &[MenuItem] {
        &self.frame().items
    }

    pub fn cursor(&self) -> usize {
        self.frame().cursor
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Depth of the stack; 1 means the root menu.
    pub fn depth(&self) -> usize {
        self.frames.len()
    }

    pub fn selected(&self) -> Option<&MenuItem> {
        self.items().get(self.cursor())
    }

    pub fn is_empty(&self) -> bool {
        self.items().is_empty()
    }

    pub fn move_down(&mut self) {
        let len = self.items().len();
        if len == 0 {
            return;
        }
        let cursor = self.cursor();
        if cursor + 1 < len {
            self.frame_mut().cursor = cursor + 1;
        }
    }

    pub fn move_up(&mut self) {
        let cursor = self.cursor();
        if cursor > 0 {
            self.frame_mut().cursor = cursor - 1;
        }
    }

    pub fn move_first(&mut self) {
        self.frame_mut().cursor = 0;
    }

    pub fn move_last(&mut self) {
        let len = self.items().len();
        if len > 0 {
            self.frame_mut().cursor = len - 1;
        }
    }

    /// Open the detail view of the selected entry. Returns `false` when the
    /// entry has no detail to show.
    ///
    /// The view holds the entry's help text, its own command (so it can still
    /// be run from in here) and its submenu. Building it this way is what lets
    /// an entry with only `help` be opened at all — with just the submenu, it
    /// would show the `>` marker and then do nothing when opened.
    pub fn enter_detail(&mut self) -> bool {
        let Some(item) = self.selected() else {
            return false;
        };
        if !item.has_detail() {
            return false;
        }

        let mut items = Vec::with_capacity(item.submenu.len() + 1);
        if item.shell.is_some() {
            items.push(MenuItem {
                title: Some(format!("Run: {}", item.label())),
                shell: item.shell.clone(),
                args: item.args.clone(),
                ..Default::default()
            });
        }
        items.extend(item.submenu.iter().cloned());

        let frame = Frame {
            title: item.label(),
            help: item.help.clone(),
            items,
            cursor: 0,
        };
        self.frames.push(frame);
        self.offset = 0;
        true
    }

    /// Go back to the parent level. Returns `false` at the root.
    pub fn leave_detail(&mut self) -> bool {
        if self.frames.len() <= 1 {
            return false;
        }
        self.frames.pop();
        self.offset = 0;
        true
    }

    /// Scroll so that the cursor is visible in a window of `height` rows.
    ///
    /// Called after every cursor move; `height` can change at any time because
    /// the terminal can be resized while the menu is open.
    pub fn scroll_into_view(&mut self, height: usize) {
        if height == 0 {
            self.offset = 0;
            return;
        }
        let cursor = self.cursor();
        if cursor < self.offset {
            self.offset = cursor;
        } else if cursor >= self.offset + height {
            self.offset = cursor + 1 - height;
        }

        // A shrinking list (or a growing window) can leave a gap at the
        // bottom; pull the window back so the last row stays in use.
        let len = self.items().len();
        let max_offset = len.saturating_sub(height);
        if self.offset > max_offset {
            self.offset = max_offset;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(n: usize) -> Vec<MenuItem> {
        (0..n)
            .map(|i| MenuItem::command(format!("item {i}"), format!("echo {i}")))
            .collect()
    }

    fn arg(name: &str) -> crate::config::ArgSpec {
        crate::config::ArgSpec {
            name: name.to_string(),
            prompt: None,
            default: None,
        }
    }

    fn state(n: usize) -> MenuState {
        MenuState::new(Frame::new("root", items(n)))
    }

    #[test]
    fn moves_within_bounds() {
        let mut s = state(3);
        assert_eq!(s.cursor(), 0);
        s.move_up();
        assert_eq!(s.cursor(), 0, "must not move above the first entry");
        s.move_down();
        s.move_down();
        assert_eq!(s.cursor(), 2);
        s.move_down();
        assert_eq!(s.cursor(), 2, "must not move past the last entry");
    }

    #[test]
    fn jumps_to_first_and_last() {
        let mut s = state(5);
        s.move_last();
        assert_eq!(s.cursor(), 4);
        s.move_first();
        assert_eq!(s.cursor(), 0);
    }

    #[test]
    fn handles_an_empty_menu() {
        let mut s = state(0);
        s.move_down();
        s.move_last();
        assert_eq!(s.cursor(), 0);
        assert!(s.selected().is_none());
        assert!(s.is_empty());
    }

    #[test]
    fn opens_and_leaves_a_submenu_keeping_the_parent_cursor() {
        let mut parent = MenuItem::command("parent", "true");
        parent.submenu = items(2);
        let mut s = MenuState::new(Frame::new(
            "root",
            vec![MenuItem::command("a", "true"), parent],
        ));

        s.move_down();
        assert_eq!(s.cursor(), 1);
        assert!(s.enter_detail());
        assert_eq!(s.depth(), 2);
        // The parent has a command of its own, so the detail view is
        // "Run: parent" plus the two submenu entries.
        assert_eq!(s.items().len(), 3);
        assert_eq!(s.items()[0].label(), "Run: parent");

        s.move_down();
        assert!(s.leave_detail());
        assert_eq!(s.depth(), 1);
        assert_eq!(s.cursor(), 1, "the parent cursor is restored");
    }

    #[test]
    fn refuses_to_open_an_entry_with_no_detail() {
        let mut s = state(1);
        assert!(!s.enter_detail());
        assert_eq!(s.depth(), 1);
    }

    #[test]
    fn opens_an_entry_that_only_has_help() {
        let mut item = MenuItem::command("a", "true");
        item.help = Some("what this does".into());
        let mut s = MenuState::new(Frame::new("root", vec![item]));

        assert!(s.enter_detail());
        assert_eq!(s.frame().help.as_deref(), Some("what this does"));
        assert_eq!(s.items().len(), 1, "the entry's own command stays runnable");
    }

    #[test]
    fn does_not_open_an_entry_that_only_has_arguments() {
        // Arguments are prompted for when the entry runs, so there is nothing
        // for a detail view to add.
        let mut item = MenuItem::command("search", "rg {pattern}");
        item.args = vec![arg("pattern")];
        let mut s = MenuState::new(Frame::new("root", vec![item]));

        assert!(!s.enter_detail());
        assert_eq!(s.depth(), 1);
    }

    #[test]
    fn the_run_line_of_a_detail_view_carries_the_arguments_but_cannot_recurse() {
        let mut item = MenuItem::command("search", "rg {pattern}");
        item.help = Some("Search the repository.".into());
        item.args = vec![arg("pattern")];
        let mut s = MenuState::new(Frame::new("root", vec![item]));

        assert!(s.enter_detail());
        let run = &s.items()[0];
        assert_eq!(run.args.len(), 1, "the arguments are carried over");
        assert!(!run.has_detail(), "otherwise the view would nest forever");
    }

    #[test]
    fn refuses_to_leave_the_root() {
        let mut s = state(1);
        assert!(!s.leave_detail());
        assert_eq!(s.depth(), 1);
    }

    #[test]
    fn scrolls_the_window_to_follow_the_cursor() {
        let mut s = state(10);
        s.scroll_into_view(3);
        assert_eq!(s.offset(), 0);

        for _ in 0..3 {
            s.move_down();
            s.scroll_into_view(3);
        }
        assert_eq!(s.cursor(), 3);
        assert_eq!(s.offset(), 1, "the window follows the cursor downwards");

        s.move_first();
        s.scroll_into_view(3);
        assert_eq!(s.offset(), 0, "the window follows the cursor upwards");
    }

    #[test]
    fn pulls_the_window_back_when_it_grows_past_the_end() {
        let mut s = state(10);
        s.move_last();
        s.scroll_into_view(3);
        assert_eq!(s.offset(), 7);

        // The terminal was made taller: the whole list now fits.
        s.scroll_into_view(20);
        assert_eq!(s.offset(), 0);
    }

    #[test]
    fn tolerates_a_zero_height_window() {
        let mut s = state(10);
        s.move_last();
        s.scroll_into_view(0);
        assert_eq!(s.offset(), 0);
    }
}
