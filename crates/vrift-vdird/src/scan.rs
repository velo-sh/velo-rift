//! RFC-0039: Compensation Scan for Live Ingest (Layer 3)
//!
//! Scans project directory for changes that occurred while daemon was stopped.
//! Uses mtime delta detection against manifest.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::ignore::IgnoreMatcher;
use crate::watch::IngestEvent;

/// Scan configuration
#[derive(Debug, Clone)]
pub struct ScanConfig {
    /// Root directory to scan
    pub root: PathBuf,
    /// Last known scan time (from manifest)
    pub last_scan: SystemTime,
    /// Ignore pattern matcher
    pub ignore: IgnoreMatcher,
    /// Maximum depth to scan
    pub max_depth: usize,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::new(),
            last_scan: SystemTime::UNIX_EPOCH,
            ignore: IgnoreMatcher::new(),
            max_depth: 50,
        }
    }
}

/// Compensation scanner for Layer 3
pub struct CompensationScanner {
    config: ScanConfig,
}

impl CompensationScanner {
    /// Create a new scanner
    pub fn new(root: PathBuf, last_scan: SystemTime) -> Self {
        Self {
            config: ScanConfig {
                root,
                last_scan,
                ..Default::default()
            },
        }
    }

    /// Check if path should be ignored
    fn should_ignore(&self, path: &Path) -> bool {
        self.config.ignore.should_ignore(path)
    }

    /// Scan directory and emit events for changed files
    pub fn scan(&self) -> Vec<IngestEvent> {
        let mut events = Vec::new();
        self.scan_dir(&self.config.root, 0, &mut events);
        events
    }

    /// Recursive directory scan
    fn scan_dir(&self, dir: &Path, depth: usize, events: &mut Vec<IngestEvent>) {
        if depth > self.config.max_depth {
            return;
        }

        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                warn!(path = %dir.display(), error = %e, "Failed to read directory");
                return;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();

            if self.should_ignore(&path) {
                continue;
            }

            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };

            let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);

            // Check if file was modified after last scan
            if mtime > self.config.last_scan {
                if metadata.is_dir() {
                    debug!(path = %path.display(), "Compensation: new directory");
                    events.push(IngestEvent::DirCreated { path: path.clone() });
                } else if metadata.is_symlink() {
                    let target = fs::read_link(&path).unwrap_or_default();
                    debug!(path = %path.display(), "Compensation: new symlink");
                    events.push(IngestEvent::SymlinkCreated {
                        path: path.clone(),
                        target,
                    });
                } else {
                    debug!(path = %path.display(), "Compensation: changed file");
                    events.push(IngestEvent::FileChanged { path: path.clone() });
                }
            }

            // Recurse into directories
            if metadata.is_dir() {
                self.scan_dir(&path, depth + 1, events);
            }
        }
    }
}

/// Run compensation scan and send events to channel
pub async fn run_compensation_scan(
    root: PathBuf,
    last_scan: SystemTime,
    tx: mpsc::Sender<IngestEvent>,
) -> usize {
    info!(root = %root.display(), "Starting compensation scan");

    let scanner = CompensationScanner::new(root, last_scan);
    let events = scanner.scan();
    let count = events.len();

    for event in events {
        if tx.send(event).await.is_err() {
            warn!("Ingest channel closed during compensation scan");
            break;
        }
    }

    info!(count, "Compensation scan complete");
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_scan_detects_new_files() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();

        // Create files
        fs::write(root.join("new_file.txt"), "content").unwrap();
        fs::create_dir(root.join("new_dir")).unwrap();

        // Scan with old timestamp
        let scanner = CompensationScanner::new(root, SystemTime::UNIX_EPOCH);
        let events = scanner.scan();

        assert!(!events.is_empty());
    }

    #[test]
    fn test_scan_with_custom_ignore() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();

        // Create custom directory
        let custom_dir = root.join("custom");
        fs::create_dir(&custom_dir).unwrap();
        fs::write(custom_dir.join("file.txt"), "content").unwrap();

        // Create normal file
        fs::write(root.join("normal.txt"), "content").unwrap();

        // Scanner with custom ignore patterns
        let mut scanner = CompensationScanner::new(root, SystemTime::UNIX_EPOCH);
        scanner.config.ignore =
            crate::ignore::IgnoreMatcher::with_patterns(&["custom".to_string()]);
        let events = scanner.scan();

        // Should include normal.txt but not custom/
        assert!(events.iter().any(
            |e| matches!(e, IngestEvent::FileChanged { path } if path.ends_with("normal.txt"))
        ));
        assert!(events.iter().all(|e| {
            match e {
                IngestEvent::FileChanged { path } => !path.to_string_lossy().contains("custom"),
                IngestEvent::DirCreated { path } => !path.to_string_lossy().contains("custom"),
                _ => true,
            }
        }));
    }
}
