use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf, MAIN_SEPARATOR};

use helix_vcs::FileChange;
use helix_view::{
    editor::Action,
    graphics::{Color, Modifier, Rect, Style},
    input::KeyEvent,
    keyboard::{KeyCode, KeyModifiers},
    theme::Theme,
    Editor,
};

use tui::buffer::Buffer as Surface;

use crate::compositor::{Callback, Compositor, Context, EventResult};
use crate::job;
use crate::ui::{completers, icons, EditorView, Prompt, PromptEvent};

const DEFAULT_WIDTH: u16 = 30;
const MIN_WIDTH: u16 = 15;
const MAX_WIDTH: u16 = 80;

const LEFT_PADDING: u16 = 1;

#[derive(Debug, Clone, Copy)]
pub(crate) enum GitStatus {
    Added,
    Modified,
    Deleted,
}

impl GitStatus {
    fn rank(self) -> u8 {
        match self {
            Self::Added => 1,
            Self::Deleted => 2,
            Self::Modified => 3,
        }
    }

    pub(crate) fn from_change(change: &FileChange) -> Self {
        match change {
            FileChange::Untracked { .. } => Self::Added,
            FileChange::Modified { .. }
            | FileChange::Conflict { .. }
            | FileChange::Renamed { .. } => Self::Modified,
            FileChange::Deleted { .. } => Self::Deleted,
        }
    }

