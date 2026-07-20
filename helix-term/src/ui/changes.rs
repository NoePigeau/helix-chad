use std::collections::HashMap;
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

const CHECKBOX_WIDTH: u16 = 2;
const CHECKBOX_GAP: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StageState {
    Unstaged,
    Staged,
    Partial,
}

impl StageState {
    fn glyph(self) -> &'static str {
        match self {
            Self::Unstaged => "\u{2610}",
            Self::Staged => "\u{2611}",
            Self::Partial => "\u{25a3}",
        }
    }

    fn is_staged(self) -> bool {
        !matches!(self, Self::Unstaged)
    }
}

#[derive(Debug, Clone, Copy)]
struct ChangeInfo {
    status: GitStatus,
    stage: StageState,
}

#[derive(Debug)]
struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    status: GitStatus,
    stage: StageState,
    expanded: bool,
    children: Vec<Entry>,
}

#[derive(Debug)]
struct Group {
    status: GitStatus,
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
    status: GitStatus,
    stage: StageState,
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
        let mut dir = theme.get("ui.text.focus");
        dir.bg = None;

        Self {
            dir,
            header: text.add_modifier(Modifier::BOLD),
            selected: if focused {
                theme.get("ui.selection")
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

        let previous = self.rows.get(self.state.selected).map(|row| row.path.clone());

        self.groups = self.build_groups(collect_changes(status.staged, status.unstaged));

        self.rebuild_rows();

        self.restore_selection(previous);
        self.state.scroll = 0;
    }

    fn restore_selection(&mut self, path: Option<PathBuf>) {
        let restored = path
            .filter(|path| !path.as_os_str().is_empty())
            .and_then(|path| self.rows.iter().position(|row| row.path == path));

        match restored {
            Some(index) => self.state.selected = index,
            None => self.state.clamp_selection(self.rows.len()),
        }
    }

    fn build_groups(&self, changes: HashMap<PathBuf, ChangeInfo>) -> Vec<Group> {
        [GitStatus::Added, GitStatus::Modified, GitStatus::Deleted]
            .into_iter()
            .filter_map(|status| self.build_group(status, &changes))
            .collect()
    }

    fn build_group(&self, status: GitStatus, changes: &HashMap<PathBuf, ChangeInfo>) -> Option<Group> {
        let files: Vec<(PathBuf, StageState)> = changes
            .iter()
            .filter(|(_, info)| info.status == status)
            .map(|(path, info)| (path.clone(), info.stage))
            .collect();

        if files.is_empty() {
            return None;
        }

        let count = files.len();
        let roots = self.build_roots(status, &files);

        Some(Group {
            status,
            expanded: true,
            count,
            roots,
        })
    }

    fn build_roots(&self, status: GitStatus, files: &[(PathBuf, StageState)]) -> Vec<Entry> {
        let mut roots: Vec<Entry> = Vec::new();

        for (path, stage) in files {
            if let Ok(rel) = path.strip_prefix(&self.root) {
                let components: Vec<&str> = rel
                    .components()
                    .filter_map(|c| c.as_os_str().to_str())
                    .collect();
                insert_path(&mut roots, &self.root, &components, status, *stage);
            }
        }

        for entry in &mut roots {
            compress(entry);
            aggregate_stage(entry);
        }
        sort_entries(&mut roots);

        roots
    }

    fn rebuild_rows(&mut self) {
        let mut rows = Vec::new();

        for group in &self.groups {
            rows.push(Row {
                depth: 0,
                label: format!("{} ({})", group.status.label(), group.count),
                kind: RowKind::Group,
                status: group.status,
                stage: StageState::Unstaged,
                path: PathBuf::new(),
                expanded: group.expanded,
            });

            if group.expanded {
                for entry in &group.roots {
                    push_entry_rows(&mut rows, entry, 1);
                }
            }
        }

        self.rows = rows;
    }

    fn selected_target(&self) -> Option<&Row> {
        self.rows
            .get(self.state.selected)
            .filter(|row| matches!(row.kind, RowKind::File | RowKind::Dir))
    }

    fn collect_target_files(&self, status: GitStatus, path: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();

        if let Some(group) = self.groups.iter().find(|g| g.status == status) {
            if let Some(entry) = find_entry(&group.roots, path) {
                collect_leaves(entry, &mut files);
            }
        }

        files
    }

    fn toggle_group(&mut self, status: GitStatus) {
        if let Some(group) = self.groups.iter_mut().find(|g| g.status == status) {
            group.expanded = !group.expanded;
        }
        self.rebuild_rows();
        self.state.clamp_selection(self.rows.len());
    }

    fn toggle_dir(&mut self, status: GitStatus, path: &Path) {
        if let Some(group) = self.groups.iter_mut().find(|g| g.status == status) {
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
                self.toggle_group(row.status);
                EventResult::Consumed(None)
            }
            RowKind::Dir => {
                let status = row.status;
                let path = row.path.clone();
                self.toggle_dir(status, &path);
                EventResult::Consumed(None)
            }
            RowKind::File => {
                let path = row.path.clone();
                self.state.unfocus();
                EventResult::Consumed(Some(open_file_callback(path)))
            }
        }
    }

