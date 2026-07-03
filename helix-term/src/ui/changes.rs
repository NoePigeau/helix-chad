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
    theme::Theme,
    DocumentId, Editor,
};

use tui::buffer::Buffer as Surface;

use crate::compositor::{Callback, Compositor, Context, EventResult};
use crate::job::{self, Job, Jobs};
use crate::ui::sidebar::{self, GitStatus, SidebarState};
use crate::ui::{completers, icons, EditorView, Prompt, PromptEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Staged,
    Unstaged(GitStatus),
}

impl Section {
    fn label(self, count: usize) -> String {
        match self {
            Self::Staged => format!("Staged ({count})"),
            Self::Unstaged(status) => format!("{} ({count})", status.label()),
        }
    }

    fn is_staged(self) -> bool {
        matches!(self, Self::Staged)
    }
}

#[derive(Debug)]
struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    status: GitStatus,
    expanded: bool,
    children: Vec<Entry>,
}

#[derive(Debug)]
struct Group {
    section: Section,
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
    section: Section,
    status: GitStatus,
    path: PathBuf,
    expanded: bool,
}

struct RowStyles {
    dir: Style,
    header: Style,
    selected: Style,
}

impl RowStyles {
    fn from_theme(theme: &Theme, focused: bool) -> Self {
        let text = theme.get("ui.text");

        Self {
            dir: theme.get("ui.text.focus"),
            header: text.add_modifier(Modifier::BOLD),
            selected: if focused {
                theme.get("ui.menu.selected")
            } else {
                theme.get("ui.cursorline.primary")
            },
        }
    }
}

#[derive(Debug, Default)]
pub struct ChangesSidebar {
    state: SidebarState,
    root: PathBuf,
    groups: Vec<Group>,
    rows: Vec<Row>,
}

impl ChangesSidebar {
    pub fn is_open(&self) -> bool {
        self.state.is_open()
    }

    pub fn is_focused(&self) -> bool {
        self.state.is_focused()
    }

    pub fn width(&self) -> u16 {
        self.state.width()
    }

    pub fn set_width(&mut self, width: u16) {
        self.state.set_width(width);
    }

    pub fn close(&mut self) {
        self.state.close();
    }

    pub fn unfocus(&mut self) {
        self.state.unfocus();
    }

    pub fn focus(&mut self, editor: &Editor) {
        self.refresh(editor);
        self.state.open_focused();
    }

    pub fn refresh_if_open(&mut self, editor: &Editor) {
        if self.state.is_open() {
            self.refresh(editor);
        }
    }

    fn refresh(&mut self, editor: &Editor) {
        self.root = helix_stdx::path::canonicalize(helix_loader::find_workspace().0);

        let workspace = helix_loader::find_workspace_in(&self.root).0;
        let trust_full = sidebar::is_git_trusted(editor, &workspace);

        let status = editor
            .diff_providers
            .working_tree_status(&self.root, trust_full)
            .unwrap_or_default();

        let mut groups = self.build_groups(true, categorize(status.staged));
        groups.extend(self.build_groups(false, categorize(status.unstaged)));
        self.groups = groups;

        self.rebuild_rows();

        if self.state.selected >= self.rows.len() {
            self.state.selected = 0;
        }
        self.state.scroll = 0;
    }

    fn build_groups(&self, staged: bool, buckets: [Vec<PathBuf>; 3]) -> Vec<Group> {
        let [added, modified, deleted] = buckets;

        if staged {
            let count = added.len() + modified.len() + deleted.len();
            if count == 0 {
                return Vec::new();
            }

            let roots = self.build_roots(&[
                (GitStatus::Added, added),
                (GitStatus::Modified, modified),
                (GitStatus::Deleted, deleted),
            ]);

            return vec![Group {
                section: Section::Staged,
                expanded: true,
                count,
                roots,
            }];
        }

        [
            (GitStatus::Added, added),
            (GitStatus::Modified, modified),
            (GitStatus::Deleted, deleted),
        ]
        .into_iter()
        .filter(|(_, files)| !files.is_empty())
        .map(|(status, files)| {
            let count = files.len();
            let roots = self.build_roots(&[(status, files)]);

            Group {
                section: Section::Unstaged(status),
                expanded: true,
                count,
                roots,
            }
        })
        .collect()
    }

    fn build_roots(&self, groups: &[(GitStatus, Vec<PathBuf>)]) -> Vec<Entry> {
        let mut roots: Vec<Entry> = Vec::new();

        for (status, files) in groups {
            for path in files {
                if let Ok(rel) = path.strip_prefix(&self.root) {
                    let components: Vec<&str> = rel
                        .components()
                        .filter_map(|c| c.as_os_str().to_str())
                        .collect();
                    insert_path(&mut roots, &self.root, &components, *status);
                }
            }
        }

        for entry in &mut roots {
            compress(entry);
        }
        sort_entries(&mut roots);

        roots
    }