    pub(crate) fn style(self, theme: &Theme) -> Style {
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
struct TreeNode {
    path: PathBuf,
    depth: usize,
    is_dir: bool,
    expanded: bool,
    ignored: bool,
}

#[derive(Debug)]
pub struct ExplorerSidebar {
    open: bool,
    focused: bool,
    width: u16,
    root: PathBuf,
    nodes: Vec<TreeNode>,
    selected: usize,
    scroll: usize,
    git_status: HashMap<PathBuf, GitStatus>,
}

impl Default for ExplorerSidebar {
    fn default() -> Self {
        Self {
            open: false,
            focused: false,
            width: DEFAULT_WIDTH,
            root: PathBuf::new(),
            nodes: Vec::new(),
            selected: 0,
            scroll: 0,
            git_status: HashMap::new(),
        }
    }
}

impl ExplorerSidebar {
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

    fn open_and_focus(&mut self, current_file: Option<PathBuf>, editor: &Editor) {
        let root = helix_stdx::path::canonicalize(helix_loader::find_workspace().0);
        if self.nodes.is_empty() || self.root != root {
            self.root = root;
            self.reload();
        }
        self.refresh_git_status(editor);

        self.open = true;
        self.focused = true;

        if let Some(file) = current_file {
            self.reveal(&file);
        }
    }

    pub fn toggle(&mut self, current_file: Option<PathBuf>, editor: &Editor) {
        if self.open {
            self.open = false;
            self.focused = false;
        } else {
            self.open_and_focus(current_file, editor);
        }
    }

    pub fn focus(&mut self, current_file: Option<PathBuf>, editor: &Editor) {
        self.open_and_focus(current_file, editor);
    }

    pub fn refresh_if_open(&mut self, editor: &Editor) {
        if !self.open {
            return;
        }
        self.reload();
        self.refresh_git_status(editor);
    }

    fn refresh_git_status(&mut self, editor: &Editor) {
        use helix_loader::workspace_trust::TrustQuery;

        self.git_status.clear();

        let trust_full = editor
            .workspace_trust
            .query(&helix_loader::find_workspace_in(&self.root).0, TrustQuery::Git)
            .is_trusted();

        let Ok(changes) = editor.diff_providers.changed_files(&self.root, trust_full) else {
            return;
        };

        for change in changes {
            let status = GitStatus::from_change(&change);

            let path = helix_stdx::path::canonicalize(change.path());
            Self::mark_status(&mut self.git_status, path.clone(), status);

            let mut current = path.as_path();
            while let Some(parent) = current.parent() {
                if parent == self.root || !parent.starts_with(&self.root) {
                    break;
                }
                Self::mark_status(&mut self.git_status, parent.to_path_buf(), status);
                current = parent;
            }
        }
    }

    fn mark_status(map: &mut HashMap<PathBuf, GitStatus>, path: PathBuf, status: GitStatus) {
        let entry = map.entry(path).or_insert(status);
        if status.rank() > entry.rank() {
            *entry = status;
        }
    }

    fn reveal(&mut self, target: &Path) {
        let target = helix_stdx::path::canonicalize(target);
        let Ok(rel) = target.strip_prefix(&self.root) else {
            return;
        };

        let mut current = self.root.clone();
        for component in rel.components() {
            current = current.join(component);
            let Some(index) = self.nodes.iter().position(|node| node.path == current) else {
                return;
            };
            if current == target {
                self.selected = index;
                return;
            }
            if self.nodes[index].is_dir && !self.nodes[index].expanded {
                self.expand(index);
            }
        }
    }

    fn unfocus(&mut self) {
        self.focused = false;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.focused = false;
    }

    fn read_dir(dir: &Path, parent_ignored: bool) -> Vec<(PathBuf, bool, bool)> {
        let mut entries: Vec<(PathBuf, bool)> = match std::fs::read_dir(dir) {
            Ok(read) => read
                .flatten()
                .map(|entry| {
                    let path = entry.path();
                    let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    (path, is_dir)
                })
                .collect(),
            Err(_) => Vec::new(),
        };

        entries.sort_by(|(a, a_dir), (b, b_dir)| {
            b_dir.cmp(a_dir).then_with(|| {
                a.file_name()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .cmp(&b.file_name().unwrap_or_default().to_ascii_lowercase())
            })
        });

        let not_ignored = if parent_ignored {
            HashSet::new()
        } else {
            Self::not_ignored(dir)
        };

        entries
            .into_iter()
            .map(|(path, is_dir)| {
                let ignored = parent_ignored || !not_ignored.contains(&path);
                (path, is_dir, ignored)
            })
            .collect()
    }

    fn not_ignored(dir: &Path) -> HashSet<PathBuf> {
        ignore::WalkBuilder::new(dir)
            .max_depth(Some(1))
            .hidden(false)
            .parents(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .ignore(false)
            .build()
            .flatten()
            .map(|entry| entry.into_path())
            .collect()
    }

    fn reload(&mut self) {
        let expanded: HashSet<PathBuf> = self
            .nodes
            .iter()
            .filter(|node| node.is_dir && node.expanded)
            .map(|node| node.path.clone())
            .collect();
        let selected_path = self.nodes.get(self.selected).map(|node| node.path.clone());

        let root = self.root.clone();
        self.nodes = Self::read_dir(&root, false)
            .into_iter()
            .map(|(path, is_dir, ignored)| TreeNode {
                path,
                depth: 0,
                is_dir,
                expanded: false,
                ignored,
            })
            .collect();

        let mut index = 0;
        while index < self.nodes.len() {
            let node = &self.nodes[index];
            if node.is_dir && !node.expanded && expanded.contains(&node.path) {
                self.expand(index);
            }
            index += 1;
        }

        self.selected = selected_path
            .and_then(|path| self.nodes.iter().position(|node| node.path == path))
            .unwrap_or(0);
        self.scroll = 0;
    }

    fn expand(&mut self, index: usize) {
        let (path, depth, parent_ignored) = {
            let node = &self.nodes[index];
            if !node.is_dir || node.expanded {
                return;
            }
            (node.path.clone(), node.depth, node.ignored)
        };

        let children: Vec<TreeNode> = Self::read_dir(&path, parent_ignored)
            .into_iter()
            .map(|(path, is_dir, ignored)| TreeNode {
                path,
                depth: depth + 1,
                is_dir,
                expanded: false,
                ignored,
            })
            .collect();

        self.nodes[index].expanded = true;
        self.nodes.splice(index + 1..index + 1, children);
    }

    fn collapse(&mut self, index: usize) {
        let depth = self.nodes[index].depth;
        let mut end = index + 1;
        while end < self.nodes.len() && self.nodes[end].depth > depth {
            end += 1;
        }
        self.nodes.drain(index + 1..end);
        self.nodes[index].expanded = false;
    }

    fn collapse_all(&mut self) {
        self.nodes.retain(|node| node.depth == 0);
        for node in &mut self.nodes {
            node.expanded = false;
        }
        self.selected = self.selected.min(self.nodes.len().saturating_sub(1));
        self.scroll = 0;
    }

    fn move_selection(&mut self, delta: isize) {
        if self.nodes.is_empty() {
            return;
        }
        let last = self.nodes.len() - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last as isize) as usize;
    }

    fn activate(&mut self, editor: &mut Editor) {
        let Some(node) = self.nodes.get(self.selected) else {
            return;
        };

        if node.is_dir {
            if node.expanded {
                self.collapse(self.selected);
            } else {
                self.expand(self.selected);
            }
        } else {
            let path = node.path.clone();
            if let Err(err) = editor.open(&path, Action::Replace) {
                editor.set_error(format!("Failed to open {}: {}", path.display(), err));
            } else {
                self.unfocus();
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
            KeyCode::Char('l') | KeyCode::Enter | KeyCode::Right => self.activate(editor),
            KeyCode::Char('h') | KeyCode::Left => {
                if let Some(node) = self.nodes.get(self.selected) {
                    if node.is_dir && node.expanded {
                        self.collapse(self.selected);
                    } else if node.depth > 0 {
                        if let Some(parent) = self.nodes[..self.selected]
                            .iter()
                            .rposition(|n| n.depth < node.depth)
                        {
                            self.selected = parent;
                            self.collapse(parent);
                        }
                    }
                }
            }
            KeyCode::Char('R') => {
                self.reload();
                self.refresh_git_status(editor);
            }
            KeyCode::Char('a') => {
                let target_dir = match self.nodes.get(self.selected) {
                    Some(node) if node.is_dir => node.path.clone(),
                    Some(node) => node
                        .path
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| self.root.clone()),
                    None => self.root.clone(),
                };
                return EventResult::Consumed(Some(Self::create_prompt(target_dir)));
            }
            KeyCode::Char('r') => {
                if let Some(node) = self.nodes.get(self.selected) {
                    let source = node.path.clone();
                    return EventResult::Consumed(Some(Self::rename_prompt(source)));
                }
            }
            KeyCode::Char('d') => {
                if let Some(node) = self.nodes.get(self.selected) {
                    let source = node.path.clone();
                    return EventResult::Consumed(Some(Self::delete_prompt(source)));
                }
            }
            KeyCode::Char('/') => {
                let search_root = match self.nodes.get(self.selected) {
                    Some(node) => node.path.clone(),
                    None => self.root.clone(),
                };
                return EventResult::Consumed(Some(Self::global_search_callback(search_root)));
            }
            KeyCode::Char('W') => self.collapse_all(),
            KeyCode::Char('q') | KeyCode::Esc => self.unfocus(),
            _ => {}
        }

        EventResult::Consumed(None)
    }

    pub(crate) fn reload_and_reveal(&mut self, path: &Path, editor: &Editor) {
        self.reload();
        self.refresh_git_status(editor);
        self.reveal(path);
    }

    fn schedule_reveal(path: PathBuf) {
        job::dispatch_blocking(move |editor, compositor| {
            if let Some(editor_view) = compositor.find::<EditorView>() {
                editor_view.explorer.reload_and_reveal(&path, editor);
            }
        });
    }

    fn global_search_callback(search_root: PathBuf) -> Callback {
        Box::new(move |compositor: &mut Compositor, cx: &mut Context| {
            let mut ctx = crate::commands::Context {
                register: None,
                count: None,
                editor: cx.editor,
                callback: Vec::new(),
                on_next_key_callback: None,
                jobs: cx.jobs,
            };
            crate::commands::global_search_in_directory(&mut ctx, search_root);

            let callbacks = std::mem::take(&mut ctx.callback);
            drop(ctx);
            for callback in callbacks {
                callback(compositor, cx);
            }
        })
    }

    fn create_prompt(target_dir: PathBuf) -> Callback {
        Box::new(move |compositor: &mut Compositor, cx: &mut Context| {
            let mut prefill = target_dir.to_string_lossy().into_owned();
            if !prefill.ends_with(MAIN_SEPARATOR) {
                prefill.push(MAIN_SEPARATOR);
            }

            let prompt = Prompt::new(
                "create:".into(),
                None,
                completers::filename,
                |cx: &mut Context, input: &str, event: PromptEvent| {
                    if event != PromptEvent::Validate {
                        return;
                    }
                    let input = input.trim();
                    if input.is_empty() {
                        return;
                    }

                    let is_dir = input.ends_with(MAIN_SEPARATOR);
                    let path = PathBuf::from(input);
                    match cx.editor.create_path(&path, is_dir) {
                        Ok(()) => Self::schedule_reveal(helix_stdx::path::canonicalize(&path)),
                        Err(err) => cx
                            .editor
                            .set_error(format!("Could not create {}: {}", path.display(), err)),
                    }
                },
            )
            .with_line(prefill, cx.editor);

            compositor.push(Box::new(prompt));
        })
    }

    fn delete_prompt(source: PathBuf) -> Callback {
        Box::new(move |compositor: &mut Compositor, cx: &mut Context| {
            let prefill = source.to_string_lossy().into_owned();

            let prompt = Prompt::new(
                "delete:".into(),
                None,
                completers::filename,
                |cx: &mut Context, input: &str, event: PromptEvent| {
                    if event != PromptEvent::Validate {
                        return;
                    }
                    let input = input.trim();
                    if input.is_empty() {
                        return;
                    }

                    let path = PathBuf::from(input);
                    match cx.editor.delete_path(&path, true) {
                        Ok(()) => {
                            let reveal = path
                                .parent()
                                .filter(|parent| !parent.as_os_str().is_empty())
                                .unwrap_or(&path);
                            Self::schedule_reveal(helix_stdx::path::canonicalize(reveal));
                        }
                        Err(err) => cx
                            .editor
                            .set_error(format!("Could not delete {}: {}", path.display(), err)),
                    }
                },
            )
            .with_line(prefill, cx.editor);

            compositor.push(Box::new(prompt));
        })
    }

    fn rename_prompt(source: PathBuf) -> Callback {
        Box::new(move |compositor: &mut Compositor, cx: &mut Context| {
            let prefill = source.to_string_lossy().into_owned();

            let prompt = Prompt::new(
                "rename:".into(),
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

                    let new_path = PathBuf::from(input);
                    if let Some(parent) = new_path.parent() {
                        if !parent.as_os_str().is_empty() && !parent.exists() {
                            if let Err(err) = std::fs::create_dir_all(parent) {
                                cx.editor.set_error(format!(
                                    "Could not create {}: {}",
                                    parent.display(),
                                    err
                                ));
                                return;
                            }
                        }
                    }

                    match cx.editor.move_path(&source, &new_path) {
                        Ok(()) => Self::schedule_reveal(helix_stdx::path::canonicalize(&new_path)),
                        Err(err) => cx.editor.set_error(format!("Could not rename: {}", err)),
                    }
                },
            )
            .with_line(prefill, cx.editor);

            compositor.push(Box::new(prompt));
        })
    }

    pub fn render(&mut self, area: Rect, surface: &mut Surface, editor: &Editor) {
        let theme = &editor.theme;
        let background = theme.get("ui.background");
        let text_style = theme.get("ui.text");
        let dir_style = theme.get("ui.text.focus");
        let ignored_fg = Color::Rgb(0x9a, 0xa5, 0xb1);
        let ignored_style = Style::default().fg(ignored_fg).add_modifier(Modifier::ITALIC);
        let selected_style = if self.focused {
            theme.get("ui.menu.selected")
        } else {
            theme.get("ui.cursorline.primary")
        };

        surface.set_style(area, background);

        let inner = area.clip_right(1);
        let height = inner.height as usize;

        let content_x = inner.x + LEFT_PADDING;
        let content_width = inner.width.saturating_sub(LEFT_PADDING);

        let separator_style = theme.get("ui.window");
        let separator_x = area.right().saturating_sub(1);
        for y in area.top()..area.bottom() {
            surface[(separator_x, y)]
                .set_symbol(tui::symbols::line::VERTICAL)
                .set_style(separator_style);
        }

        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if height > 0 && self.selected >= self.scroll + height {
            self.scroll = self.selected - height + 1;
        }

        for (row, node) in self
            .nodes
            .iter()
            .enumerate()
            .skip(self.scroll)
            .take(height)
        {
            let y = inner.y + (row - self.scroll) as u16;

            let indent = "  ".repeat(node.depth);
            let name = node
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();

            let git = self.git_status.get(&node.path).copied();
            let content_style = if node.ignored {
                ignored_style
            } else {
                match git {
                    Some(status) => status.style(theme),
                    None if node.is_dir => dir_style,
                    None => text_style,
                }
            };
            let style = if row == self.selected {
                if node.ignored || git.is_some() {
                    let mut style = selected_style;
                    style.fg = content_style.fg;
                    if node.ignored {
                        style = style.add_modifier(Modifier::ITALIC);
                    }
                    style
                } else {
                    selected_style
                }
            } else {
                content_style
            };

            if row == self.selected {
                surface.set_style(Rect::new(area.x, y, area.width, 1), selected_style);
            }

            let end = content_x + content_width;
            let (x, _) = surface.set_stringn(content_x, y, &indent, content_width as usize, style);
            let (icon, icon_color) = if node.is_dir {
                let (glyph, _) = icons::folder_icon(node.expanded);
                (glyph, dir_style.fg)
            } else {
                let (glyph, color) = icons::file_icon(&node.path);
                (glyph, Some(color))
            };
            let icon_style = if node.ignored {
                Style::default().fg(ignored_fg)
            } else {
                Style::default().fg(icon_color.unwrap_or(Color::Reset))
            };
            let glyph = format!("{} ", icon);
            let (x, _) = surface.set_stringn(x, y, &glyph, end.saturating_sub(x) as usize, icon_style);
            let (x, _) = surface.set_stringn(x, y, &name, end.saturating_sub(x) as usize, style);

            let is_modified = !node.is_dir
                && editor
                    .document_by_path(&node.path)
                    .is_some_and(|doc| doc.is_modified());
            if is_modified {
                let dot_style = style.patch(theme.get("keyword"));
                surface.set_stringn(x, y, " ⦁", end.saturating_sub(x) as usize, dot_style);
            }
        }
    }
}
