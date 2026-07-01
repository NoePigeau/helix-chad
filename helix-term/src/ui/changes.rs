use std::io;
use std::path::{Path, PathBuf, MAIN_SEPARATOR};
use std::process::{Command, ExitStatus};
use std::time::Duration;

use helix_vcs::FileChange;
use helix_view::{
    current,
    editor::Action,
    graphics::{Color, Modifier, Rect, Style},
    input::KeyEvent,
    keyboard::{KeyCode, KeyModifiers},
    DocumentId, Editor,
};

use tui::buffer::Buffer as Surface;

use crate::compositor::{Callback, Compositor, Context, EventResult};
use crate::job::{self, Job, Jobs};
use crate::ui::{completers, icons, EditorView, Prompt, PromptEvent};

const GOTO_CHANGE_RETRY_DELAY: Duration = Duration::from_millis(16);
const GOTO_CHANGE_MAX_ATTEMPTS: usize = 60;

fn schedule_goto_first_change(jobs: &mut Jobs, doc_id: DocumentId) {
    jobs.add(goto_first_change_job(doc_id, GOTO_CHANGE_MAX_ATTEMPTS));
}

fn goto_first_change_job(doc_id: DocumentId, attempts: usize) -> Job {
    Job::with_callback(async move {
        tokio::time::sleep(GOTO_CHANGE_RETRY_DELAY).await;
        Ok(job::Callback::Followup(Box::new(move |editor| {
            if crate::commands::goto_first_change_in_focused_doc(editor, doc_id) || attempts == 0 {
                None
            } else {
                Some(goto_first_change_job(doc_id, attempts - 1))
            }
        })))
    })
}

const DEFAULT_WIDTH: u16 = 30;
const MIN_WIDTH: u16 = 15;
const MAX_WIDTH: u16 = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChangeStatus {
    Added,
    Modified,
    Deleted,
}

impl ChangeStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Added => "Added",
            Self::Modified => "Modified",
            Self::Deleted => "Deleted",
        }
    }
}

#[derive(Debug)]
struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    status: ChangeStatus,
    expanded: bool,
    children: Vec<Entry>,
}

#[derive(Debug)]
struct Group {
    status: ChangeStatus,
    staged: bool,
    expanded: bool,
    count: usize,
    roots: Vec<Entry>,
}

#[derive(Debug, Clone, Copy)]
enum RowKind {
    Group,
    Dir,
    File,
}

#[derive(Debug)]
struct Row {
    depth: usize,
    label: String,
    kind: RowKind,
    status: ChangeStatus,
    staged: bool,
    path: PathBuf,
    expanded: bool,
}

#[derive(Debug)]
pub struct ChangesSidebar {
    open: bool,
    focused: bool,
    width: u16,
    root: PathBuf,
    groups: Vec<Group>,
    rows: Vec<Row>,
    selected: usize,
    scroll: usize,
}

impl Default for ChangesSidebar {
    fn default() -> Self {
        Self {
            open: false,
            focused: false,
            width: DEFAULT_WIDTH,
            root: PathBuf::new(),
            groups: Vec::new(),
            rows: Vec::new(),
            selected: 0,
            scroll: 0,
        }
    }
}

impl ChangesSidebar {
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

    pub fn close(&mut self) {
        self.open = false;
        self.focused = false;
    }

    pub fn unfocus(&mut self) {
        self.focused = false;
    }

    pub fn focus_panel(&mut self) {
        if self.open {
            self.focused = true;
        }
    }

    pub fn toggle(&mut self, editor: &Editor) {
        if self.open {
            self.close();
        } else {
            self.refresh(editor);
            self.open = true;
            self.focused = true;
        }
    }

    pub fn refresh_if_open(&mut self, editor: &Editor) {
        if self.open {
            self.refresh(editor);
        }
    }

