use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf, MAIN_SEPARATOR};

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
use crate::ui::sidebar::{self, GitStatus, SidebarState};
use crate::ui::{completers, icons, EditorView, Prompt, PromptEvent};

const LEFT_PADDING: u16 = 1;

#[derive(Debug)]
struct DirEntry {
    path: PathBuf,
    is_dir: bool,
    ignored: bool,
}

#[derive(Debug)]
struct TreeNode {
    path: PathBuf,
    depth: usize,
    is_dir: bool,
    expanded: bool,
    ignored: bool,
}

struct NodeStyles {
    text: Style,
    dir: Style,
    ignored: Style,
    ignored_fg: Color,
    selected: Style,
}

impl NodeStyles {
    fn from_theme(theme: &Theme, focused: bool) -> Self {
        let ignored_fg = Color::Rgb(0x9a, 0xa5, 0xb1);

        Self {
            text: theme.get("ui.text"),
            dir: theme.get("ui.text.focus"),
            ignored: Style::default()
                .fg(ignored_fg)
                .add_modifier(Modifier::ITALIC),
            ignored_fg,
            selected: if focused {
                theme.get("ui.menu.selected")
            } else {
                theme.get("ui.cursorline.primary")
            },
        }
    }
}

#[derive(Debug, Default)]
pub struct ExplorerSidebar {
    state: SidebarState,
    root: PathBuf,
    nodes: Vec<TreeNode>,
    git_status: HashMap<PathBuf, GitStatus>,
    clipboard: Option<PathBuf>,
}

impl ExplorerSidebar {
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

    pub fn focus(&mut self, current_file: Option<PathBuf>, editor: &Editor) {
        self.open_and_focus(current_file, editor);
    }

    pub fn refresh_if_open(&mut self, editor: &Editor) {
        if !self.state.is_open() {
            return;
        }
        self.reload();
        self.refresh_git_status(editor);
    }

    fn open_and_focus(&mut self, current_file: Option<PathBuf>, editor: &Editor) {
        let root = helix_stdx::path::canonicalize(helix_loader::find_workspace().0);
        if self.nodes.is_empty() || self.root != root {
            self.root = root;
            self.reload();
        }
        self.refresh_git_status(editor);

        self.state.open_focused();

        if let Some(file) = current_file {
            self.reveal(&file);
        }
    }

    fn refresh_git_status(&mut self, editor: &Editor) {
        self.git_status.clear();

        let workspace = helix_loader::find_workspace_in(&self.root).0;
        let trust_full = sidebar::is_git_trusted(editor, &workspace);

        let Ok(changes) = editor.diff_providers.changed_files(&self.root, trust_full) else {
            return;
        };

        for change in changes {
            let status = GitStatus::from_change(&change);
            let path = helix_stdx::path::canonicalize(change.path());
            Self::mark_status(&mut self.git_status, path.clone(), status);
            self.mark_ancestors(&path, status);
        }
    }

