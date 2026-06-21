//! Persist a scan's computed directory sizes so it can be reopened without
//! recomputing. Only the size cache is saved (the expensive part); structure is
//! re-listed from disk on load, which is instant.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Format: line 1 = root path, then one `<size>\t<dir-path>` per directory.
/// Size comes first so `split_once('\t')` keeps tabs inside paths intact.
/// ponytail: line-based, so a path containing a newline (legal but absurd on
/// macOS/Linux) would corrupt that entry. Switch to a length-prefixed format if
/// that ever matters.
pub fn save(file: &Path, root: &Path, cache: &HashMap<PathBuf, u64>) -> io::Result<()> {
    let mut out = String::with_capacity(cache.len() * 32);
    out.push_str(&root.to_string_lossy());
    out.push('\n');
    for (path, size) in cache {
        out.push_str(&size.to_string());
        out.push('\t');
        out.push_str(&path.to_string_lossy());
        out.push('\n');
    }
    fs::write(file, out)
}

pub fn load(file: &Path) -> io::Result<(PathBuf, HashMap<PathBuf, u64>)> {
    let content = fs::read_to_string(file)?;
    let mut lines = content.lines();
    let root = PathBuf::from(lines.next().unwrap_or_default());
    let mut cache = HashMap::new();
    for line in lines {
        if let Some((size, path)) = line.split_once('\t') {
            if let Ok(size) = size.parse() {
                cache.insert(PathBuf::from(path), size);
            }
        }
    }
    Ok((root, cache))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_load_roundtrip() {
        let file =
            std::env::temp_dir().join(format!("treesize_store_{}.scan", std::process::id()));
        let root = PathBuf::from("/Users/x");
        let mut cache = HashMap::new();
        cache.insert(PathBuf::from("/Users/x/a"), 100);
        cache.insert(PathBuf::from("/Users/x/weird\tname"), 200); // tab in path survives

        save(&file, &root, &cache).unwrap();
        let (loaded_root, loaded_cache) = load(&file).unwrap();

        assert_eq!(loaded_root, root);
        assert_eq!(loaded_cache, cache);
        std::fs::remove_file(&file).unwrap();
    }
}
