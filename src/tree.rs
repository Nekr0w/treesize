use std::path::{Path, PathBuf};

const SIZE_UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanState {
    /// Children not yet listed.
    NotScanned,
    /// Children have been listed (sizes may still be computing).
    Scanned,
}

/// A node in the file tree, representing either a file or a directory.
#[derive(Debug, Clone)]
pub struct FileNode {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub children: Vec<FileNode>,
    pub error: Option<String>,
    pub scan_state: ScanState,
}

impl FileNode {
    pub fn new(path: PathBuf, size: u64, is_dir: bool) -> Self {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let scan_state = if is_dir {
            ScanState::NotScanned
        } else {
            ScanState::Scanned
        };
        Self {
            path,
            name,
            size,
            is_dir,
            children: Vec::new(),
            error: None,
            scan_state,
        }
    }

    /// Recursively sort all children by size, largest first.
    #[cfg(test)]
    pub fn sort_by_size(&mut self) {
        self.children.sort_unstable_by(|a, b| b.size.cmp(&a.size));
        for child in &mut self.children {
            child.sort_by_size();
        }
    }

    /// Find a node by path (immutable). Uses `starts_with` to prune branches.
    pub fn find(&self, target: &Path) -> Option<&FileNode> {
        if self.path == target {
            return Some(self);
        }
        for child in &self.children {
            if target.starts_with(&child.path) {
                if let Some(found) = child.find(target) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Find a node by path (mutable). Uses `starts_with` to prune branches.
    pub fn find_mut(&mut self, target: &Path) -> Option<&mut FileNode> {
        if self.path == target {
            return Some(self);
        }
        for child in &mut self.children {
            if target.starts_with(&child.path) {
                if let Some(found) = child.find_mut(target) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Remove a descendant by path, subtracting its size from each ancestor.
    /// Returns the removed node if found.
    pub fn remove_descendant(&mut self, target: &Path) -> Option<FileNode> {
        for i in 0..self.children.len() {
            if self.children[i].path == target {
                let removed = self.children.remove(i);
                self.size = self.size.saturating_sub(removed.size);
                self.children.sort_unstable_by(|a, b| b.size.cmp(&a.size));
                return Some(removed);
            }
            if target.starts_with(&self.children[i].path) {
                if let Some(removed) = self.children[i].remove_descendant(target) {
                    self.size = self.size.saturating_sub(removed.size);
                    self.children.sort_unstable_by(|a, b| b.size.cmp(&a.size));
                    return Some(removed);
                }
            }
        }
        None
    }

    /// Update a descendant's size and propagate the change up to the root.
    /// Re-sorts children at each affected level.
    pub fn update_descendant_size(&mut self, target: &Path, new_size: u64) -> bool {
        if self.path == target {
            self.size = new_size;
            return true;
        }
        for child in &mut self.children {
            if target.starts_with(&child.path) {
                if child.update_descendant_size(target, new_size) {
                    self.size = self.children.iter().map(|c| c.size).sum();
                    self.children.sort_unstable_by(|a, b| b.size.cmp(&a.size));
                    return true;
                }
            }
        }
        false
    }

    /// Recompute this node's size as the sum of its children (recursive).
    #[cfg(test)]
    pub fn recompute_size(&mut self) {
        if self.is_dir {
            for child in &mut self.children {
                child.recompute_size();
            }
            self.size = self.children.iter().map(|c| c.size).sum();
        }
    }

    /// Compute this node's percentage relative to a parent size.
    pub fn percentage_of(&self, parent_size: u64) -> f64 {
        if parent_size == 0 {
            return 0.0;
        }
        self.size as f64 / parent_size as f64 * 100.0
    }

}

/// Format a byte count into a human-readable string (e.g. "1.23 GB").
pub fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        return "0 B".to_string();
    }

    let mut value = bytes as f64;
    for &unit in SIZE_UNITS {
        if value < 1024.0 {
            return if unit == "B" {
                format!("{value} {unit}")
            } else {
                format!("{value:.2} {unit}")
            };
        }
        value /= 1024.0;
    }

    format!("{value:.2} TB")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(1024), "1.00 KB");
        assert_eq!(format_size(1536), "1.50 KB");
        assert_eq!(format_size(1_048_576), "1.00 MB");
        assert_eq!(format_size(1_073_741_824), "1.00 GB");
        assert_eq!(format_size(1_099_511_627_776), "1.00 TB");
    }

    #[test]
    fn test_sort_by_size() {
        let mut root = FileNode::new(PathBuf::from("/root"), 0, true);
        root.children = vec![
            FileNode::new(PathBuf::from("/root/small"), 100, false),
            FileNode::new(PathBuf::from("/root/large"), 9999, false),
            FileNode::new(PathBuf::from("/root/medium"), 500, false),
        ];
        root.sort_by_size();
        assert_eq!(root.children[0].size, 9999);
        assert_eq!(root.children[1].size, 500);
        assert_eq!(root.children[2].size, 100);
    }

    #[test]
    fn test_remove_descendant_propagates_size() {
        let mut root = FileNode::new(PathBuf::from("/root"), 0, true);
        root.children = vec![
            FileNode::new(PathBuf::from("/root/a"), 100, false),
            FileNode::new(PathBuf::from("/root/b"), 200, false),
        ];
        root.recompute_size();
        assert_eq!(root.size, 300);

        root.remove_descendant(Path::new("/root/a"));
        assert_eq!(root.size, 200);
        assert_eq!(root.children.len(), 1);
    }

    #[test]
    fn test_update_descendant_size() {
        let mut root = FileNode::new(PathBuf::from("/root"), 0, true);
        let mut dir = FileNode::new(PathBuf::from("/root/dir"), 0, true);
        dir.children = vec![
            FileNode::new(PathBuf::from("/root/dir/file"), 50, false),
        ];
        dir.recompute_size();
        root.children = vec![dir];
        root.recompute_size();
        assert_eq!(root.size, 50);

        // Simulate a size computation result for /root/dir
        root.update_descendant_size(Path::new("/root/dir"), 500);
        assert_eq!(root.children[0].size, 500);
        assert_eq!(root.size, 500); // Propagated up
    }

    #[test]
    fn test_find() {
        let mut root = FileNode::new(PathBuf::from("/root"), 0, true);
        let mut dir = FileNode::new(PathBuf::from("/root/dir"), 0, true);
        dir.children = vec![FileNode::new(PathBuf::from("/root/dir/file"), 10, false)];
        root.children = vec![dir];

        assert!(root.find(Path::new("/root/dir/file")).is_some());
        assert!(root.find(Path::new("/root/nonexistent")).is_none());
    }
}
