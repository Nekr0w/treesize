use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::tree::FileNode;

// ── Results sent back to the main thread ──────────────────────────

/// Partial directory totals from a walk, streamed every ~300ms. Sizes only
/// grow, so later values overwrite earlier ones. Structure (which children
/// exist) is listed synchronously on the main thread, not here.
pub struct ScanResult {
    pub sizes: HashMap<PathBuf, u64>,
}

// ── Shared state between manager and workers ──────────────────────

struct QueueInner {
    queue: VecDeque<PathBuf>,
    pending: HashSet<PathBuf>,
    /// Subset of `pending` currently being walked by a worker (vs still queued).
    active: HashSet<PathBuf>,
}

/// A snapshot of what's queued and what's actively scanning, for the UI.
#[derive(Default)]
pub struct ScanStatus {
    pub queued: HashSet<PathBuf>,
    pub active: HashSet<PathBuf>,
}

struct SharedState {
    queue: Mutex<QueueInner>,
    condvar: Condvar,
    shutdown: AtomicBool,
}

// ── Public API ────────────────────────────────────────────────────

pub struct ScanManager {
    shared: Arc<SharedState>,
    pub result_rx: Receiver<ScanResult>,
}

impl ScanManager {
    pub fn new() -> Self {
        let (result_tx, result_rx) = std::sync::mpsc::channel();

        let shared = Arc::new(SharedState {
            queue: Mutex::new(QueueInner {
                queue: VecDeque::new(),
                pending: HashSet::new(),
                active: HashSet::new(),
            }),
            condvar: Condvar::new(),
            shutdown: AtomicBool::new(false),
        });

        // One worker per core: each drains one directory job, so top-level dirs
        // (Library, Pictures, Developer…) stream their sizes in parallel.
        // ponytail: scanning is I/O-bound, so core count is a fine default knob.
        let num_workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        for _ in 0..num_workers {
            let shared = Arc::clone(&shared);
            let tx = result_tx.clone();
            std::thread::spawn(move || worker_loop(&shared, &tx));
        }

        Self { shared, result_rx }
    }

    /// Submit a directory for scanning. Deduplicated: no-op if already queued.
    pub fn submit(&self, path: PathBuf) {
        let mut queue = self.shared.queue.lock().unwrap();
        if queue.pending.contains(&path) {
            return;
        }
        queue.pending.insert(path.clone());
        queue.queue.push_back(path);
        drop(queue);
        self.shared.condvar.notify_one();
    }

    /// Number of items queued or in-flight.
    pub fn pending_count(&self) -> usize {
        self.shared.queue.lock().unwrap().pending.len()
    }

    /// Snapshot of which directories are queued vs actively scanning.
    pub fn status(&self) -> ScanStatus {
        let q = self.shared.queue.lock().unwrap();
        ScanStatus {
            queued: q.queue.iter().cloned().collect(),
            active: q.active.clone(),
        }
    }
}

impl Drop for ScanManager {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, AtomicOrdering::Relaxed);
        self.shared.condvar.notify_all();
    }
}

// ── Worker ────────────────────────────────────────────────────────

fn worker_loop(shared: &SharedState, result_tx: &Sender<ScanResult>) {
    loop {
        // Wait for a request
        let path = {
            let mut queue = shared.queue.lock().unwrap();
            loop {
                if shared.shutdown.load(AtomicOrdering::Relaxed) {
                    return;
                }
                if let Some(path) = queue.queue.pop_front() {
                    queue.active.insert(path.clone());
                    break path;
                }
                queue = shared.condvar.wait(queue).unwrap();
            }
        };

        // Process the scan
        process_scan(&path, result_tx);

        // Done: drop from both the active and pending sets.
        let mut queue = shared.queue.lock().unwrap();
        queue.active.remove(&path);
        queue.pending.remove(&path);
    }
}

const FLUSH_INTERVAL: Duration = Duration::from_millis(300);

