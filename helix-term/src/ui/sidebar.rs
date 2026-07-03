use std::path::Path;

use helix_vcs::FileChange;
use helix_view::{
    graphics::{Color, Rect, Style},
    theme::Theme,
    Editor,
};

use tui::buffer::Buffer as Surface;

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

#[derive(Debug)]
pub struct SidebarState {
    open: bool,
    focused: bool,
    width: u16,
    pub selected: usize,
    pub scroll: usize,
}

impl Default for SidebarState {
    fn default() -> Self {
        Self {
            open: false,
            focused: false,
            width: DEFAULT_WIDTH,
            selected: 0,
            scroll: 0,
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

    pub fn focus(&mut self) {
        if self.open {
            self.focused = true;
        }
    }

    pub fn close(&mut self) {
        self.open = false;
        self.focused = false;
    }

    pub fn unfocus(&mut self) {
        self.focused = false;
    }

    pub fn move_selection(&mut self, delta: isize, len: usize) {
        if len == 0 {
            return;
        }

        let last = len - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last as isize) as usize;
    }

    pub fn clamp_selection(&mut self, len: usize) {
        if self.selected >= len {
            self.selected = len.saturating_sub(1);
        }
    }

    pub fn adjust_scroll(&mut self, height: usize) {
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
