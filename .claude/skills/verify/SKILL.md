---
name: verify
description: Build and drive this Helix fork's TUI to verify editor changes end-to-end in an isolated tmux session.
---

# Verify helix-fork changes

Build the editor:

```bash
cargo build --bin hx
```

Launch in an isolated tmux with a throwaway config (XDG_CONFIG_HOME must point
to the parent of a `helix/` dir containing `config.toml`):

```bash
CFG=$(mktemp -d)/helix && mkdir -p "$CFG"
printf '[editor.inline-blame]\nenable = true\n' > "$CFG/config.toml"
tmux -L verify new-session -d -x 210 -y 50 \
  "env HELIX_RUNTIME=$PWD/runtime XDG_CONFIG_HOME=$(dirname $CFG) $PWD/target/debug/hx README.md"
```

Drive and observe:

```bash
tmux -L verify send-keys "14gg"                      # motions/commands as keys
tmux -L verify send-keys ":toggle some.option" Enter
tmux -L verify capture-pane -p | sed -n '14p;49,50p' # content + statusline/message
tmux -L verify kill-server
```

Gotchas:
- Wait ~4s after launch before sending keys; ~1-2s after async actions
  (blame, LSP) before capturing.
- Async results may need a redraw: send a cheap key (`j` then `k`).
- Files with huge git history (CHANGELOG.md) make git blame take tens of
  seconds in debug builds — probe vcs features on fork-touched files like
  README.md instead.
- README.md renders with icons; capture lines by number, don't match on
  exact text.