/// Walk `path` in ONE jwalk pass, accumulating the total size of every directory
/// underneath it (each file adds its size to all its ancestors — i.e. summed up
/// from the leaves). Partial totals stream back every ~300ms so sizes fill in
/// live. One of these runs per top-level directory, in parallel across workers.
fn process_scan(path: &Path, tx: &Sender<ScanResult>) {
    // ponytail: one HashMap entry per directory; tens of MB on a full home dir.
    // Spill to disk only if memory ever bites.
    let mut totals: HashMap<PathBuf, u64> = HashMap::new();
    let mut dirty: HashSet<PathBuf> = HashSet::new();
    let mut last_flush = Instant::now();

    // Serial walk: parallelism comes from running one job per worker thread, so
    // each directory gets its own walk. Sharing jwalk's global rayon pool here
    // would let one huge dir (e.g. ~/Library) starve every other job.
    let walk = jwalk::WalkDir::new(path)
        .skip_hidden(false)
        .parallelism(jwalk::Parallelism::Serial);
    for entry in walk {
        let Ok(entry) = entry else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            continue;
        }
        let size = meta.len();

        // Add this file's size to every ancestor directory, up to `path`.
        let entry_path = entry.path();
        let mut ancestor = entry_path.parent();
        while let Some(dir) = ancestor {
            *totals.entry(dir.to_path_buf()).or_insert(0) += size;
            dirty.insert(dir.to_path_buf());
            if dir == path {
                break;
            }
            ancestor = dir.parent();
        }

        if last_flush.elapsed() > FLUSH_INTERVAL {
            if flush_delta(&totals, &mut dirty, tx).is_err() {
                return;
            }
            last_flush = Instant::now();
        }
    }

    let _ = flush_delta(&totals, &mut dirty, tx);
}

// ── Scanning primitives ───────────────────────────────────────────

/// List immediate children of a directory using `read_dir` (fast, non-recursive).
/// Public so it can be called synchronously from the main thread for instant expand.
pub fn list_children(path: &Path) -> Vec<FileNode> {
    let Ok(entries) = std::fs::read_dir(path) else {
        return vec![];
    };

    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let file_type = entry.file_type().ok()?;

            // Skip symlinks to avoid loops and double-counting
            if file_type.is_symlink() {
                return None;
            }

            let entry_path = entry.path();
            let is_dir = file_type.is_dir();
            let size = if is_dir {
                0
            } else {
                entry.metadata().ok().map(|m| m.len()).unwrap_or(0)
            };

            Some(FileNode::new(entry_path, size, is_dir))
        })
        .collect()
}

/// Send the directories whose totals changed since the last flush, then clear
/// the dirty set. Returns Err if the channel is closed.
fn flush_delta(
    totals: &HashMap<PathBuf, u64>,
    dirty: &mut HashSet<PathBuf>,
    tx: &Sender<ScanResult>,
) -> Result<(), ()> {
    if dirty.is_empty() {
        return Ok(());
    }
    let sizes: HashMap<PathBuf, u64> = dirty
        .iter()
        .filter_map(|d| totals.get(d).map(|&s| (d.clone(), s)))
        .collect();
    dirty.clear();
    tx.send(ScanResult { sizes }).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_scan_rolls_up_sizes_from_leaves() {
        // root/
        //   a.txt            (100)
        //   sub/
        //     b.txt          (200)
        //     deep/c.txt     (300)
        let root = std::env::temp_dir().join(format!("treesize_test_{}", std::process::id()));
        let deep = root.join("sub").join("deep");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(root.join("a.txt"), vec![0u8; 100]).unwrap();
        std::fs::write(root.join("sub").join("b.txt"), vec![0u8; 200]).unwrap();
        std::fs::write(deep.join("c.txt"), vec![0u8; 300]).unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        process_scan(&root, &tx);
        drop(tx);

        // Merge every streamed delta, as the app does.
        let mut totals: HashMap<PathBuf, u64> = HashMap::new();
        for result in rx {
            totals.extend(result.sizes);
        }

        assert_eq!(totals.get(&deep).copied(), Some(300));
        assert_eq!(totals.get(&root.join("sub")).copied(), Some(500)); // 200 + deep
        assert_eq!(totals.get(&root).copied(), Some(600)); // 100 + sub

        std::fs::remove_dir_all(&root).unwrap();
    }
}