    fn refresh(&mut self, editor: &Editor) {
        use helix_loader::workspace_trust::TrustQuery;

        self.root = helix_stdx::path::canonicalize(helix_loader::find_workspace().0);

        let trust_full = editor
            .workspace_trust
            .query(&helix_loader::find_workspace_in(&self.root).0, TrustQuery::Git)
            .is_trusted();

        let status = editor
            .diff_providers
            .working_tree_status(&self.root, trust_full)
            .unwrap_or_default();

        let mut groups = self.build_groups(true, categorize(status.staged));
        groups.extend(self.build_groups(false, categorize(status.unstaged)));
        self.groups = groups;

        self.rebuild_rows();
        if self.selected >= self.rows.len() {
            self.selected = 0;
        }
        self.scroll = 0;
    }

    fn build_roots(&self, groups: &[(ChangeStatus, Vec<PathBuf>)]) -> Vec<Entry> {
        let mut roots: Vec<Entry> = Vec::new();
        for (status, files) in groups {
            for path in files {
                if let Ok(rel) = path.strip_prefix(&self.root) {
                    let comps: Vec<&str> = rel
                        .components()
                        .filter_map(|c| c.as_os_str().to_str())
                        .collect();
                    insert_path(&mut roots, &self.root, &comps, *status);
                }
            }
        }
        for entry in &mut roots {
            compress(entry);
        }
        sort_entries(&mut roots);
        roots
    }

    fn build_groups(&self, staged: bool, buckets: [Vec<PathBuf>; 3]) -> Vec<Group> {
        let [added, modified, deleted] = buckets;

        if staged {
            let count = added.len() + modified.len() + deleted.len();
            if count == 0 {
                return Vec::new();
            }
            let roots = self.build_roots(&[
                (ChangeStatus::Added, added),
                (ChangeStatus::Modified, modified),
                (ChangeStatus::Deleted, deleted),
            ]);
            return vec![Group {
                status: ChangeStatus::Modified,
                staged: true,
                expanded: true,
                count,
                roots,
            }];
        }

        [
            (ChangeStatus::Added, added),
            (ChangeStatus::Modified, modified),
            (ChangeStatus::Deleted, deleted),
        ]
        .into_iter()
        .filter(|(_, files)| !files.is_empty())
        .map(|(status, files)| {
            let count = files.len();
            let roots = self.build_roots(&[(status, files)]);
            Group {
                status,
                staged: false,
                expanded: true,
                count,
                roots,
            }
        })
        .collect()
    }

    fn rebuild_rows(&mut self) {
        let mut rows = Vec::new();
        for group in &self.groups {
            let label = if group.staged {
                format!("Staged ({})", group.count)
            } else {
                format!("{} ({})", group.status.label(), group.count)
            };
            rows.push(Row {
                depth: 0,
                label,
                kind: RowKind::Group,
                status: group.status,
                staged: group.staged,
                path: PathBuf::new(),
                expanded: group.expanded,
            });
            if group.expanded {
                for entry in &group.roots {
                    push_entry_rows(&mut rows, entry, 1, group.status, group.staged);
                }
            }
        }
        self.rows = rows;
    }