    fn rebuild_rows(&mut self) {
        let mut rows = Vec::new();

        for group in &self.groups {
            rows.push(Row {
                depth: 0,
                label: group.section.label(group.count),
                kind: RowKind::Group,
                section: group.section,
                status: GitStatus::Modified,
                path: PathBuf::new(),
                expanded: group.expanded,
            });

            if group.expanded {
                for entry in &group.roots {
                    push_entry_rows(&mut rows, entry, 1, group.section);
                }
            }
        }

        self.rows = rows;
    }

    fn selected_file(&self) -> Option<&Row> {
        self.rows
            .get(self.state.selected)
            .filter(|row| matches!(row.kind, RowKind::File))
    }

    fn toggle_group(&mut self, section: Section) {
        if let Some(group) = self.groups.iter_mut().find(|g| g.section == section) {
            group.expanded = !group.expanded;
        }
        self.rebuild_rows();
        self.state.clamp_selection(self.rows.len());
    }

    fn toggle_dir(&mut self, section: Section, path: &Path) {
        if let Some(group) = self.groups.iter_mut().find(|g| g.section == section) {
            if let Some(entry) = find_entry_mut(&mut group.roots, path) {
                entry.expanded = !entry.expanded;
            }
        }
        self.rebuild_rows();
        self.state.clamp_selection(self.rows.len());
    }

    fn activate(&mut self) -> EventResult {
        let Some(row) = self.rows.get(self.state.selected) else {
            return EventResult::Consumed(None);
        };

        match row.kind {
            RowKind::Group => {
                self.toggle_group(row.section);
                EventResult::Consumed(None)
            }
            RowKind::Dir => {
                let section = row.section;
                let path = row.path.clone();
                self.toggle_dir(section, &path);
                EventResult::Consumed(None)
            }
            RowKind::File => {
                let path = row.path.clone();
                self.state.unfocus();

                let callback: Callback =
                    Box::new(
                        move |_compositor, cx| match cx.editor.open(&path, Action::Replace) {
                            Ok(doc_id) => schedule_goto_first_change(cx.jobs, doc_id),
                            Err(err) => cx.editor.set_error(format!(
                                "Failed to open {}: {}",
                                path.display(),
                                err
                            )),
                        },
                    );
                EventResult::Consumed(Some(callback))
            }
        }
    }

