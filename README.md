# Helix chad

A fork of [Helix](https://github.com/helix-editor/helix) — a nod to [NvChad](https://nvchad.com/), bringing its IDE-like, batteries-included feel (file tree, git integration, a polished UI) to the modal terminal editor. Everything under the hood is still plain Helix — see the [Helix docs](https://docs.helix-editor.com/).

> **Note:** icons require a [Nerd Font](https://www.nerdfonts.com/) in your terminal. Without one they show up as tofu (□).

## Features

### File explorer sidebar

An nvim-tree style tree browser.

- Nerd Font folder and file-type icons.
- Git status colors per file, propagated up to parent folders (VSCode style).
- Ignored files greyed and italic.
- Create, rename (auto-creates parent dirs) and delete files inline.
- Copy / paste files and folders, and yank the selected entry name to the clipboard.
- Scoped search — `Space /` narrowed to the selected folder (recursive) or file.
- Reveals and expands to the current file when opened.

<img src="./contrib/sidebar-explorer.png" alt="File explorer" width="700">

### Git changes sidebar

A Zed style list of changed files.

- A `Staged` group plus Added / Modified / Deleted groups for the unstaged changes, each with counts.
- Single-child folder chains collapsed into one line (`src/routes/api.export`).
- Stage / unstage and discard a change directly from the sidebar.
- Enter opens an added or deleted file directly, and a modified file in a side-by-side diff view.

<img src="./contrib/sidebar-git-change.png" alt="Git changes" width="700">

### Diff view

A VS Code style side-by-side diff, opened as its own read-only buffer when you press Enter on a **modified** file in the git changes sidebar (added and deleted files just open normally).

- Two panes: the committed version (`HEAD`) on the left with removed lines tinted red, the working tree on the right with added lines tinted green.
- Only the changed regions are shown, with 5 lines of context around each; the rest is collapsed behind a separator line.
- Full tree-sitter syntax highlighting on both sides.
- Appears in the bufferline as `<file> - (working tree)` and behaves like any buffer — focus it, switch away, close it.
- Scroll with `j` / `k`, `Ctrl-d` / `Ctrl-u`, `PageUp` / `PageDown`. Press `g Space` to jump to the real file.
- The line-background tints are themeable. By default they are derived from the theme's `diff.plus` / `diff.minus` colors blended into the background, so they stay light on light themes and dark on dark themes.

| Side    | Theme key         |
| ------- | ----------------- |
| Added   | `ui.diff.added`   |
| Deleted | `ui.diff.deleted` |

```toml
# in your theme
"ui.diff.added" = { bg = "#e6ffed" }
"ui.diff.deleted" = { bg = "#ffeef0" }
```

### Git status colors

- Applied to explorer files and folders, git changes rows, and buffer titles in the bufferline.
- The `[+]` modified marker becomes a colored dot (`⦁`) in the statusline and bufferline.
- Themeable, with these defaults:

| Status   | Theme key                  | Color     |
| -------- | -------------------------- | --------- |
| Added    | `version_control.added`    | `#27A657` |
| Modified | `version_control.modified` | `#D3B020` |
| Deleted  | `version_control.deleted`  | `#E06C76` |

### Inline git blame

A VSCode / Zed style blame annotation at the end of the line under the cursor.

- Shows who last changed the line, when, and the commit message: `Noé Pigeau, 2 weeks ago • feat: Update read me`.
- Hidden on lines with uncommitted changes.
- Computed in the background and refreshed when a document is opened or saved.
- Disabled by default — enable it from your `config.toml`, or toggle it at runtime with `:toggle inline-blame.enable`.
- The format is configurable with the `{author}`, `{time-ago}`, `{message}` and `{commit}` placeholders.
- Themeable through the `ui.virtual.inline-blame` key, falling back to a grey foreground (`#7A818A`).
- `Space B c` copies the URL of the commit that last changed the current line, and `Space B p` the URL of the pull request that introduced it. Both work on GitHub, GitLab and Bitbucket remotes, follow the blame of the line under the cursor, and don't require the annotation to be enabled.
- The pull request is resolved through the [`gh` CLI](https://cli.github.com/) when available (covers squash, merge and rebase merges, and links to the right repo for commits coming from a fork's upstream), and falls back to the `#123` / `!123` references found in the commit or merge-commit message.

```toml
[editor.inline-blame]
enable = true
format = "{author}, {time-ago} • {message}"
```

```toml
# in your theme
"ui.virtual.inline-blame" = { fg = "#7A818A", modifiers = ["italic"] }
```

### Polished UI

- Floating, rounded search box for `/`.
- Command line (`:`) as a floating box with completion and documentation popups.
- Pickers with rounded borders and centered titles.
- Improved UI for Rename symbol
- Extra theme: `zed_one_light_v2`.

## Keybindings

| Default key | Action                                                                            | Command                 |
| ----------- | --------------------------------------------------------------------------------- | ----------------------- |
| `Ctrl-e`    | Toggle the sidebar (opens the file explorer, or closes whichever sidebar is open) | `toggle_sidebar`        |
| `Space e`   | Focus the file explorer on the current file                                       | `focus_file_explorer`   |
| `Space g`   | Focus the git changes sidebar                                                     | `focus_changes_sidebar` |
| `g Space`   | Open the real file from the diff view                                             | `goto_diff_view_file`   |
| `Ctrl-→`    | Widen the focused sidebar                                                         | `widen_sidebar`         |
| `Ctrl-←`    | Narrow the focused sidebar                                                        | `narrow_sidebar`        |
| `Space B c` | Copy the URL of the commit that last changed the current line                     | `copy_blame_commit_url` |
| `Space B p` | Copy the URL of the pull request that introduced the commit of the current line  | `copy_blame_pull_request_url` |

These are regular defaults — rebind any of them from your `config.toml` using the command name:

```toml
[keys.normal]
C-e = "toggle_sidebar"
space.e = "focus_file_explorer"
space.g = "focus_changes_sidebar"
C-right = "widen_sidebar"
C-left = "narrow_sidebar"

[keys.normal.g]
space = "goto_diff_view_file"
```

Inside a sidebar:

| Default key     | Action                                   |
| --------------- | ---------------------------------------- |
| `j` / `k`       | Move up / down                           |
| `gg` / `ge`     | Go to first / last entry                 |
| `Ctrl-d` / `Ctrl-u` | Scroll half a page down / up         |
| `l` / `Enter`   | Expand folder or open file               |
| `h`             | Collapse / go to parent                  |
| `a` / `r` / `d` | Create / rename / delete (file explorer) |
| `c` / `p`       | Copy / paste (file explorer)             |
| `y`             | Yank the entry name (file explorer)      |
| `s` / `d`       | Stage-unstage / discard (git changes)    |
| `/`             | Scoped search (file explorer)            |
| `R`             | Reload                                   |
| `W`             | Collapse all folders.                    |
| `q` / `Esc`     | Return focus to the editor               |

`Space` and `:` still work while a sidebar is focused, so you can switch between sidebars or run any command without leaving.

The navigation keys (`gg`, `ge`, `Ctrl-d`, `Ctrl-u`) reuse your editor keymap, so rebinding `goto_file_start`, `goto_last_line`, `page_cursor_half_down`, or `page_cursor_half_up` updates the sidebar too.

The action keys inside each sidebar are configurable from your `config.toml` (defaults shown):

```toml
[editor.sidebar.file-explorer]
create = "a"
rename = "r"
delete = "d"
copy = "c"
paste = "p"
yank-name = "y"
search = "/"
collapse-all = "W"
reload = "R"

[editor.sidebar.git-changes]
stage = "s"
discard = "d"
reload = "R"
```

## Install

See the [Helix install docs](https://docs.helix-editor.com/install.html) for prerequisites.
