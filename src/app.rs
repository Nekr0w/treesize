use std::collections::HashMap;
use std::path::PathBuf;

use tui_tree_widget::TreeState;

use crate::scanner::{self, ScanManager, ScanStatus};
use crate::store;
use crate::tree::FileNode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppMode {
    Browsing,
    ConfirmDelete,
}

#[derive(Debug)]
pub enum Message {
    // Navigation
    MoveUp,
    MoveDown,
    ExpandOrEnter,
    CollapseOrBack,
    PageUp,
    PageDown,
    GoToFirst,
    GoToLast,

    // Actions
    Rescan,
    SaveScan,
    RequestDelete,
    ConfirmDelete,
    CancelDelete,

    // Lifecycle
    Quit,
    None,
}

pub struct App {
    pub mode: AppMode,
    pub root: Option<FileNode>,
    pub tree_state: TreeState<String>,
    pub status_message: Option<String>,
    pub target_path: PathBuf,
    pub delete_target: Option<(PathBuf, u64, bool)>,
    pub scanner: ScanManager,
    /// Total size of every directory seen so far, built by the background walk.
    /// Lets expand be an instant lookup instead of a re-scan.
    pub size_cache: HashMap<PathBuf, u64>,
    /// Which directories are queued / actively scanning (for per-item markers).
    pub scan_status: ScanStatus,
}

impl App {
    pub fn new(target_path: PathBuf) -> Self {
        Self {
            mode: AppMode::Browsing,
            root: None,
            tree_state: TreeState::default(),
            status_message: None,
            target_path,
            delete_target: None,
            scanner: ScanManager::new(),
            size_cache: HashMap::new(),
            scan_status: ScanStatus::default(),
        }
    }

    /// Kick off the initial scan: synchronous list for instant display,
    /// then background scan for sizes.
    pub fn start_scan(&mut self) {
        let mut root = FileNode::new(self.target_path.clone(), 0, true);

        // Synchronous read_dir → first level appears instantly
        let children = scanner::list_children(&self.target_path);
        root.children = children;
        root.scanned = true;
        root.size = root.children.iter().map(|c| c.size).sum();
        root.children.sort_unstable_by(|a, b| b.size.cmp(&a.size));

        // Submit each top-level directory as its own job so they scan in
        // parallel across workers (instead of one serial walk of the root).
        let dir_paths: Vec<PathBuf> = root
            .children
            .iter()
            .filter(|c| c.is_dir)
            .map(|c| c.path.clone())
            .collect();

        self.root = Some(root);

        for path in dir_paths {
            self.scanner.submit(path);
        }
    }

    /// Open a previously saved scan: structure from disk, sizes from the saved
    /// cache. No background scan — everything is known instantly.
    pub fn load_scan(&mut self, cache: HashMap<PathBuf, u64>) {
        self.size_cache = cache;

        let mut root = FileNode::new(self.target_path.clone(), 0, true);
        root.children = scanner::list_children(&self.target_path);
        root.scanned = true;
        self.root = Some(root);

        if let Some(root) = &mut self.root {
            root.apply_sizes(&self.size_cache);
        }
    }

    /// Save the current size cache so it can be reopened with `--load`.
    fn save_scan(&mut self) {
        let name = self
            .target_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "root".to_string());
        let file = std::env::current_dir()
            .unwrap_or_default()
            .join(format!("{name}.treesize"));

        let partial = if self.scanner.pending_count() > 0 {
            " (partial — scan still running)"
        } else {
            ""
        };

