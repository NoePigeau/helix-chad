use std::path::Path;

use helix_core::{
    syntax::{self, HighlightEvent},
    Rope, RopeSlice, Syntax,
};
use helix_stdx::rope::RopeSliceExt;
use helix_view::{
    diff_view::{DiffRowKind, DiffView, LineHighlights},
    graphics::{Color, Modifier, Rect, Style},
    theme::Theme,
};

use tui::buffer::Buffer as Surface;

const GUTTER_GAP: u16 = 1;
const DIVIDER: &str = "\u{2502}";

pub fn render(diff: &DiffView, area: Rect, surface: &mut Surface, theme: &Theme) {
    let styles = DiffStyles::from_theme(theme);
    surface.clear_with(area, styles.background);

    render_headers(area, surface, &styles);
    render_panes(diff, area, surface, &styles);
}

/// Number of diff rows visible for a window of the given total height,
/// accounting for the header row and the statusline row.
pub fn visible_rows(window_height: u16) -> usize {
    window_height.saturating_sub(2) as usize
}

/// Computes tree-sitter syntax highlights for every line of `rope`, resolving
/// each to a concrete [`Style`] via `theme`. Returns an empty list when the
/// language cannot be detected.
pub fn highlight_lines(
    rope: &Rope,
    path: &Path,
    loader: &syntax::Loader,
    theme: &Theme,
) -> LineHighlights {
    let mut lines: LineHighlights = vec![Vec::new(); rope.len_lines()];
    let slice = rope.slice(..);

    let Some(language) = loader.language_for_filename(path) else {
        return lines;
    };
    let Ok(syntax) = Syntax::new(slice, language, loader) else {
        return lines;
    };

    let mut highlighter = syntax.highlighter(slice, loader, ..);
    let mut stack = Vec::new();
    let mut pos = 0;
    let len = slice.len_bytes() as u32;

    while pos < len {
        if pos == highlighter.next_event_offset() {
            let (event, added) = highlighter.advance();
            if event == HighlightEvent::Refresh {
                stack.clear();
            }
            stack.extend(added);
        }

        let start = pos;
        pos = highlighter.next_event_offset();
        if pos == u32::MAX {
            pos = len;
        }
        if pos <= start {
            continue;
        }

        let style = stack.iter().fold(Style::default(), |acc, highlight| {
            acc.patch(theme.highlight(*highlight))
        });

        if style != Style::default() {
            push_span(&mut lines, slice, start, pos, style);
        }
    }

    lines
}

fn push_span(lines: &mut LineHighlights, slice: RopeSlice, start: u32, end: u32, style: Style) {
    let start = slice.byte_to_char(slice.ceil_char_boundary(start as usize));
    let end = slice.byte_to_char(slice.ceil_char_boundary(end as usize));

    let mut cursor = start;
    while cursor < end {
        let line = slice.char_to_line(cursor);
        let line_start = slice.line_to_char(line);
        let line_end = line_start + slice.line(line).len_chars();
        let segment_end = end.min(line_end);

        if let Some(spans) = lines.get_mut(line) {
            spans.push((cursor - line_start, segment_end - line_start, style));
        }

        cursor = segment_end.max(cursor + 1);
    }
}

fn render_headers(area: Rect, surface: &mut Surface, styles: &DiffStyles) {
    let divider_x = area.x + area.width / 2;
    surface.clear_with(Rect::new(area.x, area.y, area.width, 1), styles.header);
    surface.set_stringn(divider_x, area.y, DIVIDER, 1, styles.divider);

    surface.set_stringn(
        area.x + GUTTER_GAP,
        area.y,
        "HEAD",
        area.width as usize,
        styles.header,
    );
    surface.set_stringn(
        divider_x + 1 + GUTTER_GAP,
        area.y,
        "Working tree",
        area.width as usize,
        styles.header,
    );
}

/// One side of the diff: the pane it occupies plus the text and highlights it
/// draws. Constant across every rendered row.
struct Column<'a> {
    area: Rect,
    gutter: u16,
    rope: &'a Rope,
    highlights: &'a LineHighlights,
}

