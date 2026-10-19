//! Incremental filesystem scanner.
//!
//! A scan walks one directory subtree, compares it with what the database knows about
//! that subtree, reads metadata only for new or changed files (in parallel), and
//! applies the differences in a single transaction.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use globset::{Glob, GlobSet, GlobSetBuilder};
use nom_exif::MediaParser;
use rayon::prelude::*;
use rusqlite::{OptionalExtension, Transaction, params};
use serde::Serialize;
use walkdir::WalkDir;

use crate::db::Db;
use crate::metadata::{self, MediaInfo, MediaKind};

/// Directory or file names that are never indexed, in addition to hidden (dot) entries.
pub const DEFAULT_EXCLUDES: &[&str] = &["@eaDir", "#recycle", "#snapshot", "lost+found"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ScanStats {
    pub folders_added: u64,
    pub folders_removed: u64,
    pub media_added: u64,
    pub media_updated: u64,
    pub media_removed: u64,
    pub media_unchanged: u64,
    #[serde(serialize_with = "as_millis")]
    pub duration: Duration,
}

impl ScanStats {
    pub fn changed(&self) -> bool {
        self.folders_added
            + self.folders_removed
            + self.media_added
            + self.media_updated
            + self.media_removed
            > 0
    }

    pub fn add(&mut self, other: &ScanStats) {
        self.folders_added += other.folders_added;
        self.folders_removed += other.folders_removed;
        self.media_added += other.media_added;
        self.media_updated += other.media_updated;
        self.media_removed += other.media_removed;
        self.media_unchanged += other.media_unchanged;
        self.duration += other.duration;
    }
}

fn as_millis<S: serde::Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_u64(d.as_millis() as u64)
}

/// Name patterns (globs) that exclude a file or directory from indexing.
#[derive(Clone)]
pub struct Excludes(GlobSet);

/// A folder holding one of these files is left out, with everything below it (like
/// Android's `.nomedia`).
pub const IGNORE_MARKERS: [&str; 2] = [".nomedia", ".ignore"];

/// True if `dir` contains an ignore marker.
pub fn is_ignored_dir(dir: &Path) -> bool {
    IGNORE_MARKERS.iter().any(|m| dir.join(m).is_file())
}

impl Excludes {
    pub fn new(patterns: &[String]) -> Result<Self> {
        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            builder.add(
                Glob::new(pattern).with_context(|| format!("invalid exclude pattern {pattern}"))?,
            );
        }
        Ok(Self(builder.build()?))
    }

    /// True if an entry with this file name should be skipped.
    pub fn is_excluded(&self, name: &str) -> bool {
        name.starts_with('.') || self.0.is_match(name)
    }
}

/// Makes the `roots` table match `paths`, removing roots (and their contents) that are
/// no longer configured. Returns `(id, path)` for each root.
pub fn sync_roots(db: &Db, paths: &[PathBuf]) -> Result<Vec<(i64, PathBuf)>> {
    let mut conn = db.conn()?;
    let tx = conn.transaction()?;
    let wanted: Vec<String> = paths.iter().map(|p| path_str(p)).collect::<Result<_>>()?;
    {
        let mut existing = tx.prepare("SELECT id, path FROM roots")?;
        let rows: Vec<(i64, String)> = existing
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (id, path) in rows {
            if !wanted.contains(&path) {
                tracing::info!(root = %path, "removing root that is no longer configured");
                tx.execute("DELETE FROM folders WHERE root_id = ?1", [id])?;
                tx.execute("DELETE FROM roots WHERE id = ?1", [id])?;
            }
        }
    }
    let mut roots = Vec::new();
    for path in &wanted {
        tx.execute("INSERT OR IGNORE INTO roots (path) VALUES (?1)", [path])?;
        let id = tx.query_row("SELECT id FROM roots WHERE path = ?1", [path], |r| r.get(0))?;
        roots.push((id, PathBuf::from(path)));
    }
    tx.commit()?;
    Ok(roots)
}

