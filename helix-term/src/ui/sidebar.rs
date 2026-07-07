use std::path::Path;

use helix_vcs::FileChange;
use helix_view::{
    graphics::{Color, Rect, Style},
    input::KeyEvent,
    theme::Theme,
    Editor,
};

use tui::buffer::Buffer as Surface;

use crate::keymap::ReverseKeymap;

pub const DEFAULT_WIDTH: u16 = 30;
pub const MIN_WIDTH: u16 = 15;
pub const MAX_WIDTH: u16 = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitStatus {
    Added,
    Modified,
    Deleted,
}

impl GitStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "Added",
            Self::Modified => "Modified",
            Self::Deleted => "Deleted",
        }
    }

    pub fn rank(self) -> u8 {
        match self {
            Self::Added => 1,
            Self::Deleted => 2,
            Self::Modified => 3,
        }
    }

    pub fn from_change(change: &FileChange) -> Self {
        match change {
            FileChange::Untracked { .. } => Self::Added,
            FileChange::Modified { .. }
            | FileChange::Conflict { .. }
            | FileChange::Renamed { .. } => Self::Modified,
            FileChange::Deleted { .. } => Self::Deleted,
        }
    }

    pub fn style(self, theme: &Theme) -> Style {
        let (key, fallback) = match self {
            Self::Added => ("version_control.added", Color::Rgb(0x27, 0xA6, 0x57)),
            Self::Modified => ("version_control.modified", Color::Rgb(0xD3, 0xB0, 0x20)),
            Self::Deleted => ("version_control.deleted", Color::Rgb(0xE0, 0x6C, 0x76)),
        };

        theme
            .try_get(key)
            .unwrap_or_else(|| Style::default().fg(fallback))
    }
}

#[derive(Debug, Clone, Copy)]
enum NavAction {
    First,
    Last,
    HalfDown,
    HalfUp,
}

#[derive(Debug, Default, Clone)]
pub struct NavKeys {
    first: Vec<Vec<KeyEvent>>,
    last: Vec<Vec<KeyEvent>>,
    down: Vec<Vec<KeyEvent>>,
    up: Vec<Vec<KeyEvent>>,
}

impl NavKeys {
    pub fn from_reverse(reverse: &ReverseKeymap) -> Self {
        let bindings = |command: &str| reverse.get(command).cloned().unwrap_or_default();

        Self {
            first: bindings("goto_file_start"),
            last: bindings("goto_last_line"),
            down: bindings("page_cursor_half_down"),
            up: bindings("page_cursor_half_up"),
        }
    }

    fn actions(&self) -> [(&Vec<Vec<KeyEvent>>, NavAction); 4] {
        [
            (&self.first, NavAction::First),
            (&self.last, NavAction::Last),
            (&self.down, NavAction::HalfDown),
            (&self.up, NavAction::HalfUp),
        ]
    }

    fn action(&self, sequence: &[KeyEvent]) -> Option<NavAction> {
        self.actions().into_iter().find_map(|(bindings, action)| {
            bindings
                .iter()
                .any(|binding| binding.as_slice() == sequence)
                .then_some(action)
        })
    }

    fn is_prefix(&self, sequence: &[KeyEvent]) -> bool {
        self.actions()
            .into_iter()
            .flat_map(|(bindings, _)| bindings)
            .any(|binding| binding.len() > sequence.len() && binding.starts_with(sequence))
    }
}

#[derive(Debug)]
pub struct SidebarState {
    open: bool,
    focused: bool,
    width: u16,
    pub selected: usize,
    pub scroll: usize,
    viewport: usize,
    pending: Vec<KeyEvent>,
}

impl Default for SidebarState {
    fn default() -> Self {
        Self {
            open: false,
            focused: false,
            width: DEFAULT_WIDTH,
            selected: 0,
            scroll: 0,
            viewport: 0,
            pending: Vec::new(),
        }
    }
}

impl SidebarState {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_focused(&self) -> bool {
        self.open && self.focused
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn set_width(&mut self, width: u16) {
        self.width = width.clamp(MIN_WIDTH, MAX_WIDTH);
    }

    pub fn open_focused(&mut self) {
        self.open = true;
        self.focused = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.focused = false;
    }

    pub fn unfocus(&mut self) {
        self.focused = false;
        self.pending.clear();
    }

    pub fn move_selection(&mut self, delta: isize, len: usize) {
        if len == 0 {
            return;
        }

        let last = len - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last as isize) as usize;
    }

    pub fn handle_nav(&mut self, event: KeyEvent, keys: &NavKeys, len: usize) -> bool {
        let mut sequence = std::mem::take(&mut self.pending);
        sequence.push(event);

        if let Some(action) = keys.action(&sequence) {
            self.apply_nav(action, len);
            return true;
        }

        if keys.is_prefix(&sequence) {
            self.pending = sequence;
            return true;
        }

        false
    }

    fn apply_nav(&mut self, action: NavAction, len: usize) {
        match action {
            NavAction::First => self.selected = 0,
            NavAction::Last => self.selected = len.saturating_sub(1),
            NavAction::HalfDown => self.move_selection(self.half_page(), len),
            NavAction::HalfUp => self.move_selection(-self.half_page(), len),
        }
    }

    fn half_page(&self) -> isize {
        (self.viewport / 2).max(1) as isize
    }

    pub fn clamp_selection(&mut self, len: usize) {
        if self.selected >= len {
            self.selected = len.saturating_sub(1);
        }
    }

    pub fn adjust_scroll(&mut self, height: usize) {
        self.viewport = height;
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if height > 0 && self.selected >= self.scroll + height {
            self.scroll = self.selected - height + 1;
        }
    }
}

pub fn draw_separator(area: Rect, surface: &mut Surface, theme: &Theme) {
    let style = theme.get("ui.window");
    let x = area.right().saturating_sub(1);

    for y in area.top()..area.bottom() {
        surface[(x, y)]
            .set_symbol(tui::symbols::line::VERTICAL)
            .set_style(style);
    }
}

pub fn is_git_trusted(editor: &Editor, workspace: &Path) -> bool {
    use helix_loader::workspace_trust::TrustQuery;

    editor
        .workspace_trust
        .query(workspace, TrustQuery::Git)
        .is_trusted()
}
