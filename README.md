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
- Enter opens the file and jumps to its first diff hunk.

<img src="./contrib/sidebar-git-change.png" alt="Git changes" width="700">

### Git status colors

- Applied to explorer files and folders, git changes rows, and buffer titles in the bufferline.
- The `[+]` modified marker becomes a colored dot (`⦁`) in the statusline and bufferline.
- Themeable, with these defaults:

| Status   | Theme key                  | Color     |
| -------- | -------------------------- | --------- |
| Added    | `version_control.added`    | `#27A657` |
| Modified | `version_control.modified` | `#D3B020` |
| Deleted  | `version_control.deleted`  | `#E06C76` |

### Polished UI

- Floating, rounded search box for `/`.
- Command line (`:`) as a floating box with completion and documentation popups.
- Pickers with rounded borders and centered titles.
- Extra theme: `zed_one_light_v2`.

## Keybindings

| Default key | Action                                                                            | Command                 |
| ----------- | --------------------------------------------------------------------------------- | ----------------------- |
| `Ctrl-e`    | Toggle the sidebar (opens the file explorer, or closes whichever sidebar is open) | `toggle_sidebar`        |
| `Space e`   | Focus the file explorer on the current file                                       | `focus_file_explorer`   |
| `Space g`   | Focus the git changes sidebar                                                     | `focus_changes_sidebar` |
| `Ctrl-→`    | Widen the focused sidebar                                                         | `widen_sidebar`         |
| `Ctrl-←`    | Narrow the focused sidebar                                                        | `narrow_sidebar`        |

These are regular defaults — rebind any of them from your `config.toml` using the command name:

```toml
[keys.normal]
C-e = "toggle_sidebar"
space.e = "focus_file_explorer"
space.g = "focus_changes_sidebar"
C-right = "widen_sidebar"
C-left = "narrow_sidebar"
```

Inside a sidebar:

| Default key     | Action                                   |
| --------------- | ---------------------------------------- |
| `j` / `k`       | Move up / down                           |
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