/// Files whose metadata is read and written per transaction. Bounds memory use and
/// keeps write locks short, so a long first scan fills the library progressively.
const CHUNK_SIZE: usize = 2000;

/// Scans `target` (a directory at or below `root`), or the nearest ancestor the
/// database already knows about if `target` is new.
pub fn scan(
    db: &Db,
    root_id: i64,
    root: &Path,
    target: &Path,
    excludes: &Excludes,
) -> Result<ScanStats> {
    let started = Instant::now();
    if !target.starts_with(root) {
        bail!("{} is not inside root {}", target.display(), root.display());
    }
    let mut conn = db.conn()?;
    let mut target = target.to_path_buf();
    while target != root && folder_id(&conn, &path_str(&target)?)?.is_none() {
        target.pop();
    }

    let fs_tree = walk(root, &target, excludes)?;
    let db_tree = load_subtree(&conn, &target)?;

    // A root that is missing or completely empty while we know media in it is far more
    // likely an unmounted share than a deliberately emptied library: keep the index.
    if target == root
        && fs_tree.dirs.len() <= 1
        && fs_tree.files.is_empty()
        && !db_tree.media.is_empty()
        && !is_ignored_dir(root)
    {
        tracing::warn!(
            root = %root.display(),
            "media root is missing or empty; keeping the existing index (remove the root from the configuration to clear it)"
        );
        return Ok(ScanStats {
            duration: started.elapsed(),
            ..ScanStats::default()
        });
    }

    // Phase 1: folder structure and removed media, in one transaction.
    let tx = conn.transaction()?;
    let (dir_ids, mut stats) = apply_structure(&tx, root_id, &fs_tree, &db_tree)?;
    tx.commit()?;

    // Phase 2: metadata for new or changed files, in chunks.
    let changed: Vec<(&FsFile, i64, bool)> = fs_tree
        .files
        .iter()
        .filter_map(|f| {
            let folder_id = dir_ids[f.dir];
            match db_tree.media(folder_id, &f.name) {
                Some(known) if known.size == f.size && known.mtime == f.mtime => {
                    stats.media_unchanged += 1;
                    None
                }
                known => Some((f, folder_id, known.is_some())),
            }
        })
        .collect();
    for chunk in changed.chunks(CHUNK_SIZE) {
        let infos: Vec<MediaInfo> = chunk
            .par_iter()
            .map_init(MediaParser::new, |parser, (f, _, _)| {
                let path = Path::new(&fs_tree.dirs[f.dir]).join(&f.name);
                metadata::read(parser, &path, f.kind)
            })
            .collect();
        let tx = conn.transaction()?;
        upsert_media(&tx, root, &fs_tree.dirs, chunk, &infos, &mut stats)?;
        tx.commit()?;
    }

    stats.duration = started.elapsed();
    if stats.changed() {
        tracing::info!(path = %target.display(), ?stats, "scan complete");
    } else {
        tracing::debug!(path = %target.display(), ?stats, "scan complete, no changes");
    }
    Ok(stats)
}

struct FsFile {
    /// Index into `FsTree::dirs`.
    dir: usize,
    name: String,
    kind: MediaKind,
    size: i64,
    mtime: i64,
}

struct FsTree {
    /// Directories in the subtree (including its top), parents before children.
    dirs: Vec<String>,
    files: Vec<FsFile>,
}