        self.status_message = Some(match store::save(&file, &self.target_path, &self.size_cache) {
            Ok(()) => format!("Saved to {}{partial}", file.display()),
            Err(e) => format!("Save failed: {e}"),
        });
    }

    /// Poll the scanner for results (non-blocking). Called every tick.
    pub fn poll_scan(&mut self) {
        let mut got_sizes = false;
        while let Ok(result) = self.scanner.result_rx.try_recv() {
            self.size_cache.extend(result.sizes);
            got_sizes = true;
        }
        if got_sizes {
            if let Some(root) = &mut self.root {
                root.apply_sizes(&self.size_cache);
            }
        }
        // Refresh queued/active markers every tick (jobs start & finish on
        // worker threads, independent of whether sizes arrived this tick).
        self.scan_status = self.scanner.status();
    }

    /// Process a message and mutate state. Returns true if the app should quit.
    pub fn update(&mut self, msg: Message) -> bool {
        match msg {
            Message::None => {}

            // Navigation
            Message::MoveUp => { self.tree_state.key_up(); }
            Message::MoveDown => { self.tree_state.key_down(); }
            Message::ExpandOrEnter => {
                self.trigger_scan_on_expand();
                self.tree_state.toggle_selected();
            }
            Message::CollapseOrBack => { self.tree_state.key_left(); }
            Message::PageUp => { self.tree_state.scroll_up(10); }
            Message::PageDown => { self.tree_state.scroll_down(10); }
            Message::GoToFirst => { self.tree_state.select_first(); }
            Message::GoToLast => { self.tree_state.select_last(); }

            // Actions
            Message::Rescan => self.rescan_selected(),
            Message::SaveScan => self.save_scan(),
            Message::RequestDelete => self.request_delete(),
            Message::ConfirmDelete => self.confirm_delete(),
            Message::CancelDelete => {
                self.mode = AppMode::Browsing;
                self.delete_target = None;
            }

            Message::Quit => return true,
        }
        false
    }

    // ── Expand with synchronous read_dir ──────────────────────────

    /// When the user expands a directory, list children synchronously
    /// for instant display, then submit background scan for sizes.
    fn trigger_scan_on_expand(&mut self) {
        let selected = self.tree_state.selected();
        if selected.is_empty() {
            return;
        }

        let path = PathBuf::from(&selected[selected.len() - 1]);

        let needs_scan = self
            .root
            .as_ref()
            .and_then(|r| r.find(&path))
            .is_some_and(|n| n.is_dir && !n.scanned);

        if needs_scan {
            // Structure is a synchronous read_dir (instant). Sizes come from the
            // single background walk's cache — whatever it has computed so far,
            // filling in live from the leaves up. No per-expand re-scan.
            let mut children = scanner::list_children(&path);
            for c in &mut children {
                if c.is_dir {
                    if let Some(&size) = self.size_cache.get(&c.path) {
                        c.size = size;
                    }
                }
            }

            if let Some(root) = &mut self.root {
                if let Some(node) = root.find_mut(&path) {
                    node.children = children;
                    node.scanned = true;
                    // Keep the node's own (already-computed) total — never
                    // collapse it to the sum of not-yet-scanned children.
                    if let Some(&total) = self.size_cache.get(&path) {
                        node.size = total;
                    }
                    node.children.sort_unstable_by(|a, b| b.size.cmp(&a.size));
                }
            }
        }
    }

    // ── Rescan ────────────────────────────────────────────────────

    /// Force rescan the selected directory: re-list children and recompute sizes.
    fn rescan_selected(&mut self) {
        let selected = self.tree_state.selected();
        if selected.is_empty() {
            return;
        }

        let path = PathBuf::from(&selected[selected.len() - 1]);

        let is_dir = self
            .root
            .as_ref()
            .and_then(|r| r.find(&path))
            .is_some_and(|n| n.is_dir);

        if !is_dir {
            return;
        }

        // Synchronous re-list for instant feedback
        let children = scanner::list_children(&path);

        if let Some(root) = &mut self.root {
            if let Some(node) = root.find_mut(&path) {
                node.children = children;
                node.scanned = true;
                node.size = node.children.iter().map(|c| c.size).sum();
                node.children.sort_unstable_by(|a, b| b.size.cmp(&a.size));
            }
        }

        // Background: recompute sizes
        self.scanner.submit(path);
        self.status_message = Some("Rescanning...".to_string());
    }

    // ── Delete ────────────────────────────────────────────────────

    fn request_delete(&mut self) {
        let selected = self.tree_state.selected();
        if selected.is_empty() {
            return;
        }

        let path = PathBuf::from(&selected[selected.len() - 1]);

        if path == self.target_path {
            self.status_message = Some("Cannot delete the root directory".to_string());
            return;
        }

        if let Some(root) = &self.root {
            if let Some(node) = root.find(&path) {
                self.delete_target = Some((path, node.size, node.is_dir));
                self.mode = AppMode::ConfirmDelete;
            }
        }
    }

    fn confirm_delete(&mut self) {
        let Some((path, size, is_dir)) = self.delete_target.take() else {
            self.mode = AppMode::Browsing;
            return;
        };

        let result = if is_dir {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };

        match result {
            Ok(()) => {
                if let Some(root) = &mut self.root {
                    root.remove_descendant(&path);
                }
                // Keep cached ancestor totals accurate so re-expanding a parent
                // doesn't resurrect the deleted size from the cache.
                let mut ancestor = path.parent();
                while let Some(dir) = ancestor {
                    if let Some(v) = self.size_cache.get_mut(dir) {
                        *v = v.saturating_sub(size);
                    }
                    if dir == self.target_path {
                        break;
                    }
                    ancestor = dir.parent();
                }
                self.size_cache.remove(&path);
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                self.status_message = Some(format!("Deleted: {name}"));
            }
            Err(e) => {
                self.status_message = Some(format!("Error: {e}"));
            }
        }

        self.mode = AppMode::Browsing;
    }
}