    pub fn handle_key(&mut self, event: KeyEvent, editor: &mut Editor) -> EventResult {
        if event.modifiers.contains(KeyModifiers::CONTROL) {
            return EventResult::Ignored(None);
        }

        let keys = editor.config().sidebar.git_changes.clone();

        if event == keys.stage {
            self.toggle_stage(editor);
            return EventResult::Consumed(None);
        }

        if event == keys.discard {
            return EventResult::Consumed(self.discard_selection());
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
            RowKind::Group => self.toggle_group(row.status),
            RowKind::Dir => {
                let status = row.status;
                let path = row.path.clone();
                self.toggle_dir(status, &path);
            }
            RowKind::File => {}
        }
    }

    pub fn render(&mut self, area: Rect, surface: &mut Surface, editor: &Editor) {
        let theme = &editor.theme;
        surface.set_style(area, theme.get("ui.background"));
        sidebar::draw_separator(area, surface, theme);

        let panel = area.clip_right(1);
        render_title(panel, surface, theme);

        let inner = panel.clip_top(2);
        let height = inner.height as usize;

        if self.rows.is_empty() {
            render_empty(inner, surface, theme);
            return;
        }

        self.state.adjust_scroll(height);
        self.render_rows(inner, area, surface, theme, height);
    }

    fn render_rows(
        &self,
        inner: Rect,
        area: Rect,
        surface: &mut Surface,
        theme: &Theme,
        height: usize,
    ) {
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
        let selected = index == self.state.selected;
        let style = self.row_style(row, selected, theme, styles);

        if selected {
            surface.set_style(Rect::new(area.x, y, area.width, 1), styles.selected);
        }

        match row.kind {
            RowKind::Group => render_group_row(row, y, inner, surface, style),
            RowKind::Dir | RowKind::File => render_entry_row(row, y, inner, surface, style, styles),
        }
    }

    fn row_style(&self, row: &Row, selected: bool, theme: &Theme, styles: &RowStyles) -> Style {
        let content_style = match row.kind {
            RowKind::Group => styles.header,
            RowKind::Dir => styles.dir,
            RowKind::File => row.status.style(theme),
        };

        if selected {
            content_style.patch(styles.selected)
        } else {
            content_style
        }
    }

    fn toggle_stage(&mut self, editor: &mut Editor) {
        let Some(row) = self.selected_target() else {
            return;
        };

        let status = row.status;
        let path = row.path.clone();
        let fully_staged = matches!(row.stage, StageState::Staged);

        let files = self.collect_target_files(status, &path);
        if files.is_empty() {
            return;
        }

        let args: &[&str] = if fully_staged {
            &["restore", "--staged"]
        } else {
            &["add"]
        };

        match run_git(&self.root, args, &files) {
            Ok(exit) if exit.success() => self.refresh(editor),
            Ok(_) | Err(_) => editor.set_error(format!("git failed on {}", path.display())),
        }
    }

    fn discard_selection(&self) -> Option<Callback> {
        let row = self.selected_target()?;
        let status = row.status;
        let path = row.path.clone();
        let staged = row.stage.is_staged();

        let files = self.collect_target_files(status, &path);
        if files.is_empty() {
            return None;
        }

        Some(Self::discard_prompt(
            self.root.clone(),
            path,
            files,
            status,
            staged,
        ))
    }