fn walk(root: &Path, target: &Path, excludes: &Excludes) -> Result<FsTree> {
    let mut tree = FsTree {
        dirs: Vec::new(),
        files: Vec::new(),
    };
    if !target.is_dir() {
        if target == root {
            tracing::warn!(root = %root.display(), "media root is missing or not a directory");
        }
        return Ok(tree);
    }
    // Inside an ignored folder (the target or one of its parents up to the root):
    // nothing here belongs in the library.
    if target
        .ancestors()
        .take_while(|a| a.starts_with(root))
        .any(is_ignored_dir)
    {
        return Ok(tree);
    }
    let walker = WalkDir::new(target)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || (!excludes.is_excluded(&e.file_name().to_string_lossy())
                    && !(e.file_type().is_dir() && is_ignored_dir(e.path())))
        });
    // Directory path → index in `dirs`, for the parents of files.
    let mut dir_index: HashMap<PathBuf, usize> = HashMap::new();
    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::warn!(%err, "skipping unreadable entry");
                continue;
            }
        };
        let file_type = entry.file_type();
        if file_type.is_dir() {
            let Some(path) = entry.path().to_str() else {
                tracing::warn!(path = %entry.path().display(), "skipping directory whose path is not valid UTF-8");
                continue;
            };
            dir_index.insert(entry.path().to_path_buf(), tree.dirs.len());
            tree.dirs.push(path.to_string());
        } else if file_type.is_file()
            && let Some(kind) = metadata::kind_for_path(entry.path())
            && let Some(&dir) = entry.path().parent().and_then(|p| dir_index.get(p))
        {
            let Some(name) = entry.file_name().to_str() else {
                tracing::warn!(path = %entry.path().display(), "skipping file whose name is not valid UTF-8");
                continue;
            };
            let Ok(meta) = entry.metadata() else { continue };
            tree.files.push(FsFile {
                dir,
                name: name.to_string(),
                kind,
                size: meta.len() as i64,
                mtime: metadata::mtime_secs(&meta),
            });
        }
    }
    Ok(tree)
}

struct KnownMedia {
    id: i64,
    size: i64,
    mtime: i64,
}

struct DbTree {
    /// Folder path → id.
    folders: HashMap<String, i64>,
    /// Folder id → file name → media.
    media: HashMap<i64, HashMap<String, KnownMedia>>,
}

impl DbTree {
    fn media(&self, folder_id: i64, name: &str) -> Option<&KnownMedia> {
        self.media.get(&folder_id)?.get(name)
    }
}

/// SQL condition selecting a folder path equal to ?1 or below it (?2 is `?1 + "/"`,
/// ?3 is `?1 + "0"`, the character after '/'), using the unique index on `path`.
const IN_SUBTREE: &str = "(f.path = ?1 OR (f.path >= ?2 AND f.path < ?3))";

fn subtree_params(target: &str) -> [String; 3] {
    let base = target.trim_end_matches('/');
    [target.to_string(), format!("{base}/"), format!("{base}0")]
}

fn load_subtree(conn: &rusqlite::Connection, target: &Path) -> Result<DbTree> {
    let params = subtree_params(&path_str(target)?);
    let mut folders = HashMap::new();
    let mut stmt = conn.prepare(&format!(
        "SELECT f.path, f.id FROM folders f WHERE {IN_SUBTREE}"
    ))?;
    let mut rows = stmt.query(rusqlite::params_from_iter(&params))?;
    while let Some(row) = rows.next()? {
        folders.insert(row.get(0)?, row.get(1)?);
    }
    let mut media: HashMap<i64, HashMap<String, KnownMedia>> = HashMap::new();
    let mut stmt = conn.prepare(&format!(
        "SELECT m.folder_id, m.name, m.id, m.size, m.mtime
         FROM media m JOIN folders f ON f.id = m.folder_id WHERE {IN_SUBTREE}"
    ))?;
    let mut rows = stmt.query(rusqlite::params_from_iter(&params))?;
    while let Some(row) = rows.next()? {
        media.entry(row.get(0)?).or_default().insert(
            row.get(1)?,
            KnownMedia {
                id: row.get(2)?,
                size: row.get(3)?,
                mtime: row.get(4)?,
            },
        );
    }
    Ok(DbTree { folders, media })
}

