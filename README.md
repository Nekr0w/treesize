# TreeSize TUI

A fast, interactive disk space analyzer for the terminal. Built in Rust with [ratatui](https://github.com/ratatui/ratatui).

![treesize](https://img.shields.io/badge/rust-stable-orange) ![platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-blue)

## Features

- **Instant display** -- directory contents appear immediately via synchronous `read_dir`, sizes compute progressively in background
- **Interactive tree navigation** -- expand/collapse directories, vim-style keybindings
- **Smart scanning** -- single parallel walk (jwalk/rayon) computes all sibling sizes simultaneously with 300ms progressive flushes
- **Disk usage indicator** -- real-time filesystem usage in the header (used/total with percentage)
- **Delete files/directories** -- remove items directly from the TUI with confirmation dialog
- **Rescan** -- force refresh any directory after external changes
- **Lightweight** -- ~1.1 MB release binary

## Installation

```bash
git clone https://github.com/your-username/treesize-homemade.git
cd treesize-homemade
cargo build --release
# Binary at ./target/release/treesize
```

## Usage

```bash
# Scan current directory
treesize

# Scan a specific path
treesize /Users/nekr0w
treesize /
```

## Keybindings

| Key | Action |
|-----|--------|
| `j` / `↓` | Move down |
| `k` / `↑` | Move up |
| `l` / `Enter` / `→` | Expand directory |
| `h` / `Backspace` / `←` | Collapse directory |
| `g` / `Home` | Go to first item |
| `G` / `End` | Go to last item |
| `PgUp` / `PgDown` | Scroll by page |
| `r` | Rescan selected directory |
| `d` / `Delete` | Delete selected item |
| `q` / `Ctrl+C` | Quit |

## Architecture

```
src/
  main.rs      -- entry point, terminal setup, event loop (50ms poll)
  app.rs       -- application state, message-driven update logic
  tree.rs      -- FileNode data structure, size formatting, tree operations
  scanner.rs   -- background scan manager with priority queue and worker pool
  ui.rs        -- TUI rendering: tree view, header, footer, dialogs
  keys.rs      -- keyboard mapping (KeyEvent, AppMode) → Message
```

**Design principles:**
- Message-driven architecture -- all mutations go through `App::update(Message)`
- Single responsibility per module -- formatting in `tree.rs`, keys in `keys.rs`, rendering in `ui.rs`
- Synchronous expand + async sizes -- `read_dir` is instant, `jwalk` computes sizes in background
- No pre-fetch cascade -- scans are triggered only by user actions

## Dependencies

| Crate | Purpose |
|-------|---------|
| `ratatui` | TUI framework |
| `crossterm` | Terminal backend + keyboard events |
| `tui-tree-widget` | Tree widget with expand/collapse/selection |
| `jwalk` | Parallel directory walking (rayon-based) |
| `color-eyre` | Error handling |
| `libc` | Filesystem stats (statvfs) |

## License

MIT