fn render_panes(diff: &DiffView, area: Rect, surface: &mut Surface, styles: &DiffStyles) {
    let inner_y = area.y + 1;
    let height = area.height.saturating_sub(1);
    let divider_x = area.x + area.width / 2;
    let gutter = line_number_width(diff.base.len_lines().max(diff.doc.len_lines()));

    let left = Column {
        area: Rect::new(area.x, inner_y, area.width / 2, height),
        gutter,
        rope: &diff.base,
        highlights: &diff.base_highlights,
    };
    let right = Column {
        area: Rect::new(
            divider_x + 1,
            inner_y,
            area.width.saturating_sub(area.width / 2 + 1),
            height,
        ),
        gutter,
        rope: &diff.doc,
        highlights: &diff.doc_highlights,
    };

    for offset in 0..height {
        let y = inner_y + offset;

        let Some(row) = diff.rows.get(diff.scroll + offset as usize) else {
            surface.set_stringn(divider_x, y, DIVIDER, 1, styles.divider);
            continue;
        };

        if row.kind == DiffRowKind::Separator {
            render_separator(left.area, right.area, y, surface, styles);
            continue;
        }

        surface.set_stringn(divider_x, y, DIVIDER, 1, styles.divider);
        render_cell(surface, &left, y, row.left, left_style(row.kind, styles));
        render_cell(surface, &right, y, row.right, right_style(row.kind, styles));
    }
}

fn render_cell(
    surface: &mut Surface,
    column: &Column,
    y: u16,
    line: Option<usize>,
    line_style: Style,
) {
    surface.clear_with(
        Rect::new(column.area.x, y, column.area.width, 1),
        line_style,
    );

    let Some(line) = line else {
        return;
    };

    let number = format!("{:>width$}", line + 1, width = column.gutter as usize);
    surface.set_stringn(
        column.area.x,
        y,
        &number,
        column.gutter as usize,
        line_style,
    );

    let mut writer = LineWriter {
        surface,
        y,
        x: column.area.x + column.gutter + GUTTER_GAP,
        end_x: column.area.x + column.area.width,
    };
    writer.render_line(column.rope, line, line_style, column.highlights.get(line));
}

/// A left-to-right cursor that paints styled runs across a single row, expanding
/// tabs and stopping at the pane edge.
struct LineWriter<'a> {
    surface: &'a mut Surface,
    y: u16,
    x: u16,
    end_x: u16,
}

impl LineWriter<'_> {
    fn render_line(
        &mut self,
        rope: &Rope,
        line: usize,
        base: Style,
        spans: Option<&Vec<(usize, usize, Style)>>,
    ) {
        let empty = Vec::new();
        let spans = spans.unwrap_or(&empty);
        let text = rope.line(line);

        let mut run = String::new();
        let mut run_style = base;
        let mut span = 0;

        for (index, ch) in text.chars().enumerate() {
            if matches!(ch, '\n' | '\r') {
                break;
            }

            let style = style_at(base, spans, index, &mut span);
            if style != run_style && !run.is_empty() {
                self.write(&run, run_style);
                run.clear();
                if self.x >= self.end_x {
                    return;
                }
            }
            run_style = style;

            if ch == '\t' {
                run.push_str("    ");
            } else {
                run.push(ch);
            }
        }

        if !run.is_empty() {
            self.write(&run, run_style);
        }
    }

    fn write(&mut self, text: &str, style: Style) {
        if self.x >= self.end_x {
            return;
        }

        let (next_x, _) =
            self.surface
                .set_stringn(self.x, self.y, text, (self.end_x - self.x) as usize, style);
        self.x = next_x;
    }
}

/// Resolves the style for a character, advancing `span` through the ordered,
/// non-overlapping spans so the whole line stays O(n).
fn style_at(base: Style, spans: &[(usize, usize, Style)], index: usize, span: &mut usize) -> Style {
    while *span < spans.len() && index >= spans[*span].1 {
        *span += 1;
    }

    match spans.get(*span) {
        Some((start, _, style)) if index >= *start => merge(base, *style),
        _ => base,
    }
}

/// Syntax foreground over the diff line background: takes the syntax fg and
/// modifiers but keeps the row's background tint.
fn merge(base: Style, syntax: Style) -> Style {
    let mut merged = base.patch(syntax);
    merged.bg = base.bg;
    merged
}