    pub fn handle_key(&mut self, event: KeyEvent, editor: &mut Editor) -> EventResult {
        if event.modifiers.contains(KeyModifiers::CONTROL) {
            return EventResult::Ignored(None);
        }

        let keys = editor.config().sidebar.git_changes.clone();

        if event == keys.stage {
            if let Some(row) = self.selected_file() {
                return EventResult::Consumed(Some(Self::toggle_stage_prompt(
                    self.root.clone(),
                    row.path.clone(),
                    row.section.is_staged(),
                )));
            }
            return EventResult::Consumed(None);
        }
        if event == keys.discard {
            if let Some(row) = self.selected_file() {
                return EventResult::Consumed(Some(Self::discard_prompt(
                    self.root.clone(),
                    row.path.clone(),
                    row.status,
                    row.section.is_staged(),
                )));
            }
            return EventResult::Consumed(None);
        }
        if event == keys.reload {
            self.refresh(editor);
            return EventResult::Consumed(None);
        }

        match event.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.state.move_selection(1, self.rows.len());
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.state.move_selection(-1, self.rows.len());
            }
            KeyCode::Char('l') | KeyCode::Enter | KeyCode::Right => return self.activate(),
            KeyCode::Char('h') | KeyCode::Left => self.collapse_selected(),
            KeyCode::Char('q') | KeyCode::Esc => self.state.unfocus(),
            _ => {}
        }

        EventResult::Consumed(None)
    }

    fn collapse_selected(&mut self) {
        let Some(row) = self.rows.get(self.state.selected) else {
            return;
        };
        if !row.expanded {
            return;
        }

        match row.kind {
            RowKind::Group => self.toggle_group(row.section),
            RowKind::Dir => {
                let section = row.section;
                let path = row.path.clone();
                self.toggle_dir(section, &path);
            }
            RowKind::File => {}
        }
    }

    pub fn render(&mut self, area: Rect, surface: &mut Surface, editor: &Editor) {
        let theme = &editor.theme;
        let background = theme.get("ui.background");
        surface.set_style(area, background);

        sidebar::draw_separator(area, surface, theme);

        let inner = area.clip_right(1);
        let height = inner.height as usize;

        if self.rows.is_empty() {
            let text_style = theme.get("ui.text");
            surface.set_stringn(
                inner.x,
                inner.y,
                "No changes",
                inner.width as usize,
                text_style,
            );
            return;
        }

        self.state.adjust_scroll(height);

        let styles = RowStyles::from_theme(theme, self.state.is_focused());
        for (index, row) in self
            .rows
            .iter()
            .enumerate()
            .skip(self.state.scroll)
            .take(height)
        {
            let y = inner.y + (index - self.state.scroll) as u16;
            self.render_row(index, row, y, area, inner, surface, theme, &styles);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_row(
        &self,
        index: usize,
        row: &Row,
        y: u16,
        area: Rect,
        inner: Rect,
        surface: &mut Surface,
        theme: &Theme,
        styles: &RowStyles,
    ) {
        let content_style = match row.kind {
            RowKind::Group => styles.header,
            RowKind::Dir => styles.dir,
            RowKind::File => row.status.style(theme),
        };

        let selected = index == self.state.selected;
        let style = if selected {
            let mut style = styles.selected;
            style.fg = content_style.fg;
            style
        } else {
            content_style
        };

        if selected {
            surface.set_style(Rect::new(area.x, y, area.width, 1), styles.selected);
        }

        let indent = "  ".repeat(row.depth);
        if let RowKind::Group = row.kind {
            let marker = if row.expanded {
                "\u{25be} "
            } else {
                "\u{25b8} "
            };
            let line = format!("{indent}{marker}{}", row.label);
            surface.set_stringn(inner.x, y, &line, inner.width as usize, style);
            return;
        }

        let end = inner.x + inner.width;
        let (x, _) = surface.set_stringn(inner.x, y, &indent, inner.width as usize, style);

        let (icon, color) = if let RowKind::Dir = row.kind {
            let (glyph, _) = icons::folder_icon(row.expanded);
            (glyph, styles.dir.fg.unwrap_or(Color::Reset))
        } else {
            icons::file_icon(Path::new(&row.label))
        };

        let glyph = format!("{icon} ");
        let (x, _) = surface.set_stringn(
            x,
            y,
            &glyph,
            end.saturating_sub(x) as usize,
            Style::default().fg(color),
        );
        surface.set_stringn(x, y, &row.label, end.saturating_sub(x) as usize, style);
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

    fn discard_prompt(root: PathBuf, path: PathBuf, status: GitStatus, staged: bool) -> Callback {
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

fn insert_path(children: &mut Vec<Entry>, base: &Path, components: &[&str], status: GitStatus) {
    let Some((first, rest)) = components.split_first() else {
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

fn push_entry_rows(rows: &mut Vec<Row>, entry: &Entry, depth: usize, section: Section) {
    rows.push(Row {
        depth,
        label: entry.name.clone(),
        kind: if entry.is_dir {
            RowKind::Dir
        } else {
            RowKind::File
        },
        section,
        status: entry.status,
        path: entry.path.clone(),
        expanded: entry.expanded,
    });

    if entry.is_dir && entry.expanded {
        for child in &entry.children {
            push_entry_rows(rows, child, depth + 1, section);
        }
    }
}

fn categorize(changes: Vec<FileChange>) -> [Vec<PathBuf>; 3] {
    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut deleted = Vec::new();

    for change in changes {
        let status = GitStatus::from_change(&change);
        let path = helix_stdx::path::canonicalize(change.path());
        match status {
            GitStatus::Added => added.push(path),
            GitStatus::Modified => modified.push(path),
            GitStatus::Deleted => deleted.push(path),
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

fn run_git(root: &Path, args: &[&str], path: &Path) -> io::Result<ExitStatus> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .arg("--")
        .arg(path)
        .status()
}

fn discard(root: &Path, path: &Path, status: GitStatus, staged: bool) -> io::Result<()> {
    let check = |status: ExitStatus| {
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other("git command failed"))
        }
    };

    if staged {
        check(run_git(root, &["restore", "--staged"], path)?)?;
    }

    match status {
        GitStatus::Added => {
            if path.is_dir() {
                std::fs::remove_dir_all(path)?;
            } else if path.exists() {
                std::fs::remove_file(path)?;
            }
        }
        GitStatus::Modified | GitStatus::Deleted => {
            check(run_git(root, &["restore"], path)?)?;
        }
    }

    Ok(())
}

fn schedule_post_git(reload_path: Option<PathBuf>) {
    job::dispatch_blocking(move |editor, compositor| {
        if let Some(reload_path) = &reload_path {
            reload_document(editor, reload_path);
        }

        if let Some(editor_view) = compositor.find::<EditorView>() {
            editor_view.changes.refresh_if_open(editor);
            editor_view.explorer.refresh_if_open(editor);
        }
    });
}

fn reload_document(editor: &mut Editor, reload_path: &Path) {
    let reload_path = helix_stdx::path::canonicalize(reload_path);
    let scrolloff = editor.config().scrolloff;

    let workspace = {
        let (_, doc) = current!(editor);
        if doc.path().map(helix_stdx::path::canonicalize) != Some(reload_path) {
            return;
        }
        doc.workspace_root().to_path_buf()
    };

    let trust_full = sidebar::is_git_trusted(editor, &workspace);

    let (view, doc) = current!(editor);
    if doc.reload(view, &editor.diff_providers, trust_full).is_ok() {
        view.ensure_cursor_in_view(doc, scrolloff);
    }
}

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
