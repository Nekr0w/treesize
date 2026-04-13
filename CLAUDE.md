# CLAUDE.md

## Project overview

TreeSize TUI -- a Rust terminal disk space analyzer. Scans directories, displays an interactive tree sorted by size, supports deletion and rescan.

## Build & run

```bash
cargo build --release          # release binary at target/release/treesize
cargo run -- /path/to/scan     # dev mode
cargo test                     # unit tests in tree.rs
```

## Architecture

Message-driven architecture: all user actions are translated to `Message` variants, processed by a single `App::update()` method.

### Module responsibilities (strict separation)

| Module | Responsibility | Does NOT do |
|--------|---------------|-------------|
| `tree.rs` | `FileNode` struct, `format_size()`, tree operations (`find`, `find_mut`, `remove_descendant`, `update_descendant_size`) | No I/O, no rendering |
| `scanner.rs` | `ScanManager` (priority queue + 2 worker threads), `list_children()` (sync read_dir), `process_scan()` (jwalk walk), `ScanResult` enum | No state mutation, no rendering |
| `app.rs` | `App` state, `Message` enum, `update()` dispatcher, scan result handlers, expand/rescan/delete logic | No rendering, no direct I/O (delegates to scanner) |
| `ui.rs` | All rendering: tree→TreeItem conversion, header/footer/dialog, `size_color()`, `render_size_bar()`, disk usage via `statvfs` | No state mutation |
| `keys.rs` | Single `handle_key(KeyEvent, AppMode) → Message` function | Nothing else |
| `main.rs` | Terminal setup, event loop with 50ms poll | No business logic |

### Key design decisions

- **Synchronous expand**: pressing Enter on a directory calls `scanner::list_children()` on the main thread (read_dir is <1ms). Background scan only computes sizes.
- **Single-walk size computation**: one `jwalk::WalkDir` per scanned directory distributes file sizes to all children simultaneously via `immediate_child_of()`, with periodic flushes every 300ms.
- **No pre-fetch cascade**: scans are submitted only by explicit user actions (expand, rescan, initial load). No automatic pre-fetching of sibling directories.
- **Tree path optimization**: `find()` and `find_mut()` use `Path::starts_with()` to prune branches, avoiding O(n) full-tree scans.

### Scan flow

1. User expands dir → sync `list_children()` → children appear instantly
2. `scanner.submit(path, High)` → worker picks up request
3. Worker sends `ChildrenListed` (merge with existing), then streams `ChildSizeComputed` every 300ms, then `ScanComplete`
4. `App::poll_scan()` processes results: `update_descendant_size()` propagates sizes up and re-sorts at each level

### Data flow

```
KeyEvent → keys::handle_key() → Message → App::update() → state mutation
                                                         → scanner.submit()
Scanner worker → ScanResult → App::poll_scan() → tree mutation
App state → ui::render() → Frame
```

## Code conventions

- No logic duplication -- each piece of logic exists in exactly one place
- `format_size()` is the single size formatter, used everywhere
- `size_color()` is the single color-by-size function
- `build_display_line()` is the single node-to-text composer
- Tree mutations always propagate sizes to ancestors and re-sort

## Testing

```bash
cargo test    # runs 5 unit tests in tree.rs
```

Tests cover: `format_size`, `sort_by_size`, `remove_descendant` with size propagation, `update_descendant_size` with propagation, `find`.

## Platform

macOS and Linux (uses `libc::statvfs` for disk stats). The `crossterm` backend is cross-platform.
