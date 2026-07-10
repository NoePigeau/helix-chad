use std::ops::Range;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default)]
pub struct FileBlame {
    hunks: Vec<BlameHunk>,
}

impl FileBlame {
    pub(crate) fn new(mut hunks: Vec<BlameHunk>) -> Self {
        hunks.sort_by_key(|hunk| hunk.lines.start);
        Self { hunks }
    }

    pub fn blame_for_line(&self, line: u32) -> Option<&LineBlame> {
        let index = self
            .hunks
            .partition_point(|hunk| hunk.lines.start <= line)
            .checked_sub(1)?;
        let hunk = &self.hunks[index];
        hunk.lines.contains(&line).then(|| &*hunk.blame)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BlameHunk {
    lines: Range<u32>,
    blame: Arc<LineBlame>,
}

impl BlameHunk {
    pub(crate) fn new(lines: Range<u32>, blame: Arc<LineBlame>) -> Self {
        Self { lines, blame }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineBlame {
    pub commit: String,
    pub author: String,
    pub timestamp: i64,
    pub message: String,
}

impl LineBlame {
    pub fn format(&self, format: &str) -> String {
        format
            .replace("{author}", &self.author)
            .replace("{time-ago}", &time_ago(self.timestamp))
            .replace("{message}", &self.message)
            .replace("{commit}", self.short_commit())
    }

    pub fn short_commit(&self) -> &str {
        &self.commit[..self.commit.len().min(8)]
    }
}

const UNITS: &[(u64, &str)] = &[
    (365 * 24 * 3600, "year"),
    (30 * 24 * 3600, "month"),
    (7 * 24 * 3600, "week"),
    (24 * 3600, "day"),
    (3600, "hour"),
    (60, "minute"),
];

fn time_ago(timestamp: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default();
    let elapsed_seconds = now.saturating_sub(timestamp).max(0) as u64;

    for &(unit_seconds, unit) in UNITS {
        let amount = elapsed_seconds / unit_seconds;
        if amount > 0 {
            let plural = if amount > 1 { "s" } else { "" };
            return format!("{amount} {unit}{plural} ago");
        }
    }
    "just now".to_owned()
}

#[cfg(test)]
mod test {
    use super::*;

    fn line_blame(commit: &str) -> Arc<LineBlame> {
        Arc::new(LineBlame {
            commit: commit.to_owned(),
            author: "Jane Doe".to_owned(),
            timestamp: 0,
            message: "initial commit".to_owned(),
        })
    }

    #[test]
    fn blame_for_line_finds_containing_hunk() {
        let blame = FileBlame::new(vec![
            BlameHunk::new(0..2, line_blame("aaaa")),
            BlameHunk::new(5..8, line_blame("bbbb")),
        ]);

        assert_eq!(blame.blame_for_line(0).unwrap().commit, "aaaa");
        assert_eq!(blame.blame_for_line(1).unwrap().commit, "aaaa");
        assert_eq!(blame.blame_for_line(2), None);
        assert_eq!(blame.blame_for_line(5).unwrap().commit, "bbbb");
        assert_eq!(blame.blame_for_line(7).unwrap().commit, "bbbb");
        assert_eq!(blame.blame_for_line(8), None);
    }

    #[test]
    fn format_replaces_placeholders() {
        let blame = line_blame("abcd1234");
        assert_eq!(
            blame.format("{author} • {message} • {commit}"),
            "Jane Doe • initial commit • abcd1234"
        );
    }
}
