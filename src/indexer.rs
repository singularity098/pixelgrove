//! Keeps the database in sync with the media roots: a full scan at startup, live
//! filesystem watching, and periodic full rescans (watching is unreliable on NFS/SMB).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use serde::Serialize;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::db::Db;
use crate::scanner::{self, Excludes, ScanStats};

/// How long the watcher waits for a burst of changes (e.g. a copy) to settle.
const WATCH_DEBOUNCE: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Default, Serialize)]
pub struct IndexStatus {
    pub scanning: bool,
    pub watching: bool,
    /// Unix seconds when the last full scan finished.
    pub last_full_scan_at: Option<i64>,
    pub last_full_scan: Option<ScanStats>,
}

#[derive(Clone)]
pub struct Indexer {
    inner: Arc<Inner>,
}

struct Inner {
    db: Db,
    excludes: Excludes,
    rescan_interval: Duration,
    roots: Vec<(i64, PathBuf)>,
    status: Mutex<IndexStatus>,
    /// Serialises scans so watcher-triggered and periodic scans never overlap.
    scan_lock: tokio::sync::Mutex<()>,
}

impl Indexer {
    pub fn new(db: Db, config: &Config) -> Result<Self> {
        let excludes = Excludes::new(&config.exclude)?;
        let roots = scanner::sync_roots(&db, &config.media_roots)?;
        Ok(Self {
            inner: Arc::new(Inner {
                db,
                excludes,
                rescan_interval: Duration::from_secs(config.rescan_interval_hours.max(1) * 3600),
                roots,
                status: Mutex::new(IndexStatus::default()),
                scan_lock: tokio::sync::Mutex::new(()),
            }),
        })
    }

    pub fn status(&self) -> IndexStatus {
        self.inner.status.lock().unwrap().clone()
    }

    /// Starts the background tasks: initial scan, watcher and periodic rescans.
    pub fn start(&self) {
        let (tx, rx) = mpsc::unbounded_channel();
        let watcher = self.watch(tx);

        let this = self.clone();
        tokio::spawn(async move {
            // Keep the watcher alive for as long as the indexer runs.
            let _watcher = watcher;
            let mut interval = tokio::time::interval(this.inner.rescan_interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await; // The first tick fires immediately.
                this.full_scan().await;
            }
        });

        let this = self.clone();
        tokio::spawn(async move { this.handle_changes(rx).await });
    }