    fn mark_ancestors(&mut self, path: &Path, status: GitStatus) {
        let mut current = path;
        while let Some(parent) = current.parent() {
            if parent == self.root || !parent.starts_with(&self.root) {
                break;
            }
            Self::mark_status(&mut self.git_status, parent.to_path_buf(), status);
            current = parent;
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
                self.state.selected = index;
                return;
            }
            if self.nodes[index].is_dir && !self.nodes[index].expanded {
                self.expand(index);
            }
        }
    }

    fn read_dir(dir: &Path, parent_ignored: bool) -> Vec<DirEntry> {
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

        let visible = if parent_ignored {
            HashSet::new()
        } else {
            Self::visible_paths(dir)
        };

        entries
            .into_iter()
            .map(|(path, is_dir)| {
                let ignored = parent_ignored || !visible.contains(&path);
                DirEntry {
                    path,
                    is_dir,
                    ignored,
                }
            })
            .collect()
    }

    fn visible_paths(dir: &Path) -> HashSet<PathBuf> {
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
        let selected_path = self
            .nodes
            .get(self.state.selected)
            .map(|node| node.path.clone());

        let root = self.root.clone();
        self.nodes = Self::read_dir(&root, false)
            .into_iter()
            .map(|entry| entry.into_node(0))
            .collect();

        let mut index = 0;
        while index < self.nodes.len() {
            let node = &self.nodes[index];
            if node.is_dir && !node.expanded && expanded.contains(&node.path) {
                self.expand(index);
            }
            index += 1;
        }

        self.state.selected = selected_path
            .and_then(|path| self.nodes.iter().position(|node| node.path == path))
            .unwrap_or(0);
        self.state.scroll = 0;
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
            .map(|entry| entry.into_node(depth + 1))
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
        self.state.selected = self.state.selected.min(self.nodes.len().saturating_sub(1));
        self.state.scroll = 0;
    }

    fn activate(&mut self, editor: &mut Editor) {
        let Some(node) = self.nodes.get(self.state.selected) else {
            return;
        };

        if node.is_dir {
            if node.expanded {
                self.collapse(self.state.selected);
            } else {
                self.expand(self.state.selected);
            }
        } else {
            let path = node.path.clone();
            if let Err(err) = editor.open(&path, Action::Replace) {
                editor.set_error(format!("Failed to open {}: {}", path.display(), err));
            } else {
                self.state.unfocus();
            }
        }
    }

    pub fn handle_key(&mut self, event: KeyEvent, editor: &mut Editor) -> EventResult {
        if event.modifiers.contains(KeyModifiers::CONTROL) {
            return EventResult::Ignored(None);
        }

        let keys = editor.config().sidebar.file_explorer.clone();

        if event == keys.create {
            return EventResult::Consumed(Some(Self::create_prompt(self.target_dir())));
        }
        if event == keys.rename {
            if let Some(node) = self.nodes.get(self.state.selected) {
                return EventResult::Consumed(Some(Self::rename_prompt(node.path.clone())));
            }
            return EventResult::Consumed(None);
        }
        if event == keys.delete {
            if let Some(node) = self.nodes.get(self.state.selected) {
                return EventResult::Consumed(Some(Self::delete_prompt(node.path.clone())));
            }
            return EventResult::Consumed(None);
        }
        if event == keys.copy {
            self.copy_selection(editor);
            return EventResult::Consumed(None);
        }
        if event == keys.paste {
            self.paste(editor);
            return EventResult::Consumed(None);
        }
        if event == keys.yank_name {
            self.yank_name(editor);
            return EventResult::Consumed(None);
        }
        if event == keys.search {
            let search_root = match self.nodes.get(self.state.selected) {
                Some(node) => node.path.clone(),
                None => self.root.clone(),
            };
            return EventResult::Consumed(Some(Self::global_search_callback(search_root)));
        }
        if event == keys.collapse_all {
            self.collapse_all();
            return EventResult::Consumed(None);
        }
        if event == keys.reload {
            self.reload();
            self.refresh_git_status(editor);
            return EventResult::Consumed(None);
        }

        match event.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.state.move_selection(1, self.nodes.len());
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.state.move_selection(-1, self.nodes.len());
            }
            KeyCode::Char('l') | KeyCode::Enter | KeyCode::Right => self.activate(editor),
            KeyCode::Char('h') | KeyCode::Left => self.collapse_selected(),
            KeyCode::Char('q') | KeyCode::Esc => self.state.unfocus(),
            _ => {}
        }

        EventResult::Consumed(None)
    }

    fn collapse_selected(&mut self) {
        let Some(node) = self.nodes.get(self.state.selected) else {
            return;
        };

        if node.is_dir && node.expanded {
            self.collapse(self.state.selected);
            return;
        }

        if node.depth == 0 {
            return;
        }

        if let Some(parent) = self.nodes[..self.state.selected]
            .iter()
            .rposition(|n| n.depth < node.depth)
        {
            self.state.selected = parent;
            self.collapse(parent);
        }
    }

    fn target_dir(&self) -> PathBuf {
        match self.nodes.get(self.state.selected) {
            Some(node) if node.is_dir => node.path.clone(),
            Some(node) => node
                .path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.root.clone()),
            None => self.root.clone(),
        }
    }

    fn copy_selection(&mut self, editor: &mut Editor) {
        let Some(node) = self.nodes.get(self.state.selected) else {
            return;
        };
        let path = node.path.clone();
        let name = file_name(&path);
        self.clipboard = Some(path);
        editor.set_status(format!("Copied '{name}'"));
    }

    fn paste(&mut self, editor: &mut Editor) {
        let Some(source) = self.clipboard.clone() else {
            editor.set_error("Nothing to paste");
            return;
        };
        let Some(name) = source.file_name() else {
            return;
        };

        let dest = unique_destination(&self.target_dir(), name);
        match editor.copy_path(&source, &dest) {
            Ok(()) => self.reload_and_reveal(&dest, editor),
            Err(err) => editor.set_error(format!("Could not paste: {err}")),
        }
    }

    fn yank_name(&mut self, editor: &mut Editor) {
        let Some(node) = self.nodes.get(self.state.selected) else {
            return;
        };
        let name = file_name(&node.path);
        match editor.registers.write('+', vec![name.clone()]) {
            Ok(()) => editor.set_status(format!("Yanked '{name}'")),
            Err(err) => editor.set_error(err.to_string()),
        }
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
                        Err(err) => cx.editor.set_error(format!(
                            "Could not create {}: {}",
                            path.display(),
                            err
                        )),
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
                        Err(err) => cx.editor.set_error(format!(
                            "Could not delete {}: {}",
                            path.display(),
                            err
                        )),
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
                    if let Err(err) = ensure_parent_dir(&new_path) {
                        cx.editor.set_error(err);
                        return;
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
        surface.set_style(area, background);

        sidebar::draw_separator(area, surface, theme);

        let inner = area.clip_right(1);
        let height = inner.height as usize;
        let content_x = inner.x + LEFT_PADDING;
        let content_width = inner.width.saturating_sub(LEFT_PADDING);

        self.state.adjust_scroll(height);

        let styles = NodeStyles::from_theme(theme, self.state.is_focused());
        for (row, node) in self
            .nodes
            .iter()
            .enumerate()
            .skip(self.state.scroll)
            .take(height)
        {
            let y = inner.y + (row - self.state.scroll) as u16;
            self.render_node(
                row,
                node,
                y,
                area,
                content_x,
                content_width,
                surface,
                editor,
                &styles,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_node(
        &self,
        index: usize,
        node: &TreeNode,
        y: u16,
        area: Rect,
        content_x: u16,
        content_width: u16,
        surface: &mut Surface,
        editor: &Editor,
        styles: &NodeStyles,
    ) {
        let theme = &editor.theme;
        let git = self.git_status.get(&node.path).copied();
        let content_style = node_content_style(node, git, styles, theme);

        let selected = index == self.state.selected;
        let style = node_row_style(selected, node.ignored, git.is_some(), content_style, styles);

        if selected {
            surface.set_style(Rect::new(area.x, y, area.width, 1), styles.selected);
        }

        let name = node
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        let end = content_x + content_width;
        let indent = "  ".repeat(node.depth);
        let (x, _) = surface.set_stringn(content_x, y, &indent, content_width as usize, style);

        let (x, _) = render_icon(node, styles, x, y, end, surface);
        let (x, _) = surface.set_stringn(x, y, &name, end.saturating_sub(x) as usize, style);

        render_modified_dot(node, style, x, y, end, editor, surface);
    }
}

fn render_icon(
    node: &TreeNode,
    styles: &NodeStyles,
    x: u16,
    y: u16,
    end: u16,
    surface: &mut Surface,
) -> (u16, u16) {
    let (icon, icon_color) = if node.is_dir {
        let (glyph, _) = icons::folder_icon(node.expanded);
        (glyph, styles.dir.fg)
    } else {
        let (glyph, color) = icons::file_icon(&node.path);
        (glyph, Some(color))
    };

    let icon_style = if node.ignored {
        Style::default().fg(styles.ignored_fg)
    } else {
        Style::default().fg(icon_color.unwrap_or(Color::Reset))
    };

    let glyph = format!("{icon} ");
    surface.set_stringn(x, y, &glyph, end.saturating_sub(x) as usize, icon_style)
}

fn render_modified_dot(
    node: &TreeNode,
    style: Style,
    x: u16,
    y: u16,
    end: u16,
    editor: &Editor,
    surface: &mut Surface,
) {
    let is_modified = !node.is_dir
        && editor
            .document_by_path(&node.path)
            .is_some_and(|doc| doc.is_modified());
    if is_modified {
        let dot_style = style.patch(editor.theme.get("keyword"));
        surface.set_stringn(x, y, " ⦁", end.saturating_sub(x) as usize, dot_style);
    }
}

impl DirEntry {
    fn into_node(self, depth: usize) -> TreeNode {
        TreeNode {
            path: self.path,
            depth,
            is_dir: self.is_dir,
            expanded: false,
            ignored: self.ignored,
        }
    }
}

fn node_content_style(
    node: &TreeNode,
    git: Option<GitStatus>,
    styles: &NodeStyles,
    theme: &Theme,
) -> Style {
    if node.ignored {
        return styles.ignored;
    }

    match git {
        Some(status) => status.style(theme),
        None if node.is_dir => styles.dir,
        None => styles.text,
    }
}

fn node_row_style(
    selected: bool,
    ignored: bool,
    has_git: bool,
    content_style: Style,
    styles: &NodeStyles,
) -> Style {
    if !selected {
        return content_style;
    }
    if !ignored && !has_git {
        return styles.selected;
    }

    let mut style = styles.selected;
    style.fg = content_style.fg;
    if ignored {
        style = style.add_modifier(Modifier::ITALIC);
    }
    style
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn unique_destination(dir: &Path, name: &OsStr) -> PathBuf {
    let base = dir.join(name);
    if !base.exists() {
        return base;
    }

    let name = Path::new(name);
    let stem = name
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let extension = name
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned());

    let mut counter = 1;
    loop {
        let suffix = if counter == 1 {
            format!("{stem} copy")
        } else {
            format!("{stem} copy {counter}")
        };
        let file_name = match &extension {
            Some(extension) => format!("{suffix}.{extension}"),
            None => suffix,
        };
        let candidate = dir.join(file_name);
        if !candidate.exists() {
            return candidate;
        }
        counter += 1;
    }
}

fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() || parent.exists() {
        return Ok(());
    }

    std::fs::create_dir_all(parent)
        .map_err(|err| format!("Could not create {}: {}", parent.display(), err))
}
