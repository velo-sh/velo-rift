//! RFC-0039: FS Watch for Live Ingest (Layer 2)
//!
//! Watches project directory for changes made outside the shim layer.
//! Uses FSEvents on macOS, inotify on Linux.

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
use tokio::sync::mpsc as tokio_mpsc;
use tracing::{debug, info, warn};

use crate::ignore::IgnoreMatcher;

/// Ingest event from any source (L1/L2/L3)
#[derive(Debug, Clone)]
pub enum IngestEvent {
    /// File created or modified
    FileChanged { path: PathBuf },
    /// Directory created
    DirCreated { path: PathBuf },
    /// File/Dir removed
    Removed { path: PathBuf },
    /// Symlink created
    SymlinkCreated { path: PathBuf, target: PathBuf },
}

/// FS Watch configuration
#[derive(Debug, Clone)]
pub struct WatchConfig {
    /// Root directory to watch
    pub root: PathBuf,
    /// Debounce duration (to coalesce rapid writes)
    pub debounce: Duration,
    /// Ignore matcher
    pub ignore: IgnoreMatcher,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::new(),
            debounce: Duration::from_millis(100),
            ignore: IgnoreMatcher::new(),
        }
    }
}

/// FS Watcher for Layer 2
pub struct FsWatch {
    config: WatchConfig,
    #[allow(dead_code)]
    watcher: RecommendedWatcher,
    event_rx: Receiver<Result<Event, notify::Error>>,
}

impl FsWatch {
    /// Create a new FS watcher for the given path
    pub fn new(root: PathBuf) -> notify::Result<Self> {
        let config = WatchConfig {
            root: root.clone(),
            ..Default::default()
        };

        let (tx, rx) = mpsc::channel();

        let watcher_config = Config::default()
            .with_poll_interval(Duration::from_secs(2))
            .with_compare_contents(false);

        let mut watcher = RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            watcher_config,
        )?;

        watcher.watch(&root, RecursiveMode::Recursive)?;

        info!(path = %root.display(), "FS Watch started");

        Ok(Self {
            config,
            watcher,
            event_rx: rx,
        })
    }

    /// Check if path should be ignored
    fn should_ignore(&self, path: &Path) -> bool {
        self.config.ignore.should_ignore(path)
    }

    /// Convert notify event to IngestEvent
    fn to_ingest_event(&self, event: Event) -> Vec<IngestEvent> {
        use notify::EventKind;

        let mut events = Vec::new();

        for path in event.paths {
            if self.should_ignore(&path) {
                continue;
            }

            let ingest_event = match event.kind {
                EventKind::Create(_) => {
                    if path.is_dir() {
                        IngestEvent::DirCreated { path }
                    } else if path.is_symlink() {
                        // Read symlink target
                        let target = std::fs::read_link(&path).unwrap_or_default();
                        IngestEvent::SymlinkCreated { path, target }
                    } else {
                        IngestEvent::FileChanged { path }
                    }
                }
                EventKind::Modify(_) => IngestEvent::FileChanged { path },
                EventKind::Remove(_) => IngestEvent::Removed { path },
                _ => continue,
            };

            events.push(ingest_event);
        }

        events
    }

    /// Poll for events (non-blocking)
    pub fn poll(&self) -> Vec<IngestEvent> {
        let mut events = Vec::new();

        while let Ok(result) = self.event_rx.try_recv() {
            match result {
                Ok(event) => {
                    debug!(?event, "FS event received");
                    events.extend(self.to_ingest_event(event));
                }
                Err(e) => {
                    warn!(error = %e, "FS watch error");
                }
            }
        }

        events
    }
}

/// Spawn async watcher task that sends events to a channel
pub fn spawn_watch_task(
    root: PathBuf,
    tx: tokio_mpsc::Sender<IngestEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let watcher = match FsWatch::new(root.clone()) {
            Ok(w) => w,
            Err(e) => {
                warn!(error = %e, "Failed to start FS watch");
                return;
            }
        };

        loop {
            // Poll for events
            let events = watcher.poll();

            for event in events {
                if tx.send(event).await.is_err() {
                    // Channel closed, exit
                    return;
                }
            }

            // Sleep briefly to avoid busy loop
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper to check if path matches ignore patterns
    fn should_ignore_path(path: &Path, patterns: &[String]) -> bool {
        for pattern in patterns {
            if path
                .components()
                .any(|c| c.as_os_str().to_string_lossy() == *pattern)
            {
                return true;
            }
        }
        false
    }

    #[test]
    fn test_should_ignore() {
        let patterns = vec![".git".into(), "target".into()];

        assert!(should_ignore_path(
            Path::new("/project/.git/config"),
            &patterns
        ));
        assert!(should_ignore_path(
            Path::new("/project/target/debug/bin"),
            &patterns
        ));
        assert!(!should_ignore_path(
            Path::new("/project/src/main.rs"),
            &patterns
        ));
    }

    #[test]
    fn test_watch_config_default() {
        let config = WatchConfig::default();
        // Test that actual defaults are in place (NOT .git or target - those are user-configured)
        assert!(config.ignore.should_ignore(Path::new(".vrift")));
        assert!(config.ignore.should_ignore(Path::new(".DS_Store")));
    }
}
