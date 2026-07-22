use std::path::PathBuf;

use helix_core::Rope;
use helix_vcs::{diff_hunks, Hunk};

use crate::graphics::Style;

/// Number of unchanged context lines kept around each change.
const CONTEXT: usize = 5;

/// Per-line syntax highlight spans: for each line, a list of
/// `(start_col, end_col, style)` char ranges.
pub type LineHighlights = Vec<Vec<(usize, usize, Style)>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffRowKind {
    Unchanged,
    Deleted,
    Added,
    Modified,
    Separator,
}

#[derive(Debug, Clone, Copy)]
pub struct DiffRow {
    pub left: Option<usize>,
    pub right: Option<usize>,
    pub kind: DiffRowKind,
}

/// A read-only, side-by-side comparison between the committed version of a file
/// (`base`) and its working-tree version (`doc`), rendered as its own buffer.
#[derive(Debug)]
pub struct DiffView {
    pub title: String,
    pub path: PathBuf,
    pub base: Rope,
    pub doc: Rope,
    pub rows: Vec<DiffRow>,
    pub scroll: usize,
    pub base_highlights: LineHighlights,
    pub doc_highlights: LineHighlights,
}

impl DiffView {
    pub fn new(title: String, path: PathBuf, base: Rope, doc: Rope) -> Self {
        let hunks = diff_hunks(base.clone(), doc.clone());
        let rows = collapse_context(build_rows(
            content_lines(&base),
            content_lines(&doc),
            &hunks,
        ));

        Self {
            title,
            path,
            base,
            doc,
            rows,
            scroll: 0,
            base_highlights: Vec::new(),
            doc_highlights: Vec::new(),
        }
    }

    pub fn max_scroll(&self, height: usize) -> usize {
        self.rows.len().saturating_sub(height)
    }

    pub fn scroll_down(&mut self, amount: usize, height: usize) {
        self.scroll = (self.scroll + amount).min(self.max_scroll(height));
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll = self.scroll.saturating_sub(amount);
    }
}

fn build_rows(base_lines: usize, doc_lines: usize, hunks: &[Hunk]) -> Vec<DiffRow> {
    let mut rows = Vec::new();
    let mut base = 0;
    let mut doc = 0;

    for hunk in hunks {
        let deleted = hunk.before.start as usize..hunk.before.end as usize;
        let added = hunk.after.start as usize..hunk.after.end as usize;

        while base < deleted.start {
            rows.push(unchanged(base, doc));
            base += 1;
            doc += 1;
        }

        let span = deleted.len().max(added.len());
        for k in 0..span {
            let left = (k < deleted.len()).then_some(deleted.start + k);
            let right = (k < added.len()).then_some(added.start + k);
            rows.push(DiffRow {
                left,
                right,
                kind: change_kind(left.is_some(), right.is_some()),
            });
        }

        base = deleted.end;
        doc = added.end;
    }

    while base < base_lines && doc < doc_lines {
        rows.push(unchanged(base, doc));
        base += 1;
        doc += 1;
    }

    rows
}

fn unchanged(base: usize, doc: usize) -> DiffRow {
    DiffRow {
        left: Some(base),
        right: Some(doc),
        kind: DiffRowKind::Unchanged,
    }
}

/// Drops unchanged lines that are more than `CONTEXT` rows away from a change,
/// replacing each collapsed gap with a single separator row.
fn collapse_context(rows: Vec<DiffRow>) -> Vec<DiffRow> {
    let changed: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.kind != DiffRowKind::Unchanged)
        .map(|(index, _)| index)
        .collect();

    if changed.is_empty() {
        return Vec::new();
    }

    let mut kept = vec![false; rows.len()];
    for index in changed {
        let start = index.saturating_sub(CONTEXT);
        let end = (index + CONTEXT + 1).min(rows.len());
        kept[start..end].iter_mut().for_each(|keep| *keep = true);
    }

    let mut out = Vec::new();
    let mut gap = false;

    for (index, row) in rows.into_iter().enumerate() {
        if kept[index] {
            if gap {
                out.push(separator());
                gap = false;
            }
            out.push(row);
        } else {
            gap = true;
        }
    }

    if gap {
        out.push(separator());
    }

    out
}

fn separator() -> DiffRow {
    DiffRow {
        left: None,
        right: None,
        kind: DiffRowKind::Separator,
    }
}

fn change_kind(has_left: bool, has_right: bool) -> DiffRowKind {
    match (has_left, has_right) {
        (true, true) => DiffRowKind::Modified,
        (true, false) => DiffRowKind::Deleted,
        _ => DiffRowKind::Added,
    }
}

fn content_lines(rope: &Rope) -> usize {
    let lines = rope.len_lines();
    match rope.get_line(lines.saturating_sub(1)) {
        Some(last) if last.len_chars() == 0 => lines.saturating_sub(1),
        _ => lines,
    }
}
