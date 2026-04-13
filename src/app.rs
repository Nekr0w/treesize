use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tui_tree_widget::TreeState;

use crate::scanner::{self, ScanManager, ScanPriority, ScanResult};
use crate::tree::{FileNode, ScanState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppMode {
    Scanning,
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
    RequestDelete,
    ConfirmDelete,
    CancelDelete,

    // Lifecycle
    Quit,
    ForceQuit,
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
}

impl App {
    pub fn new(target_path: PathBuf) -> Self {
        Self {
            mode: AppMode::Scanning,
            root: None,
            tree_state: TreeState::default(),
            status_message: None,
            target_path,
            delete_target: None,
            scanner: ScanManager::new(),
        }
    }

    /// Kick off the initial scan: synchronous list for instant display,
    /// then background scan for sizes.
    pub fn start_scan(&mut self) {
        let mut root = FileNode::new(self.target_path.clone(), 0, true);

        // Synchronous read_dir → first level appears instantly
        let children = scanner::list_children(&self.target_path);
        root.children = children;
        root.scan_state = ScanState::Scanned;
        root.size = root.children.iter().map(|c| c.size).sum();
        root.children.sort_unstable_by(|a, b| b.size.cmp(&a.size));

        self.root = Some(root);
        self.mode = AppMode::Browsing;

        // Background: compute sizes via single jwalk walk
        self.scanner
            .submit(self.target_path.clone(), ScanPriority::High);
    }

    /// Poll the scanner for results (non-blocking). Called every tick.
    pub fn poll_scan(&mut self) {
        while let Ok(result) = self.scanner.result_rx.try_recv() {
            match result {
                ScanResult::ChildrenListed { parent, children } => {
                    self.handle_children_listed(&parent, children);
                }
                ScanResult::ChildSizeComputed { child, size } => {
                    self.handle_size_computed(&child, size);
                }
                ScanResult::ScanComplete { parent } => {
                    self.handle_scan_complete(&parent);
                }
            }
        }
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
            Message::RequestDelete => self.request_delete(),
            Message::ConfirmDelete => self.confirm_delete(),
            Message::CancelDelete => {
                self.mode = AppMode::Browsing;
                self.delete_target = None;
            }

            Message::Quit | Message::ForceQuit => return true,
        }
        false
    }

    // ── Scan result handlers ──────────────────────────────────────

    fn handle_children_listed(&mut self, parent_path: &Path, new_children: Vec<FileNode>) {
        if let Some(root) = &mut self.root {
            if let Some(parent) = root.find_mut(parent_path) {
                // Merge: preserve existing sub-trees and computed sizes
                let mut existing: HashMap<PathBuf, FileNode> = parent
                    .children
                    .drain(..)
                    .map(|c| (c.path.clone(), c))
                    .collect();

                parent.children = new_children
                    .into_iter()
                    .map(|mut new_child| {
                        if let Some(old) = existing.remove(&new_child.path) {
                            if new_child.is_dir {
                                // Preserve sub-tree, scan state, and computed size
                                if !old.children.is_empty() {
                                    new_child.children = old.children;
                                    new_child.scan_state = old.scan_state;
                                }
                                if old.size > 0 {
                                    new_child.size = old.size;
                                }
                            }
                        }
                        new_child
                    })
                    .collect();

                parent.scan_state = ScanState::Scanned;
                parent.size = parent.children.iter().map(|c| c.size).sum();
                parent.children.sort_unstable_by(|a, b| b.size.cmp(&a.size));
            }
        }
    }

    fn handle_size_computed(&mut self, child_path: &Path, size: u64) {
        if let Some(root) = &mut self.root {
            root.update_descendant_size(child_path, size);
        }
    }

    fn handle_scan_complete(&mut self, parent_path: &Path) {
        if let Some(root) = &mut self.root {
            if let Some(parent) = root.find_mut(parent_path) {
                parent.scan_state = ScanState::Scanned;
            }
        }
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
            .is_some_and(|n| n.is_dir && matches!(n.scan_state, ScanState::NotScanned));

        if needs_scan {
            // Synchronous read_dir → children appear instantly
            let children = scanner::list_children(&path);

            if let Some(root) = &mut self.root {
                if let Some(node) = root.find_mut(&path) {
                    node.children = children;
                    node.scan_state = ScanState::Scanned;
                    node.size = node.children.iter().map(|c| c.size).sum();
                    node.children.sort_unstable_by(|a, b| b.size.cmp(&a.size));
                }
            }

            // Background: compute children sizes
            self.scanner.submit(path, ScanPriority::High);
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
                node.scan_state = ScanState::Scanned;
                node.size = node.children.iter().map(|c| c.size).sum();
                node.children.sort_unstable_by(|a, b| b.size.cmp(&a.size));
            }
        }

        // Background: recompute sizes
        self.scanner.submit(path, ScanPriority::High);
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
        let Some((path, _size, is_dir)) = self.delete_target.take() else {
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