/// Removes vanished folders and media and adds new folders. Returns the folder id for
/// each entry of `fs_tree.dirs`.
fn apply_structure(
    tx: &Transaction,
    root_id: i64,
    fs_tree: &FsTree,
    db_tree: &DbTree,
) -> Result<(Vec<i64>, ScanStats)> {
    let mut stats = ScanStats::default();
    let mut folder_ids = db_tree.folders.clone();
    let mut touched: HashSet<i64> = HashSet::new();

    // Folders that disappeared; their media cascade away with them.
    let fs_dirs: HashSet<&str> = fs_tree.dirs.iter().map(String::as_str).collect();
    {
        let mut delete = tx.prepare("DELETE FROM folders WHERE id = ?1")?;
        for (path, id) in &db_tree.folders {
            if !fs_dirs.contains(path.as_str()) {
                delete.execute([id])?;
                folder_ids.remove(path);
                stats.folders_removed += 1;
                stats.media_removed += db_tree.media.get(id).map_or(0, |m| m.len() as u64);
            }
        }
    }

    // New folders, parents first (`dirs` is in walk order).
    let mut dir_ids = Vec::with_capacity(fs_tree.dirs.len());
    {
        let mut insert = tx.prepare(
            "INSERT INTO folders (root_id, parent_id, path, name) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for dir in &fs_tree.dirs {
            if let Some(&id) = folder_ids.get(dir) {
                dir_ids.push(id);
                continue;
            }
            let path = Path::new(dir);
            let parent_id = match path.parent().and_then(Path::to_str) {
                Some(parent) => match folder_ids.get(parent) {
                    Some(&id) => Some(id),
                    None => folder_id(tx, parent)?,
                },
                None => None,
            };
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(dir.as_str());
            insert.execute(params![root_id, parent_id, dir, name])?;
            let id = tx.last_insert_rowid();
            folder_ids.insert(dir.clone(), id);
            dir_ids.push(id);
            stats.folders_added += 1;
        }
    }

    // Media that disappeared from folders that still exist.
    {
        let fs_files: HashSet<(i64, &str)> = fs_tree
            .files
            .iter()
            .map(|f| (dir_ids[f.dir], f.name.as_str()))
            .collect();
        let live_folders: HashSet<i64> = dir_ids.iter().copied().collect();
        let mut delete = tx.prepare("DELETE FROM media WHERE id = ?1")?;
        for (&folder_id, files) in &db_tree.media {
            if !live_folders.contains(&folder_id) {
                continue; // Removed with its folder above.
            }
            for (name, known) in files {
                if !fs_files.contains(&(folder_id, name.as_str())) {
                    delete.execute([known.id])?;
                    touched.insert(folder_id);
                    stats.media_removed += 1;
                }
            }
        }
    }

    update_aggregates(tx, &touched)?;
    Ok((dir_ids, stats))
}

/// Inserts or updates one chunk of media rows: `(file, folder id, already known)`.
fn upsert_media(
    tx: &Transaction,
    root: &Path,
    dirs: &[String],
    files: &[(&FsFile, i64, bool)],
    infos: &[MediaInfo],
    stats: &mut ScanStats,
) -> Result<()> {
    let mut touched = HashSet::new();
    {
        let mut upsert = tx.prepare(
            "INSERT INTO media (folder_id, name, kind, size, mtime, taken_at, tz_offset, width,
                 height, orientation, duration_ms, camera_make, camera_model, lens, latitude,
                 longitude, description, search_text, place, country)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                     ?18, ?19, ?20)
             ON CONFLICT (folder_id, name) DO UPDATE SET
                 kind = excluded.kind, size = excluded.size, mtime = excluded.mtime,
                 taken_at = excluded.taken_at, tz_offset = excluded.tz_offset,
                 width = excluded.width, height = excluded.height,
                 orientation = excluded.orientation, duration_ms = excluded.duration_ms,
                 camera_make = excluded.camera_make, camera_model = excluded.camera_model,
                 lens = excluded.lens, latitude = excluded.latitude,
                 longitude = excluded.longitude, description = excluded.description,
                 search_text = excluded.search_text, place = excluded.place,
                 country = excluded.country",
        )?;
        for ((file, folder_id, known), info) in files.iter().zip(infos) {
            let taken_at = info.taken_at.unwrap_or(file.mtime);
            let folder = Path::new(&dirs[file.dir])
                .strip_prefix(root)
                .map(|p| p.to_string_lossy())
                .unwrap_or_default();
            let place = match (info.latitude, info.longitude) {
                (Some(lat), Some(lon)) => crate::places::lookup(lat, lon),
                _ => None,
            };
            let search = search_text(&file.name, &folder, info, taken_at, place.as_ref());
            upsert.execute(params![
                folder_id,
                file.name,
                file.kind.as_str(),
                file.size,
                file.mtime,
                taken_at,
                info.tz_offset,
                info.width,
                info.height,
                info.orientation,
                info.duration_ms.map(|d| d as i64),
                info.camera_make,
                info.camera_model,
                info.lens,
                info.latitude,
                info.longitude,
                info.description,
                search,
                place.as_ref().map(|p| &p.city),
                place.as_ref().map(|p| &p.country),
            ])?;
            touched.insert(*folder_id);
            if *known {
                stats.media_updated += 1;
            } else {
                stats.media_added += 1;
            }
        }
    }
    update_aggregates(tx, &touched)
}