    fn move_selection(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last as isize) as usize;
    }

    fn toggle_group(&mut self, staged: bool, status: ChangeStatus) {
        if let Some(group) = self
            .groups
            .iter_mut()
            .find(|g| g.staged == staged && g.status == status)
        {
            group.expanded = !group.expanded;
        }
        self.rebuild_rows();
        self.clamp_selection();
    }

    fn toggle_dir(&mut self, staged: bool, status: ChangeStatus, path: &Path) {
        if let Some(group) = self
            .groups
            .iter_mut()
            .find(|g| g.staged == staged && g.status == status)
        {
            if let Some(entry) = find_entry_mut(&mut group.roots, path) {
                entry.expanded = !entry.expanded;
            }
        }
        self.rebuild_rows();
        self.clamp_selection();
    }

    fn clamp_selection(&mut self) {
        if self.selected >= self.rows.len() {
            self.selected = self.rows.len().saturating_sub(1);
        }
    }

    fn activate(&mut self) -> EventResult {
        let Some(row) = self.rows.get(self.selected) else {
            return EventResult::Consumed(None);
        };
        match row.kind {
            RowKind::Group => {
                let staged = row.staged;
                let status = row.status;
                self.toggle_group(staged, status);
                EventResult::Consumed(None)
            }
            RowKind::Dir => {
                let staged = row.staged;
                let status = row.status;
                let path = row.path.clone();
                self.toggle_dir(staged, status, &path);
                EventResult::Consumed(None)
            }
            RowKind::File => {
                let path = row.path.clone();
                self.unfocus();
                let callback: Callback = Box::new(move |_compositor, cx| {
                    match cx.editor.open(&path, Action::Replace) {
                        Ok(doc_id) => schedule_goto_first_change(cx.jobs, doc_id),
                        Err(err) => cx
                            .editor
                            .set_error(format!("Failed to open {}: {}", path.display(), err)),
                    }
                });
                EventResult::Consumed(Some(callback))
            }
        }
    }

    pub fn handle_key(&mut self, event: KeyEvent, editor: &mut Editor) -> EventResult {
        if event.modifiers.contains(KeyModifiers::CONTROL) {
            return EventResult::Ignored(None);
        }

        match event.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Char('l') | KeyCode::Enter | KeyCode::Right => return self.activate(),
            KeyCode::Char('h') | KeyCode::Left => {
                if let Some(row) = self.rows.get(self.selected) {
                    match row.kind {
                        RowKind::Group if row.expanded => {
                            let staged = row.staged;
                            let status = row.status;
                            self.toggle_group(staged, status);
                        }
                        RowKind::Dir if row.expanded => {
                            let staged = row.staged;
                            let status = row.status;
                            let path = row.path.clone();
                            self.toggle_dir(staged, status, &path);
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Char('s') => {
                if let Some(row) = self.rows.get(self.selected) {
                    if let RowKind::File = row.kind {
                        return EventResult::Consumed(Some(Self::toggle_stage_prompt(
                            self.root.clone(),
                            row.path.clone(),
                            row.staged,
                        )));
                    }
                }
            }
            KeyCode::Char('d') => {
                if let Some(row) = self.rows.get(self.selected) {
                    if let RowKind::File = row.kind {
                        return EventResult::Consumed(Some(Self::discard_prompt(
                            self.root.clone(),
                            row.path.clone(),
                            row.status,
                            row.staged,
                        )));
                    }
                }
            }
            KeyCode::Char('R') => self.refresh(editor),
            KeyCode::Char('q') | KeyCode::Esc => self.unfocus(),
            _ => {}
        }

        EventResult::Consumed(None)
    }

    pub fn render(&mut self, area: Rect, surface: &mut Surface, editor: &Editor) {
        let theme = &editor.theme;
        let background = theme.get("ui.background");
        let text_style = theme.get("ui.text");
        let dir_style = theme.get("ui.text.focus");
        let header_style = text_style.add_modifier(Modifier::BOLD);
        let selected_style = if self.focused {
            theme.get("ui.menu.selected")
        } else {
            theme.get("ui.cursorline.primary")
        };

        let added_style = theme
            .try_get("version_control.added")
            .unwrap_or_else(|| Style::default().fg(Color::Rgb(0x27, 0xA6, 0x57)));
        let modified_style = theme
            .try_get("version_control.modified")
            .unwrap_or_else(|| Style::default().fg(Color::Rgb(0xD3, 0xB0, 0x20)));
        let deleted_style = theme
            .try_get("version_control.deleted")
            .unwrap_or_else(|| Style::default().fg(Color::Rgb(0xE0, 0x6C, 0x76)));
        let status_style = |status: ChangeStatus| match status {
            ChangeStatus::Added => added_style,
            ChangeStatus::Modified => modified_style,
            ChangeStatus::Deleted => deleted_style,
        };

        surface.set_style(area, background);

        let separator_style = theme.get("ui.window");
        let separator_x = area.right().saturating_sub(1);
        for y in area.top()..area.bottom() {
            surface[(separator_x, y)]
                .set_symbol(tui::symbols::line::VERTICAL)
                .set_style(separator_style);
        }

        let inner = area.clip_right(1);
        let height = inner.height as usize;

        if self.rows.is_empty() {
            surface.set_stringn(inner.x, inner.y, "No changes", inner.width as usize, text_style);
            return;
        }

        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if height > 0 && self.selected >= self.scroll + height {
            self.scroll = self.selected - height + 1;
        }

        for (index, row) in self.rows.iter().enumerate().skip(self.scroll).take(height) {
            let y = inner.y + (index - self.scroll) as u16;
            let indent = "  ".repeat(row.depth);

            let content_style = match row.kind {
                RowKind::Group => header_style,
                RowKind::Dir => dir_style,
                RowKind::File => status_style(row.status),
            };

            let style = if index == self.selected {
                let mut style = selected_style;
                style.fg = content_style.fg;
                style
            } else {
                content_style
            };

            if index == self.selected {
                surface.set_style(Rect::new(area.x, y, area.width, 1), selected_style);
            }

            if let RowKind::Group = row.kind {
                let marker = if row.expanded { "\u{25be} " } else { "\u{25b8} " };
                let line = format!("{}{}{}", indent, marker, row.label);
                surface.set_stringn(inner.x, y, &line, inner.width as usize, style);
            } else {
                let end = inner.x + inner.width;
                let (x, _) = surface.set_stringn(inner.x, y, &indent, inner.width as usize, style);
                let (icon, color) = if let RowKind::Dir = row.kind {
                    let (glyph, _) = icons::folder_icon(row.expanded);
                    (glyph, dir_style.fg.unwrap_or(Color::Reset))
                } else {
                    icons::file_icon(Path::new(&row.label))
                };
                let glyph = format!("{} ", icon);
                let (x, _) =
                    surface.set_stringn(x, y, &glyph, end.saturating_sub(x) as usize, Style::default().fg(color));
                surface.set_stringn(x, y, &row.label, end.saturating_sub(x) as usize, style);
            }
        }
    }

    fn toggle_stage_prompt(root: PathBuf, path: PathBuf, staged: bool) -> Callback {
        Box::new(move |compositor: &mut Compositor, cx: &mut Context| {
            let prefill = path.to_string_lossy().into_owned();
            let prefix = if staged { "unstage:" } else { "stage:" };

            let prompt = Prompt::new(
                prefix.into(),
                None,
                completers::filename,
                move |cx: &mut Context, input: &str, event: PromptEvent| {
                    if event != PromptEvent::Validate {
                        return;
                    }
                    let input = input.trim();
                    if input.is_empty() {
                        return;
                    }

                    let path = PathBuf::from(input);
                    let args: &[&str] = if staged {
                        &["restore", "--staged"]
                    } else {
                        &["add"]
                    };
                    match run_git(&root, args, &path) {
                        Ok(status) if status.success() => schedule_post_git(None),
                        Ok(_) | Err(_) => cx
                            .editor
                            .set_error(format!("git failed on {}", path.display())),
                    }
                },
            )
            .with_line(prefill, cx.editor);

            compositor.push(Box::new(prompt));
        })
    }

    fn discard_prompt(
        root: PathBuf,
        path: PathBuf,
        status: ChangeStatus,
        staged: bool,
    ) -> Callback {
        Box::new(move |compositor: &mut Compositor, cx: &mut Context| {
            let prefill = path.to_string_lossy().into_owned();

            let prompt = Prompt::new(
                "discard:".into(),
                None,
                completers::filename,
                move |cx: &mut Context, input: &str, event: PromptEvent| {
                    if event != PromptEvent::Validate {
                        return;
                    }
                    let input = input.trim();
                    if input.is_empty() {
                        return;
                    }

                    let path = PathBuf::from(input);
                    match discard(&root, &path, status, staged) {
                        Ok(()) => schedule_post_git(Some(path)),
                        Err(_) => cx
                            .editor
                            .set_error(format!("Could not discard {}", path.display())),
                    }
                },
            )
            .with_line(prefill, cx.editor);

            compositor.push(Box::new(prompt));
        })
    }
}

fn run_git(root: &Path, args: &[&str], path: &Path) -> io::Result<ExitStatus> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .arg("--")
        .arg(path)
        .status()
}

fn discard(root: &Path, path: &Path, status: ChangeStatus, staged: bool) -> io::Result<()> {
    let check = |status: ExitStatus| {
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::Other, "git command failed"))
        }
    };

    if staged {
        check(run_git(root, &["restore", "--staged"], path)?)?;
    }

    match status {
        ChangeStatus::Added => {
            if path.exists() {
                std::fs::remove_file(path)?;
            }
        }
        ChangeStatus::Modified | ChangeStatus::Deleted => {
            check(run_git(root, &["restore"], path)?)?;
        }
    }

    Ok(())
}

