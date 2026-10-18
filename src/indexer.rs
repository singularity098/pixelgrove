//! Keeps the database in sync with the media roots: a full scan at startup, live
//! filesystem watching, and periodic full rescans (watching is unreliable on NFS/SMB).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use serde::Serialize;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::db::Db;
use crate::scanner::{self, Excludes, ScanStats};

/// How long the watcher waits for a burst of changes (e.g. a copy) to settle.
const WATCH_DEBOUNCE: Duration = Duration::from_secs(3);

/// Settings key holding the media folders added in the web UI (a JSON list of paths).
const EXTRA_ROOTS_KEY: &str = "extra_media_roots";

type Watcher = Debouncer<RecommendedWatcher, RecommendedCache>;

/// Why a media folder couldn't be added or removed.
#[derive(Debug)]
pub enum RootError {
    /// Explained to the user as-is.
    Invalid(String),
    Internal(anyhow::Error),
}

impl<E: Into<anyhow::Error>> From<E> for RootError {
    fn from(err: E) -> Self {
        Self::Internal(err.into())
    }
}

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
    /// Roots from the config file or environment; the web UI can't remove these.
    configured: Vec<PathBuf>,
    /// Every root (configured and added in the web UI) as `(id, path)`.
    roots: RwLock<Vec<(i64, PathBuf)>>,
    watcher: Mutex<Option<Watcher>>,
    status: Mutex<IndexStatus>,
    /// Serialises scans so watcher-triggered and periodic scans never overlap.
    scan_lock: tokio::sync::Mutex<()>,
}