/// The lower-cased text that search matches against: file name, folder path below the
/// root, camera, lens, description, the capture date as `YYYY-MM-DD`, and the place.
pub fn search_text(
    name: &str,
    folder: &str,
    info: &MediaInfo,
    taken_at: i64,
    place: Option<&crate::places::Place>,
) -> String {
    let date = chrono::DateTime::from_timestamp(taken_at, 0)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    [
        name,
        folder,
        info.camera_make.as_deref().unwrap_or(""),
        info.camera_model.as_deref().unwrap_or(""),
        info.lens.as_deref().unwrap_or(""),
        info.description.as_deref().unwrap_or(""),
        &date,
        place.map_or("", |p| p.city.as_str()),
        place.map_or("", |p| p.country.as_str()),
    ]
    .join(" ")
    .to_lowercase()
}

/// Recomputes count / newest time / cover for the given folders.
fn update_aggregates(tx: &Transaction, folder_ids: &HashSet<i64>) -> Result<()> {
    let mut update = tx.prepare(
        "UPDATE folders SET
            media_count = (SELECT count(*) FROM media WHERE folder_id = ?1),
            latest_taken_at = (SELECT max(taken_at) FROM media WHERE folder_id = ?1),
            cover_media_id = (SELECT id FROM media WHERE folder_id = ?1
                              ORDER BY taken_at DESC, id DESC LIMIT 1)
         WHERE id = ?1",
    )?;
    for id in folder_ids {
        update.execute([id])?;
    }
    Ok(())
}

fn folder_id(conn: &rusqlite::Connection, path: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row("SELECT id FROM folders WHERE path = ?1", [path], |r| {
            r.get(0)
        })
        .optional()?)
}

fn path_str(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_string)
        .with_context(|| format!("path is not valid UTF-8: {}", path.display()))
}

