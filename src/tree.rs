use std::collections::HashMap;
use std::path::{Path, PathBuf};

const SIZE_UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];

/// A node in the file tree, representing either a file or a directory.
#[derive(Debug, Clone)]
pub struct FileNode {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub children: Vec<FileNode>,
    /// Whether children have been listed (sizes may still be computing).
    /// Files are scanned from birth; directories only once expanded.
    pub scanned: bool,
}

impl FileNode {
    pub fn new(path: PathBuf, size: u64, is_dir: bool) -> Self {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        Self {
            path,
            name,
            size,
            is_dir,
            children: Vec::new(),
            scanned: !is_dir,
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

    /// Apply precomputed directory totals from the size cache to every loaded
    /// node, then re-sort each level. Dirs prefer the cached total; if absent
    /// (e.g. cache not ready yet) they fall back to the sum of their children.
    pub fn apply_sizes(&mut self, cache: &HashMap<PathBuf, u64>) {
        for child in &mut self.children {
            child.apply_sizes(cache);
        }
        if self.is_dir {
            if let Some(&size) = cache.get(&self.path) {
                self.size = size;
            } else if !self.children.is_empty() {
                self.size = self.children.iter().map(|c| c.size).sum();
            }
            self.children.sort_unstable_by(|a, b| b.size.cmp(&a.size));
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
    fn test_apply_sizes_uses_cache_not_just_children() {
        // Regression: expanding a dir must NOT collapse its known total to the
        // sum of its (still-zero) immediate sub-dirs. The cache holds the truth.
        let mut root = FileNode::new(PathBuf::from("/root"), 0, true);
        let mut lib = FileNode::new(PathBuf::from("/root/lib"), 0, true);
        // lib was just expanded: a 0-size sub-dir + one small loose file.
        lib.children = vec![
            FileNode::new(PathBuf::from("/root/lib/sub"), 0, true),
            FileNode::new(PathBuf::from("/root/lib/f"), 10, false),
        ];
        root.children = vec![lib];

        let mut cache = HashMap::new();
        cache.insert(PathBuf::from("/root/lib"), 99_000);
        cache.insert(PathBuf::from("/root/lib/sub"), 98_990);

        root.apply_sizes(&cache);

        let lib = &root.children[0];
        assert_eq!(lib.size, 99_000); // from cache, not 10 (sum of children)
        assert_eq!(lib.children[0].size, 98_990); // sub got its cached total
        assert_eq!(root.size, 99_000); // propagated to root
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
