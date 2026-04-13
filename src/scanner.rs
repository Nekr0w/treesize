use std::collections::{BinaryHeap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::tree::FileNode;

const NUM_WORKERS: usize = 2;

// ── Priority ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[allow(dead_code)]
pub enum ScanPriority {
    Low = 0,
    Medium = 1,
    High = 2,
}

// ── Results sent back to the main thread ──────────────────────────

pub enum ScanResult {
    /// Immediate children have been listed (dirs have size 0).
    ChildrenListed {
        parent: PathBuf,
        children: Vec<FileNode>,
    },
    /// A single child directory's full subtree size has been computed.
    ChildSizeComputed {
        child: PathBuf,
        size: u64,
    },
    /// All children of `parent` have been fully processed.
    ScanComplete {
        parent: PathBuf,
    },
}

// ── Internal request ──────────────────────────────────────────────

struct ScanRequest {
    path: PathBuf,
    priority: ScanPriority,
}

impl PartialEq for ScanRequest {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}
impl Eq for ScanRequest {}
impl PartialOrd for ScanRequest {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for ScanRequest {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority.cmp(&other.priority)
    }
}

// ── Shared state between manager and workers ──────────────────────

struct QueueInner {
    heap: BinaryHeap<ScanRequest>,
    pending: HashSet<PathBuf>,
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
                heap: BinaryHeap::new(),
                pending: HashSet::new(),
            }),
            condvar: Condvar::new(),
            shutdown: AtomicBool::new(false),
        });

        for _ in 0..NUM_WORKERS {
            let shared = Arc::clone(&shared);
            let tx = result_tx.clone();
            std::thread::spawn(move || worker_loop(&shared, &tx));
        }

        Self { shared, result_rx }
    }

    /// Submit a directory for scanning. Deduplicated: no-op if already queued.
    pub fn submit(&self, path: PathBuf, priority: ScanPriority) {
        let mut queue = self.shared.queue.lock().unwrap();
        if queue.pending.contains(&path) {
            return;
        }
        queue.pending.insert(path.clone());
        queue.heap.push(ScanRequest { path, priority });
        drop(queue);
        self.shared.condvar.notify_one();
    }

    /// Number of items queued or in-flight.
    pub fn pending_count(&self) -> usize {
        self.shared.queue.lock().unwrap().pending.len()
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
        let request = {
            let mut queue = shared.queue.lock().unwrap();
            loop {
                if shared.shutdown.load(AtomicOrdering::Relaxed) {
                    return;
                }
                if let Some(req) = queue.heap.pop() {
                    break req;
                }
                queue = shared.condvar.wait(queue).unwrap();
            }
        };

        // Process the scan
        process_scan(&request.path, result_tx);

        // Remove from pending set
        shared
            .queue
            .lock()
            .unwrap()
            .pending
            .remove(&request.path);
    }
}

const FLUSH_INTERVAL: Duration = Duration::from_millis(300);

/// Scan a directory: list children, then compute ALL dir children sizes
/// simultaneously via a single jwalk walk with periodic flushes.
fn process_scan(path: &Path, tx: &Sender<ScanResult>) {
    // Phase 1: list immediate children (fast, read_dir)
    let children = list_children(path);

    let dir_children: HashSet<PathBuf> = children
        .iter()
        .filter(|c| c.is_dir)
        .map(|c| c.path.clone())
        .collect();

    if tx
        .send(ScanResult::ChildrenListed {
            parent: path.to_path_buf(),
            children,
        })
        .is_err()
    {
        return;
    }

    if dir_children.is_empty() {
        let _ = tx.send(ScanResult::ScanComplete {
            parent: path.to_path_buf(),
        });
        return;
    }

    // Phase 2: single jwalk walk — all children sizes progress simultaneously
    let mut sizes: HashMap<PathBuf, u64> = HashMap::new();
    let mut last_flush = Instant::now();

    for entry in jwalk::WalkDir::new(path).skip_hidden(false) {
        let Ok(entry) = entry else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            continue;
        }

        if let Some(child_path) = immediate_child_of(path, &entry.path()) {
            *sizes.entry(child_path).or_insert(0) += meta.len();
        }

        // Flush intermediate results periodically → progressive UI updates
        if last_flush.elapsed() > FLUSH_INTERVAL {
            if flush_dir_sizes(&sizes, &dir_children, tx).is_err() {
                return;
            }
            last_flush = Instant::now();
        }
    }

    // Final flush with definitive sizes
    let _ = flush_dir_sizes(&sizes, &dir_children, tx);
    let _ = tx.send(ScanResult::ScanComplete {
        parent: path.to_path_buf(),
    });
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

/// Extract the immediate child path of `parent` that contains `descendant`.
/// e.g. immediate_child_of("/root", "/root/dir/file.txt") → Some("/root/dir")
fn immediate_child_of(parent: &Path, descendant: &Path) -> Option<PathBuf> {
    let relative = descendant.strip_prefix(parent).ok()?;
    let first = relative.components().next()?;
    Some(parent.join(first))
}

/// Send size updates for directory children only. Returns Err if channel closed.
fn flush_dir_sizes(
    sizes: &HashMap<PathBuf, u64>,
    dir_children: &HashSet<PathBuf>,
    tx: &Sender<ScanResult>,
) -> Result<(), ()> {
    for (child, &size) in sizes {
        if dir_children.contains(child) {
            tx.send(ScanResult::ChildSizeComputed {
                child: child.clone(),
                size,
            })
            .map_err(|_| ())?;
        }
    }
    Ok(())
}