fn render_separator(left: Rect, right: Rect, y: u16, surface: &mut Surface, styles: &DiffStyles) {
    let dash = |width: u16| "\u{2504}".repeat(width as usize);
    surface.set_stringn(
        left.x,
        y,
        &dash(left.width),
        left.width as usize,
        styles.separator,
    );
    surface.set_stringn(
        right.x,
        y,
        &dash(right.width),
        right.width as usize,
        styles.separator,
    );
    surface.set_stringn(left.x + left.width, y, "\u{253c}", 1, styles.separator);
}

struct DiffStyles {
    background: Style,
    header: Style,
    context: Style,
    deleted: Style,
    added: Style,
    filler: Style,
    divider: Style,
    separator: Style,
}

/// How much of the diff colour is mixed into the background for the default
/// line tint. Low so it stays subtle and adapts to light and dark themes.
const TINT_ALPHA: f32 = 0.22;

impl DiffStyles {
    fn from_theme(theme: &Theme) -> Self {
        let background = theme.get("ui.background");
        let context = theme.get("ui.text");
        let divider_fg = theme
            .try_get("ui.linenr")
            .and_then(|style| style.fg)
            .unwrap_or(Color::Gray);

        Self {
            background,
            header: background.patch(context).add_modifier(Modifier::BOLD),
            context,
            deleted: line_tint(
                theme,
                context,
                "ui.diff.deleted",
                "diff.minus",
                Color::Rgb(0xF0, 0xCF, 0xCF),
            ),
            added: line_tint(
                theme,
                context,
                "ui.diff.added",
                "diff.plus",
                Color::Rgb(0xCF, 0xEB, 0xD4),
            ),
            filler: filler_style(theme, context),
            divider: Style::default().fg(divider_fg),
            separator: Style::default().fg(divider_fg),
        }
    }
}

/// The whole-line style for an added/deleted row.
///
/// Prefers the dedicated theme scope (`ui.diff.added` / `ui.diff.deleted`) so
/// it can be customised. Otherwise derives a subtle tint by blending the
/// theme's diff colour into the background, which adapts to light and dark
/// themes automatically.
fn line_tint(
    theme: &Theme,
    context: Style,
    override_key: &str,
    diff_key: &str,
    fallback: Color,
) -> Style {
    if let Some(style) = theme.try_get(override_key) {
        return context.patch(style);
    }

    let bg = blend_into_background(theme, diff_key, TINT_ALPHA).unwrap_or(fallback);
    context.bg(bg)
}

fn filler_style(theme: &Theme, context: Style) -> Style {
    match blend_into_background(theme, "ui.linenr", 0.08) {
        Some(bg) => Style::default().bg(bg),
        None => context,
    }
}

/// Blends the foreground colour of `key` into `ui.background` by `alpha`.
/// Returns `None` unless both colours are concrete RGB values.
fn blend_into_background(theme: &Theme, key: &str, alpha: f32) -> Option<Color> {
    let foreground = as_rgb(theme.try_get(key)?.fg?)?;
    let background = as_rgb(theme.get("ui.background").bg?)?;
    Some(blend(foreground, background, alpha))
}

fn as_rgb(color: Color) -> Option<(u8, u8, u8)> {
    match color {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        _ => None,
    }
}

fn blend(fg: (u8, u8, u8), bg: (u8, u8, u8), alpha: f32) -> Color {
    let mix = |fg: u8, bg: u8| (fg as f32 * alpha + bg as f32 * (1.0 - alpha)).round() as u8;
    Color::Rgb(mix(fg.0, bg.0), mix(fg.1, bg.1), mix(fg.2, bg.2))
}

fn left_style(kind: DiffRowKind, styles: &DiffStyles) -> Style {
    match kind {
        DiffRowKind::Deleted | DiffRowKind::Modified => styles.deleted,
        DiffRowKind::Added => styles.filler,
        _ => styles.context,
    }
}

fn right_style(kind: DiffRowKind, styles: &DiffStyles) -> Style {
    match kind {
        DiffRowKind::Added | DiffRowKind::Modified => styles.added,
        DiffRowKind::Deleted => styles.filler,
        _ => styles.context,
    }
}

fn line_number_width(lines: usize) -> u16 {
    let digits = lines.max(1).to_string().len() as u16;
    digits.max(2)
}
