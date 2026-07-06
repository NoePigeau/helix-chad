use crate::compositor::{Component, Context, Event, EventResult};
use crate::ui::document::{LinePos, TextRenderer};
use crate::ui::text_decorations::Decoration;
use crate::ui::Prompt;

use helix_core::doc_formatter::FormattedGrapheme;
use helix_core::text_annotations::LineAnnotation;
use helix_core::{unicode::width::UnicodeWidthStr, Position};
use helix_view::editor::{CursorCache, Rename};
use helix_view::graphics::{CursorKind, Rect, Style};
use helix_view::{Editor, Theme};

const MIN_FIELD_WIDTH: u16 = 8;
const FIELD_PADDING: u16 = 1;

pub struct RenamePrompt {
    prompt: Prompt,
}

impl RenamePrompt {
    pub const ID: &'static str = "rename";

    pub fn new(prompt: Prompt) -> Self {
        Self { prompt }
    }

    fn sync_input(&self, editor: &mut Editor) {
        if let Some(rename) = &mut editor.rename {
            rename.input = self.prompt.line().clone();
            rename.cursor = self.prompt.position();
        }
    }
}

impl Component for RenamePrompt {
    fn handle_event(&mut self, event: &Event, cx: &mut Context) -> EventResult {
        let result = self.prompt.handle_event(event, cx);
        self.sync_input(cx.editor);
        result
    }

    // The input field is drawn inline over the buffer by RenameDecoration (see editor.rs).
    fn render(&mut self, _area: Rect, _surface: &mut tui::buffer::Buffer, _cx: &mut Context) {}

    fn cursor(&self, _area: Rect, editor: &Editor) -> (Option<Position>, CursorKind) {
        (editor.cursor().0, CursorKind::Bar)
    }

    fn id(&self) -> Option<&'static str> {
        Some(Self::ID)
    }
}

pub struct RenameLineAnnotation {
    anchor_line: usize,
}

impl RenameLineAnnotation {
    pub fn new(anchor_line: usize) -> Self {
        Self { anchor_line }
    }
}

impl LineAnnotation for RenameLineAnnotation {
    fn insert_virtual_lines(
        &mut self,
        _line_end_char_idx: usize,
        _line_end_visual_pos: Position,
        doc_line: usize,
    ) -> Position {
        if doc_line == self.anchor_line {
            Position::new(1, 0)
        } else {
            Position::new(0, 0)
        }
    }
}

pub struct RenameDecoration<'a> {
    rename: &'a Rename,
    cache: &'a CursorCache,
    field_style: Style,
    symbol_col: u16,
}

impl<'a> RenameDecoration<'a> {
    pub fn new(rename: &'a Rename, cache: &'a CursorCache, theme: &Theme) -> Self {
        Self {
            rename,
            cache,
            field_style: theme.get("ui.menu"),
            symbol_col: 0,
        }
    }

    fn field_width(&self, available: u16) -> u16 {
        (self.rename.input.width() as u16 + FIELD_PADDING)
            .max(MIN_FIELD_WIDTH)
            .min(available)
    }
}

impl Decoration for RenameDecoration<'_> {
    fn reset_pos(&mut self, pos: usize) -> usize {
        if pos <= self.rename.symbol_char_idx {
            self.rename.symbol_char_idx
        } else {
            usize::MAX
        }
    }

    fn decorate_grapheme(
        &mut self,
        _renderer: &mut TextRenderer,
        grapheme: &FormattedGrapheme,
    ) -> usize {
        if grapheme.char_idx == self.rename.symbol_char_idx {
            self.symbol_col = grapheme.visual_pos.col as u16;
        }
        usize::MAX
    }

    fn render_virt_lines(
        &mut self,
        renderer: &mut TextRenderer,
        pos: LinePos,
        virt_off: Position,
    ) -> Position {
        if pos.doc_line != self.rename.anchor_line {
            return Position::new(0, 0);
        }

        let screen_col = self.symbol_col.saturating_sub(renderer.offset.col as u16);
        if screen_col >= renderer.viewport.width {
            return Position::new(1, 0);
        }

        let row = pos.visual_line + virt_off.row as u16;
        let x = renderer.viewport.x + screen_col;
        let available = renderer.viewport.width - screen_col;
        let field_width = self.field_width(available);

        renderer.set_style(Rect::new(x, row, field_width, 1), self.field_style);
        renderer.set_stringn(
            x,
            row,
            &self.rename.input,
            field_width as usize,
            self.field_style,
        );

        let caret_col = screen_col + self.rename.input[..self.rename.cursor].width() as u16;
        let cache_row = (row as usize).saturating_sub(renderer.offset.row);
        self.cache
            .set(Some(Position::new(cache_row, caret_col as usize)));

        Position::new(1, 0)
    }
}