fn schedule_post_git(reload_path: Option<PathBuf>) {
    use helix_loader::workspace_trust::TrustQuery;

    job::dispatch_blocking(move |editor, compositor| {
        if let Some(reload_path) = &reload_path {
            let reload_path = helix_stdx::path::canonicalize(reload_path);
            let scrolloff = editor.config().scrolloff;
            let (view, doc) = current!(editor);
            if doc.path().map(helix_stdx::path::canonicalize) == Some(reload_path) {
                let trust_full = editor
                    .workspace_trust
                    .query(doc.workspace_root(), TrustQuery::Git)
                    .is_trusted();
                if doc.reload(view, &editor.diff_providers, trust_full).is_ok() {
                    view.ensure_cursor_in_view(doc, scrolloff);
                }
            }
        }

        if let Some(editor_view) = compositor.find::<EditorView>() {
            editor_view.changes.refresh_if_open(editor);
            editor_view.explorer.refresh_if_open(editor);
        }
    });
}

fn insert_path(children: &mut Vec<Entry>, base: &Path, comps: &[&str], status: ChangeStatus) {
    let Some((first, rest)) = comps.split_first() else {
        return;
    };
    let path = base.join(first);
    let is_file = rest.is_empty();
    let index = match children.iter().position(|child| child.name == *first) {
        Some(index) => index,
        None => {
            children.push(Entry {
                name: (*first).to_string(),
                path: path.clone(),
                is_dir: !is_file,
                status,
                expanded: true,
                children: Vec::new(),
            });
            children.len() - 1
        }
    };
    if !is_file {
        insert_path(&mut children[index].children, &path, rest, status);
    }
}