impl Indexer {
    pub fn new(db: Db, config: &Config) -> Result<Self> {
        let excludes = Excludes::new(&config.exclude)?;
        let configured = config.media_roots.clone();
        let all = all_roots(&configured, &load_extra_roots(&db)?);
        let roots = scanner::sync_roots(&db, &all)?;
        Ok(Self {
            inner: Arc::new(Inner {
                db,
                excludes,
                rescan_interval: Duration::from_secs(config.rescan_interval_hours.max(1) * 3600),
                configured,
                roots: RwLock::new(roots),
                watcher: Mutex::new(None),
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
        *self.inner.watcher.lock().unwrap() = self.watch(tx);

        let this = self.clone();
        tokio::spawn(async move {
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
        let roots = self.roots();
        let result = tokio::task::spawn_blocking(move || {
            let mut total = ScanStats::default();
            for (id, root) in &roots {
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

    fn watch(&self, tx: mpsc::UnboundedSender<PathBuf>) -> Option<Watcher> {
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
        for (_, root) in self.roots() {
            watching |= watch_root(&mut debouncer, &root);
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

    /// Every media root as `(id, path)`.
    pub fn roots(&self) -> Vec<(i64, PathBuf)> {
        self.inner.roots.read().unwrap().clone()
    }

    /// Whether `path` comes from the config file or environment (not removable here).
    pub fn is_configured(&self, path: &Path) -> bool {
        self.inner.configured.iter().any(|r| r == path)
    }

    /// Adds a media folder (saved in the database), then indexes it in the background.
    pub async fn add_root(&self, path: &str) -> Result<(), RootError> {
        let path = normalise_root(path)?;
        let meta = std::fs::metadata(&path).map_err(|_| {
            RootError::Invalid(format!(
                "{} doesn't exist in the container.",
                path.display()
            ))
        })?;
        if !meta.is_dir() {
            return Err(RootError::Invalid(format!(
                "{} isn't a directory.",
                path.display()
            )));
        }
        let _guard = self.inner.scan_lock.lock().await;
        for (_, root) in self.roots() {
            if path.starts_with(&root) || root.starts_with(&path) {
                return Err(RootError::Invalid(if path == root {
                    format!("{} is already in the library.", path.display())
                } else {
                    format!(
                        "{} overlaps {}, which is already in the library.",
                        path.display(),
                        root.display()
                    )
                }));
            }
        }
        let mut extra = load_extra_roots(&self.inner.db)?;
        extra.push(path.clone());
        self.apply_roots(&extra)?;
        if let Some(watcher) = self.inner.watcher.lock().unwrap().as_mut() {
            watch_root(watcher, &path);
        }
        tracing::info!(root = %path.display(), "media folder added");
        drop(_guard);

        let this = self.clone();
        tokio::spawn(async move { this.full_scan().await });
        Ok(())
    }

    /// Removes a media folder added in the web UI, and everything indexed from it.
    /// The files themselves are untouched.
    pub async fn remove_root(&self, path: &str) -> Result<(), RootError> {
        let path = normalise_root(path)?;
        if self.is_configured(&path) {
            return Err(RootError::Invalid(format!(
                "{} is set in the server configuration (PIXELGROVE_MEDIA_ROOTS), so it can only be removed there.",
                path.display()
            )));
        }
        let _guard = self.inner.scan_lock.lock().await;
        let mut extra = load_extra_roots(&self.inner.db)?;
        let before = extra.len();
        extra.retain(|r| *r != path);
        if extra.len() == before {
            return Err(RootError::Invalid(format!(
                "{} isn't in the library.",
                path.display()
            )));
        }
        if let Some(watcher) = self.inner.watcher.lock().unwrap().as_mut() {
            let _ = watcher.unwatch(&path);
        }
        self.apply_roots(&extra)?;
        tracing::info!(root = %path.display(), "media folder removed");
        Ok(())
    }

    /// Saves the added roots and brings the database and root list in line.
    fn apply_roots(&self, extra: &[PathBuf]) -> Result<()> {
        let list: Vec<String> = extra
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        self.inner.db.conn()?.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [EXTRA_ROOTS_KEY, &serde_json::to_string(&list)?],
        )?;
        let roots = scanner::sync_roots(&self.inner.db, &all_roots(&self.inner.configured, extra))?;
        *self.inner.roots.write().unwrap() = roots;
        Ok(())
    }

    /// Maps changed paths to the minimal set of directories to rescan.
    fn scan_targets(&self, paths: Vec<PathBuf>) -> Vec<(i64, PathBuf, PathBuf)> {
        let roots = self.roots();
        let mut dirs = BTreeSet::new();
        for path in paths {
            let Some((_, root)) = roots.iter().find(|(_, r)| path.starts_with(r)) else {
                continue;
            };
            // An ignore marker appearing or going away changes its whole folder.
            let marker = path
                .file_name()
                .is_some_and(|n| scanner::IGNORE_MARKERS.iter().any(|m| n == *m));
            let path = match (marker, path.parent()) {
                (true, Some(parent)) => parent.to_path_buf(),
                _ => path,
            };
            let relative = path.strip_prefix(root).unwrap_or(&path);
            if relative
                .iter()
                .any(|c| self.inner.excludes.is_excluded(&c.to_string_lossy()))
            {
                continue;
            }
            let dir = if marker || path.is_dir() {
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
                let (id, root) = roots.iter().find(|(_, r)| dir.starts_with(r))?;
                Some((*id, root.clone(), dir))
            })
            .collect()
    }
}

fn watch_root(watcher: &mut Watcher, root: &Path) -> bool {
    match watcher.watch(root, RecursiveMode::Recursive) {
        Ok(()) => true,
        Err(err) => {
            tracing::warn!(
                root = %root.display(),
                %err,
                "cannot watch root; changes will be picked up by periodic rescans"
            );
            false
        }
    }
}

/// Media folders added in the web UI.
pub fn load_extra_roots(db: &Db) -> Result<Vec<PathBuf>> {
    let value: Option<String> = rusqlite::OptionalExtension::optional(db.conn()?.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        [EXTRA_ROOTS_KEY],
        |r| r.get(0),
    ))?;
    let list: Vec<String> = match value {
        Some(json) => serde_json::from_str(&json).context("reading added media folders")?,
        None => Vec::new(),
    };
    Ok(list.into_iter().map(PathBuf::from).collect())
}

/// Configured roots first, then added ones not already among them.
fn all_roots(configured: &[PathBuf], extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut all = configured.to_vec();
    for path in extra {
        if !all.contains(path) {
            all.push(path.clone());
        }
    }
    all
}

/// An absolute path without `..` or a trailing slash.
fn normalise_root(path: &str) -> Result<PathBuf, RootError> {
    let trimmed = path.trim();
    let trimmed = if trimmed.len() > 1 {
        trimmed.trim_end_matches('/')
    } else {
        trimmed
    };
    let path = Path::new(trimmed);
    if !path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(RootError::Invalid(
            "Enter a full path, such as /photos.".into(),
        ));
    }
    if path == Path::new("/") {
        return Err(RootError::Invalid(
            "The whole filesystem can't be added; pick a folder.".into(),
        ));
    }
    Ok(path.to_path_buf())
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

        std::fs::write(media.path().join("new/.nomedia"), b"").unwrap();
        assert!(
            wait_for(|| media_count(&db) == 1).await,
            "a new .nomedia hides its folder"
        );
        std::fs::remove_file(media.path().join("new/.nomedia")).unwrap();
        assert!(
            wait_for(|| media_count(&db) == 2).await,
            "removing it brings the folder back"
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

        // Ignore markers are hidden files, but adding or removing one rescans its folder.
        let targets = indexer.scan_targets(vec![root.join("a/b/.nomedia")]);
        let dirs: Vec<PathBuf> = targets.into_iter().map(|t| t.2).collect();
        assert_eq!(dirs, [root.join("a/b")]);
    }
}