    /// Scans every root.
    pub async fn full_scan(&self) {
        let _guard = self.inner.scan_lock.lock().await;
        self.inner.status.lock().unwrap().scanning = true;
        let inner = self.inner.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut total = ScanStats::default();
            for (id, root) in &inner.roots {
                match scanner::scan(&inner.db, *id, root, root, &inner.excludes) {
                    Ok(stats) => total.add(&stats),
                    Err(err) => tracing::error!(root = %root.display(), "scan failed: {err:#}"),
                }
            }
            crate::alloc::release_free_memory();
            total
        })
        .await;

        let mut status = self.inner.status.lock().unwrap();
        status.scanning = false;
        if let Ok(stats) = result {
            tracing::info!(
                media_added = stats.media_added,
                media_updated = stats.media_updated,
                media_removed = stats.media_removed,
                media_unchanged = stats.media_unchanged,
                duration_ms = stats.duration.as_millis() as u64,
                "full scan finished"
            );
            status.last_full_scan_at = Some(unix_now());
            status.last_full_scan = Some(stats);
        }
    }

    fn watch(
        &self,
        tx: mpsc::UnboundedSender<PathBuf>,
    ) -> Option<Debouncer<RecommendedWatcher, RecommendedCache>> {
        let handler = move |result: DebounceEventResult| match result {
            Ok(events) => {
                for event in events {
                    for path in &event.paths {
                        let _ = tx.send(path.clone());
                    }
                }
            }
            Err(errors) => {
                for err in errors {
                    tracing::warn!(%err, "filesystem watch error");
                }
            }
        };
        let mut debouncer = match new_debouncer(WATCH_DEBOUNCE, None, handler) {
            Ok(debouncer) => debouncer,
            Err(err) => {
                tracing::warn!(%err, "filesystem watching unavailable; relying on periodic rescans");
                return None;
            }
        };
        let mut watching = false;
        for (_, root) in &self.inner.roots {
            match debouncer.watch(root, RecursiveMode::Recursive) {
                Ok(()) => watching = true,
                Err(err) => tracing::warn!(
                    root = %root.display(),
                    %err,
                    "cannot watch root; changes will be picked up by periodic rescans"
                ),
            }
        }
        self.inner.status.lock().unwrap().watching = watching;
        Some(debouncer)
    }

    async fn handle_changes(&self, mut rx: mpsc::UnboundedReceiver<PathBuf>) {
        while let Some(first) = rx.recv().await {
            // Collect everything already queued so one burst becomes one scan pass.
            let mut paths = vec![first];
            while let Ok(path) = rx.try_recv() {
                paths.push(path);
            }
            self.rescan(paths).await;
        }
    }

    /// Rescans the folders containing `paths` right away (e.g. after the app itself
    /// moved a file), without waiting for the watcher.
    pub async fn rescan(&self, paths: Vec<PathBuf>) {
        let targets = self.scan_targets(paths);
        if targets.is_empty() {
            return;
        }
        let _guard = self.inner.scan_lock.lock().await;
        let inner = self.inner.clone();
        let _ = tokio::task::spawn_blocking(move || {
            for (root_id, root, target) in targets {
                if let Err(err) = scanner::scan(&inner.db, root_id, &root, &target, &inner.excludes)
                {
                    tracing::warn!(path = %target.display(), "scan failed: {err:#}");
                }
            }
        })
        .await;
    }

    /// The configured media roots as `(id, path)`.
    pub fn roots(&self) -> &[(i64, PathBuf)] {
        &self.inner.roots
    }

    /// Maps changed paths to the minimal set of directories to rescan.
    fn scan_targets(&self, paths: Vec<PathBuf>) -> Vec<(i64, PathBuf, PathBuf)> {
        let mut dirs = BTreeSet::new();
        for path in paths {
            let Some((_, root)) = self.inner.roots.iter().find(|(_, r)| path.starts_with(r)) else {
                continue;
            };
            let relative = path.strip_prefix(root).unwrap_or(&path);
            if relative
                .iter()
                .any(|c| self.inner.excludes.is_excluded(&c.to_string_lossy()))
            {
                continue;
            }
            let dir = if path.is_dir() {
                path
            } else {
                path.parent().map(Path::to_path_buf).unwrap_or(path)
            };
            dirs.insert(dir);
        }
        // Drop directories whose ancestor is already being scanned (sorted order puts
        // ancestors first).
        let mut targets: Vec<PathBuf> = Vec::new();
        for dir in dirs {
            if !targets.iter().any(|t| dir.starts_with(t)) {
                targets.push(dir);
            }
        }
        targets
            .into_iter()
            .filter_map(|dir| {
                let (id, root) = self.inner.roots.iter().find(|(_, r)| dir.starts_with(r))?;
                Some((*id, root.clone(), dir))
            })
            .collect()
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdata;

    async fn wait_for(mut check: impl FnMut() -> bool) -> bool {
        for _ in 0..100 {
            if check() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }

    fn media_count(db: &Db) -> i64 {
        db.conn()
            .unwrap()
            .query_row("SELECT count(*) FROM media", [], |r| r.get(0))
            .unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn initial_scan_then_watch_picks_up_changes() {
        let config_dir = tempfile::tempdir().unwrap();
        let media = tempfile::tempdir().unwrap();
        std::fs::create_dir(media.path().join("album")).unwrap();
        std::fs::write(media.path().join("album/a.jpg"), testdata::jpeg_with_exif()).unwrap();

        let mut config = Config::with_config_dir(config_dir.path().to_path_buf());
        config.media_roots = vec![media.path().to_path_buf()];
        let db = Db::open(&config_dir.path().join("t.db")).unwrap();
        let indexer = Indexer::new(db.clone(), &config).unwrap();
        indexer.start();

        assert!(wait_for(|| indexer.status().last_full_scan_at.is_some()).await);
        assert_eq!(media_count(&db), 1);
        assert!(indexer.status().watching);

        std::fs::create_dir(media.path().join("new")).unwrap();
        std::fs::write(media.path().join("new/b.jpg"), testdata::jpeg_with_exif()).unwrap();
        assert!(
            wait_for(|| media_count(&db) == 2).await,
            "watcher should index the new file"
        );

        std::fs::remove_dir_all(media.path().join("album")).unwrap();
        assert!(
            wait_for(|| media_count(&db) == 1).await,
            "watcher should remove the deleted folder"
        );
    }

    #[test]
    fn scan_targets_are_minimal_and_skip_excluded() {
        let config_dir = tempfile::tempdir().unwrap();
        let media = tempfile::tempdir().unwrap();
        let root = media.path();
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        let mut config = Config::with_config_dir(config_dir.path().to_path_buf());
        config.media_roots = vec![root.to_path_buf()];
        let db = Db::open(&config_dir.path().join("t.db")).unwrap();
        let indexer = Indexer::new(db, &config).unwrap();

        let targets = indexer.scan_targets(vec![
            root.join("a/b/photo.jpg"),
            root.join("a"),
            root.join(".hidden/x.jpg"),
            root.join("@eaDir/thumb.jpg"),
            PathBuf::from("/elsewhere/x.jpg"),
        ]);
        let dirs: Vec<PathBuf> = targets.into_iter().map(|t| t.2).collect();
        assert_eq!(dirs, [root.join("a")]);
    }
}