/// Fills in place names for media indexed before places existed (or whose coordinates
/// had no nearby city then). Runs at startup; cheap when there's nothing to do.
pub fn backfill_places(db: &Db) -> Result<usize> {
    let mut conn = db.conn()?;
    let pending: Vec<(i64, f64, f64)> = conn
        .prepare(
            "SELECT id, latitude, longitude FROM media
             WHERE latitude IS NOT NULL AND longitude IS NOT NULL AND country IS NULL",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let tx = conn.transaction()?;
    let mut filled = 0;
    {
        let mut update = tx.prepare(
            "UPDATE media SET place = ?1, country = ?2,
                 search_text = search_text || ' ' || lower(?1 || ' ' || ?2)
             WHERE id = ?3",
        )?;
        for (id, lat, lon) in pending {
            if let Some(place) = crate::places::lookup(lat, lon) {
                update.execute(params![place.city, place.country, id])?;
                filled += 1;
            }
        }
    }
    tx.commit()?;
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdata;
    use std::fs;

    struct Fixture {
        _config: tempfile::TempDir,
        media: tempfile::TempDir,
        db: Db,
        root_id: i64,
        excludes: Excludes,
    }

    impl Fixture {
        fn new() -> Self {
            let config = tempfile::tempdir().unwrap();
            let media = tempfile::tempdir().unwrap();
            let db = Db::open(&config.path().join("test.db")).unwrap();
            let roots = sync_roots(&db, &[media.path().to_path_buf()]).unwrap();
            let defaults: Vec<String> = DEFAULT_EXCLUDES.iter().map(|s| s.to_string()).collect();
            Self {
                _config: config,
                root_id: roots[0].0,
                media,
                db,
                excludes: Excludes::new(&defaults).unwrap(),
            }
        }

        fn root(&self) -> &Path {
            self.media.path()
        }

        fn write(&self, rel: &str, bytes: &[u8]) {
            let path = self.root().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }

        fn scan(&self) -> ScanStats {
            self.scan_at(self.root())
        }

        fn scan_at(&self, target: &Path) -> ScanStats {
            scan(&self.db, self.root_id, self.root(), target, &self.excludes).unwrap()
        }

        fn query<T: rusqlite::types::FromSql>(&self, sql: &str) -> Vec<T> {
            let conn = self.db.conn().unwrap();
            let mut stmt = conn.prepare(sql).unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        }
    }

    #[test]
    fn moved_files_follow() {
        let fx = Fixture::new();
        fx.write("a/x.jpg", &testdata::jpeg_taken_at("2020:01:01 00:00:00"));
        fx.write("b/y.jpg", &testdata::jpeg_taken_at("2021:01:01 00:00:00"));
        fx.scan();
        for (from, to) in [
            ("a/x.jpg", "b/x.jpg"),
            ("b/x.jpg", "c/new/x.jpg"),
            ("c/new/x.jpg", "x.jpg"),
        ] {
            let to_path = fx.root().join(to);
            fs::create_dir_all(to_path.parent().unwrap()).unwrap();
            fs::rename(fx.root().join(from), &to_path).unwrap();
            fx.scan();
            let found: Vec<String> = fx.query(
                "SELECT f.path || '/' || m.name FROM media m JOIN folders f ON f.id = m.folder_id
                 WHERE m.name = 'x.jpg'",
            );
            assert_eq!(
                found,
                [to_path.to_string_lossy()],
                "after moving {from} to {to}"
            );
            let counts: Vec<i64> = fx.query("SELECT sum(media_count) FROM folders");
            assert_eq!(counts, [2]);
        }
    }

    #[test]
    fn indexes_media_and_folders() {
        let fx = Fixture::new();
        fx.write(
            "2023/trip/a.jpg",
            &testdata::jpeg_taken_at("2023:06:15 14:30:00"),
        );
        fx.write(
            "2023/trip/b.jpg",
            &testdata::jpeg_taken_at("2023:06:16 09:00:00"),
        );
        fx.write("2023/trip/notes.txt", b"ignored");
        fx.write("2024/c.png", testdata::PNG_3X2);
        fs::create_dir_all(fx.root().join("empty")).unwrap();

        let stats = fx.scan();
        assert_eq!(stats.media_added, 3);
        assert_eq!(stats.folders_added, 5); // root, 2023, 2023/trip, 2024, empty
        let names: Vec<String> = fx.query("SELECT name FROM media ORDER BY name");
        assert_eq!(names, ["a.jpg", "b.jpg", "c.png"]);

        // Folder aggregates: newest photo is the cover.
        let trip = format!("{}/2023/trip", fx.root().display());
        let conn = fx.db.conn().unwrap();
        let (count, latest, cover): (i64, i64, String) = conn
            .query_row(
                "SELECT f.media_count, f.latest_taken_at, m.name FROM folders f
                 JOIN media m ON m.id = f.cover_media_id WHERE f.path = ?1",
                [&trip],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((count, latest, cover.as_str()), (2, 1_686_906_000, "b.jpg"));
    }

    #[test]
    fn folders_with_ignore_markers_are_left_out() {
        let fx = Fixture::new();
        fx.write("keep/a.jpg", &testdata::jpeg_with_exif());
        fx.write("private/b.jpg", &testdata::jpeg_with_exif());
        fx.write("private/deeper/c.jpg", &testdata::jpeg_with_exif());
        fx.write("dev/d.jpg", &testdata::jpeg_with_exif());
        fx.scan();
        assert_eq!(fx.query::<i64>("SELECT count(*) FROM media"), [4]);

        // Adding markers removes those folders and everything below them.
        fx.write("private/.nomedia", b"");
        fx.write("dev/.ignore", b"");
        let stats = fx.scan();
        assert_eq!(stats.media_removed, 3);
        let names: Vec<String> = fx.query("SELECT name FROM media");
        assert_eq!(names, ["a.jpg"]);
        let folders: Vec<String> = fx.query("SELECT name FROM folders WHERE parent_id IS NOT NULL");
        assert_eq!(folders, ["keep"]);

        // A rescan of just a subfolder of an ignored folder adds nothing back.
        fx.scan_at(&fx.root().join("private/deeper"));
        assert_eq!(fx.query::<i64>("SELECT count(*) FROM media"), [1]);

        // Removing the marker brings them back.
        fs::remove_file(fx.root().join("private/.nomedia")).unwrap();
        fx.scan();
        assert_eq!(fx.query::<i64>("SELECT count(*) FROM media"), [3]);
    }

    #[test]
    fn ignore_marker_at_the_root_clears_the_library() {
        let fx = Fixture::new();
        fx.write("a.jpg", &testdata::jpeg_with_exif());
        fx.scan();
        fx.write(".nomedia", b"");
        fx.scan();
        assert_eq!(fx.query::<i64>("SELECT count(*) FROM media"), [0]);
    }

    #[test]
    fn rescan_without_changes_reads_nothing() {
        let fx = Fixture::new();
        fx.write("a.jpg", &testdata::jpeg_with_exif());
        fx.scan();
        let stats = fx.scan();
        assert!(!stats.changed());
        assert_eq!(stats.media_unchanged, 1);
    }

    #[test]
    fn detects_changes_and_removals() {
        let fx = Fixture::new();
        fx.write(
            "keep/a.jpg",
            &testdata::jpeg_taken_at("2020:01:01 00:00:00"),
        );
        fx.write(
            "keep/b.jpg",
            &testdata::jpeg_taken_at("2020:01:02 00:00:00"),
        );
        fx.write("gone/c.jpg", &testdata::jpeg_with_exif());
        fx.scan();
        let id_before: Vec<i64> = fx.query("SELECT id FROM media WHERE name = 'a.jpg'");

        fx.write(
            "keep/a.jpg",
            &testdata::jpeg_taken_at("2021:01:01 00:00:00"),
        );
        // Same size as before, so make sure the mtime differs.
        let file = fs::File::options()
            .write(true)
            .open(fx.root().join("keep/a.jpg"))
            .unwrap();
        file.set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000))
            .unwrap();
        fs::remove_file(fx.root().join("keep/b.jpg")).unwrap();
        fs::remove_dir_all(fx.root().join("gone")).unwrap();
        let stats = fx.scan();

        assert_eq!(stats.media_updated, 1);
        assert_eq!(stats.media_removed, 2);
        assert_eq!(stats.folders_removed, 1);
        let id_after: Vec<i64> = fx.query("SELECT id FROM media WHERE name = 'a.jpg'");
        assert_eq!(id_before, id_after, "updates keep the media id");
        let counts: Vec<i64> = fx.query("SELECT media_count FROM folders WHERE name = 'keep'");
        assert_eq!(counts, [1]);
    }

    #[test]
    fn skips_hidden_and_excluded() {
        let fx = Fixture::new();
        fx.write(".hidden/a.jpg", &testdata::jpeg_with_exif());
        fx.write("@eaDir/b.jpg", &testdata::jpeg_with_exif());
        fx.write("._c.jpg", &testdata::jpeg_with_exif());
        fx.write("ok/d.jpg", &testdata::jpeg_with_exif());
        fx.scan();
        let names: Vec<String> = fx.query("SELECT name FROM media");
        assert_eq!(names, ["d.jpg"]);
    }

    #[test]
    fn subtree_scan_only_touches_subtree() {
        let fx = Fixture::new();
        fx.write("a/1.jpg", &testdata::jpeg_with_exif());
        fx.write("b/2.jpg", &testdata::jpeg_with_exif());
        fx.scan();

        // Changes in both folders, but only `a` is rescanned.
        fx.write("a/new.jpg", &testdata::jpeg_with_exif());
        fs::remove_file(fx.root().join("b/2.jpg")).unwrap();
        let stats = fx.scan_at(&fx.root().join("a"));
        assert_eq!((stats.media_added, stats.media_removed), (1, 0));
        let names: Vec<String> = fx.query("SELECT name FROM media ORDER BY name");
        assert_eq!(names, ["1.jpg", "2.jpg", "new.jpg"]);
    }

    #[test]
    fn subtree_scan_of_new_nested_dir_climbs_to_known_ancestor() {
        let fx = Fixture::new();
        fx.write("a/1.jpg", &testdata::jpeg_with_exif());
        fx.scan();
        fx.write("a/x/y/2.jpg", &testdata::jpeg_with_exif());
        let stats = fx.scan_at(&fx.root().join("a/x/y"));
        assert_eq!((stats.folders_added, stats.media_added), (2, 1));
        let parents: Vec<Option<i64>> = fx.query("SELECT parent_id FROM folders WHERE name = 'y'");
        assert!(parents[0].is_some());
    }

    #[test]
    fn similar_prefixes_are_not_in_subtree() {
        let fx = Fixture::new();
        fx.write("a/1.jpg", &testdata::jpeg_with_exif());
        fx.write("a b/2.jpg", &testdata::jpeg_with_exif());
        fx.write("ab/3.jpg", &testdata::jpeg_with_exif());
        fx.scan();
        fs::remove_dir_all(fx.root().join("a")).unwrap();
        fs::create_dir(fx.root().join("a")).unwrap();
        let stats = fx.scan_at(&fx.root().join("a"));
        assert_eq!(stats.media_removed, 1);
        let names: Vec<String> = fx.query("SELECT name FROM media ORDER BY name");
        assert_eq!(names, ["2.jpg", "3.jpg"]);
    }

    #[test]
    fn missing_or_empty_root_keeps_index() {
        let fx = Fixture::new();
        fx.write("album/a.jpg", &testdata::jpeg_with_exif());
        fx.scan();

        fs::remove_dir_all(fx.root().join("album")).unwrap();
        assert!(!fx.scan().changed(), "empty root must not wipe the index");

        fs::remove_dir(fx.root()).unwrap();
        assert!(!fx.scan().changed(), "missing root must not wipe the index");
        let count: Vec<i64> = fx.query("SELECT count(*) FROM media");
        assert_eq!(count, [1]);

        // Media deleted from a still-populated root is removed as usual.
        fx.write("album/a.jpg", &testdata::jpeg_with_exif());
        fx.write("album/b.jpg", &testdata::jpeg_with_exif());
        fx.scan();
        fs::remove_file(fx.root().join("album/a.jpg")).unwrap();
        assert_eq!(fx.scan().media_removed, 1);
    }

    #[test]
    fn removing_a_root_removes_its_contents() {
        let fx = Fixture::new();
        fx.write("a.jpg", &testdata::jpeg_with_exif());
        fx.scan();
        sync_roots(&fx.db, &[]).unwrap();
        let count: Vec<i64> = fx.query("SELECT count(*) FROM media");
        assert_eq!(count, [0]);
    }
}
