use helix_core::Position;
use helix_view::graphics::Color;
use helix_view::theme::Style;
use helix_view::Theme;

use crate::ui::document::{LinePos, TextRenderer};
use crate::ui::text_decorations::Decoration;

const INLINE_BLAME_GAP: usize = 6;
const INLINE_BLAME_COLOR: Color = Color::Rgb(0x7A, 0x81, 0x8A);

pub struct InlineBlame {
    text: String,
    style: Style,
    anchor_line: usize,
}

impl InlineBlame {
    pub fn new(theme: &Theme, anchor_line: usize, text: String) -> Self {
        InlineBlame {
            style: theme
                .try_get_exact("ui.virtual.inline-blame")
                .unwrap_or_else(|| Style::default().fg(INLINE_BLAME_COLOR)),
            text,
            anchor_line,
        }
    }
}

impl Decoration for InlineBlame {
    fn render_virt_lines(
        &mut self,
        renderer: &mut TextRenderer,
        pos: LinePos,
        virt_off: Position,
    ) -> Position {
        if pos.doc_line != self.anchor_line {
            return Position::new(0, 0);
        }

        let draw_col = virt_off.col + INLINE_BLAME_GAP;
        if !renderer.column_in_bounds(draw_col, 1) {
            return Position::new(0, 0);
        }

        let width = renderer.viewport.width;
        let (end_col, _) = renderer.set_string_truncated(
            renderer.viewport.x + draw_col as u16,
            pos.visual_line,
            &self.text,
            width.saturating_sub(draw_col as u16) as usize,
            |_| self.style,
            true,
            false,
        );

        let start_col = virt_off.col.saturating_sub(renderer.offset.col);
        let drawn_width = (end_col - renderer.viewport.x) as usize - start_col;
        Position::new(0, drawn_width)
    }
}