fn compress(entry: &mut Entry) {
    for child in &mut entry.children {
        compress(child);
    }
    while entry.is_dir && entry.children.len() == 1 && entry.children[0].is_dir {
        let child = entry.children.remove(0);
        entry.name = format!("{}{}{}", entry.name, MAIN_SEPARATOR, child.name);
        entry.path = child.path;
        entry.children = child.children;
    }
}

fn sort_entries(entries: &mut Vec<Entry>) {
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    for entry in entries {
        sort_entries(&mut entry.children);
    }
}

fn push_entry_rows(
    rows: &mut Vec<Row>,
    entry: &Entry,
    depth: usize,
    group_status: ChangeStatus,
    staged: bool,
) {
    rows.push(Row {
        depth,
        label: entry.name.clone(),
        kind: if entry.is_dir {
            RowKind::Dir
        } else {
            RowKind::File
        },
        status: if entry.is_dir {
            group_status
        } else {
            entry.status
        },
        staged,
        path: entry.path.clone(),
        expanded: entry.expanded,
    });
    if entry.is_dir && entry.expanded {
        for child in &entry.children {
            push_entry_rows(rows, child, depth + 1, group_status, staged);
        }
    }
}

fn categorize(changes: Vec<FileChange>) -> [Vec<PathBuf>; 3] {
    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut deleted = Vec::new();
    for change in changes {
        let (status, path) = match change {
            FileChange::Untracked { path } => (ChangeStatus::Added, path),
            FileChange::Modified { path } | FileChange::Conflict { path } => {
                (ChangeStatus::Modified, path)
            }
            FileChange::Renamed { to_path, .. } => (ChangeStatus::Modified, to_path),
            FileChange::Deleted { path } => (ChangeStatus::Deleted, path),
        };
        let path = helix_stdx::path::canonicalize(path);
        match status {
            ChangeStatus::Added => added.push(path),
            ChangeStatus::Modified => modified.push(path),
            ChangeStatus::Deleted => deleted.push(path),
        }
    }
    [added, modified, deleted]
}

fn find_entry_mut<'a>(entries: &'a mut [Entry], path: &Path) -> Option<&'a mut Entry> {
    for entry in entries.iter_mut() {
        if entry.path == path {
            return Some(entry);
        }
        if entry.is_dir {
            if let Some(found) = find_entry_mut(&mut entry.children, path) {
                return Some(found);
            }
        }
    }
    None
}