    fn discard_prompt(
        root: PathBuf,
        path: PathBuf,
        files: Vec<PathBuf>,
        status: GitStatus,
        staged: bool,
    ) -> Callback {
        Box::new(move |compositor: &mut Compositor, cx: &mut Context| {
            let prefill = path.to_string_lossy().into_owned();

            let prompt = Prompt::new(
                "discard (enter to confirm):".into(),
                None,
                completers::none,
                move |cx: &mut Context, _input: &str, event: PromptEvent| {
                    if event != PromptEvent::Validate {
                        return;
                    }

                    let reload = single_file(&files);
                    match discard(&root, &files, status, staged) {
                        Ok(()) => schedule_post_git(reload),
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

fn single_file(files: &[PathBuf]) -> Option<PathBuf> {
    match files {
        [path] => Some(path.clone()),
        _ => None,
    }
}

fn open_file_callback(path: PathBuf) -> Callback {
    Box::new(
        move |_compositor, cx| match cx.editor.open(&path, Action::Replace) {
            Ok(doc_id) => schedule_goto_first_change(cx.jobs, doc_id),
            Err(err) => cx
                .editor
                .set_error(format!("Failed to open {}: {}", path.display(), err)),
        },
    )
}

fn render_title(panel: Rect, surface: &mut Surface, theme: &Theme) {
    let title = "Git changes";
    let style = theme
        .try_get("ui.text.inactive")
        .or_else(|| theme.try_get("ui.virtual"))
        .or_else(|| theme.try_get("comment"))
        .unwrap_or_else(|| theme.get("ui.text"))
        .add_modifier(Modifier::BOLD);

    let x = panel.x + panel.width.saturating_sub(title.len() as u16) / 2;
    surface.set_stringn(x, panel.y, title, panel.width as usize, style);
}

fn render_empty(inner: Rect, surface: &mut Surface, theme: &Theme) {
    let style = theme.get("ui.text");
    surface.set_stringn(inner.x, inner.y, "No changes", inner.width as usize, style);
}

fn render_group_row(row: &Row, y: u16, inner: Rect, surface: &mut Surface, style: Style) {
    let indent = "  ".repeat(row.depth);
    let marker = if row.expanded { "\u{25be} " } else { "\u{25b8} " };
    let line = format!("{indent}{marker}{}", row.label);

    surface.set_stringn(inner.x, y, &line, inner.width as usize, style);
}

fn render_entry_row(
    row: &Row,
    y: u16,
    inner: Rect,
    surface: &mut Surface,
    style: Style,
    styles: &RowStyles,
) {
    let end = inner.x + inner.width;
    let label_end = end.saturating_sub(CHECKBOX_GAP + CHECKBOX_WIDTH);

    let indent = "  ".repeat(row.depth);
    let (x, _) = surface.set_stringn(inner.x, y, &indent, inner.width as usize, style);

    let (icon, color) = entry_icon(row, styles);
    let glyph = format!("{icon} ");
    let (x, _) = surface.set_stringn(
        x,
        y,
        &glyph,
        label_end.saturating_sub(x) as usize,
        Style::default().fg(color),
    );
    surface.set_stringn(x, y, &row.label, label_end.saturating_sub(x) as usize, style);

    let cb_x = end.saturating_sub(CHECKBOX_WIDTH);
    surface.set_stringn(cb_x, y, row.stage.glyph(), CHECKBOX_WIDTH as usize, style);
}

fn entry_icon(row: &Row, styles: &RowStyles) -> (&'static str, Color) {
    if let RowKind::Dir = row.kind {
        let (glyph, _) = icons::folder_icon(row.expanded);
        (glyph, styles.dir.fg.unwrap_or(Color::Reset))
    } else {
        icons::file_icon(Path::new(&row.label))
    }
}

fn insert_path(
    children: &mut Vec<Entry>,
    base: &Path,
    components: &[&str],
    status: GitStatus,
    stage: StageState,
) {
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
                stage,
                expanded: true,
                children: Vec::new(),
            });
            children.len() - 1
        }
    };

    if !is_file {
        insert_path(&mut children[index].children, &path, rest, status, stage);
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

fn push_entry_rows(rows: &mut Vec<Row>, entry: &Entry, depth: usize) {
    rows.push(Row {
        depth,
        label: entry.name.clone(),
        kind: if entry.is_dir {
            RowKind::Dir
        } else {
            RowKind::File
        },
        status: entry.status,
        stage: entry.stage,
        path: entry.path.clone(),
        expanded: entry.expanded,
    });

    if entry.is_dir && entry.expanded {
        for child in &entry.children {
            push_entry_rows(rows, child, depth + 1);
        }
    }
}

fn collect_changes(
    staged: Vec<FileChange>,
    unstaged: Vec<FileChange>,
) -> HashMap<PathBuf, ChangeInfo> {
    let mut changes: HashMap<PathBuf, ChangeInfo> = HashMap::new();

    for change in staged {
        let (path, status) = change_info(&change);
        changes.insert(
            path,
            ChangeInfo {
                status,
                stage: StageState::Staged,
            },
        );
    }

    for change in unstaged {
        let (path, status) = change_info(&change);
        changes
            .entry(path)
            .and_modify(|info| info.stage = StageState::Partial)
            .or_insert(ChangeInfo {
                status,
                stage: StageState::Unstaged,
            });
    }

    changes
}

fn change_info(change: &FileChange) -> (PathBuf, GitStatus) {
    let path = helix_stdx::path::canonicalize(change.path());
    let status = GitStatus::from_change(change);

    (path, status)
}

fn entry_index_path(entries: &[Entry], path: &Path) -> Option<Vec<usize>> {
    for (index, entry) in entries.iter().enumerate() {
        if entry.path == path {
            return Some(vec![index]);
        }
        if entry.is_dir {
            if let Some(mut rest) = entry_index_path(&entry.children, path) {
                rest.insert(0, index);
                return Some(rest);
            }
        }
    }
    None
}

fn find_entry_mut<'a>(entries: &'a mut [Entry], path: &Path) -> Option<&'a mut Entry> {
    let indices = entry_index_path(entries, path)?;
    let (first, rest) = indices.split_first()?;

    let mut entry = entries.get_mut(*first)?;
    for index in rest {
        entry = entry.children.get_mut(*index)?;
    }

    Some(entry)
}

fn find_entry<'a>(entries: &'a [Entry], path: &Path) -> Option<&'a Entry> {
    let indices = entry_index_path(entries, path)?;
    let (first, rest) = indices.split_first()?;

    let mut entry = entries.get(*first)?;
    for index in rest {
        entry = entry.children.get(*index)?;
    }

    Some(entry)
}

fn collect_leaves(entry: &Entry, out: &mut Vec<PathBuf>) {
    if entry.is_dir {
        for child in &entry.children {
            collect_leaves(child, out);
        }
    } else {
        out.push(entry.path.clone());
    }
}

fn aggregate_stage(entry: &mut Entry) -> StageState {
    if !entry.is_dir {
        return entry.stage;
    }

    let mut combined: Option<StageState> = None;
    for child in &mut entry.children {
        let child_stage = aggregate_stage(child);
        combined = Some(match combined {
            Some(current) if current != child_stage => StageState::Partial,
            Some(current) => current,
            None => child_stage,
        });
    }

    entry.stage = combined.unwrap_or(StageState::Unstaged);
    entry.stage
}

fn run_git(root: &Path, args: &[&str], paths: &[PathBuf]) -> io::Result<ExitStatus> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .arg("--")
        .args(paths)
        .status()
}

fn discard(root: &Path, files: &[PathBuf], status: GitStatus, staged: bool) -> io::Result<()> {
    let check = |exit: ExitStatus| {
        if exit.success() {
            Ok(())
        } else {
            Err(io::Error::other("git command failed"))
        }
    };

    if staged {
        check(run_git(root, &["restore", "--staged"], files)?)?;
    }

    match status {
        GitStatus::Added => {
            for path in files {
                delete_from_disk(path)?;
            }
            prune_empty_ancestors(root, files);
        }
        GitStatus::Modified | GitStatus::Deleted => {
            check(run_git(root, &["restore"], files)?)?;
        }
    }

    Ok(())
}

fn delete_from_disk(path: &Path) -> io::Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else if path.exists() {
        std::fs::remove_file(path)
    } else {
        Ok(())
    }
}

fn prune_empty_ancestors(root: &Path, files: &[PathBuf]) {
    for file in files {
        let mut dir = file.parent();

        while let Some(current) = dir {
            if current == root || std::fs::remove_dir(current).is_err() {
                break;
            }
            dir = current.parent();
        }
    }
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
