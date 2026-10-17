//! Trash: deleted media is moved into a hidden `.pixelgrove-trash` folder on the same
//! volume (so nothing is copied), can be restored to where it was, and is removed for
//! good after [`RETENTION`].

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

/// Hidden, so the scanner skips it.
pub const TRASH_DIR: &str = ".pixelgrove-trash";
pub const RETENTION: Duration = Duration::from_secs(30 * 24 * 3600);

/// Why a file couldn't be moved.
#[derive(Debug, PartialEq)]
pub enum MoveError {
    /// The volume is mounted read-only or we lack permission.
    ReadOnly,
    /// The file is no longer where the index says.
    Missing,
}

/// Where `original` (inside `root`) goes in the trash. Mirrors the folder structure so
/// restoring is unambiguous and trashed files are easy to find by hand.
fn trash_location(root: &Path, original: &Path) -> PathBuf {
    let relative = original.strip_prefix(root).unwrap_or(original);
    root.join(TRASH_DIR).join(relative)
}

/// `path`, or `name (2).ext`, `name (3).ext`, … if it already exists.
fn unused_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (2..)
        .map(|n| path.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|candidate| !candidate.exists())
        .expect("some suffix is free")
}

/// Moves `from` to `to`, creating `to`'s folder. Falls back to a trash folder next to
/// the file when `to` is on another device (e.g. a nested mount inside the root).
fn move_file(from: &Path, to: &Path) -> Result<PathBuf, io::Error> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let to = unused_path(to);
    std::fs::rename(from, &to)?;
    Ok(to)
}

fn classify(err: &io::Error) -> Option<MoveError> {
    match err.kind() {
        io::ErrorKind::NotFound => Some(MoveError::Missing),
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
            Some(MoveError::ReadOnly)
        }
        _ => None,
    }
}

/// Media details captured when it's trashed, for listing and restoring.
pub struct Trashed<'a> {
    pub root_id: i64,
    pub root: &'a Path,
    pub original: &'a Path,
    pub name: &'a str,
    pub kind: &'a str,
    pub size: i64,
    pub mtime: i64,
    pub taken_at: i64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub orientation: Option<u16>,
    pub duration_ms: Option<i64>,
    pub deleted_by: i64,
}

/// Moves the file into the trash and records it. The caller rescans the original
/// folder so the index drops it.
pub fn move_to_trash(
    conn: &Connection,
    item: &Trashed,
) -> Result<std::result::Result<i64, MoveError>> {
    let target = trash_location(item.root, item.original);
    let moved = match move_file(item.original, &target) {
        Ok(path) => path,
        // A nested mount: keep the trash on the file's own volume.
        Err(err) if err.kind() == io::ErrorKind::CrossesDevices => {
            let folder = item.original.parent().unwrap_or(item.root);
            match move_file(item.original, &folder.join(TRASH_DIR).join(item.name)) {
                Ok(path) => path,
                Err(err) => return classified(err),
            }
        }
        Err(err) => return classified(err),
    };
    conn.execute(
        "INSERT INTO trash (root_id, original_path, trash_path, name, kind, size, mtime,
             taken_at, width, height, orientation, duration_ms, deleted_at, deleted_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            item.root_id,
            path_str(item.original)?,
            path_str(&moved)?,
            item.name,
            item.kind,
            item.size,
            item.mtime,
            item.taken_at,
            item.width,
            item.height,
            item.orientation,
            item.duration_ms,
            crate::auth::now(),
            item.deleted_by,
        ],
    )?;
    Ok(Ok(conn.last_insert_rowid()))
}

fn classified<T>(err: io::Error) -> Result<std::result::Result<T, MoveError>> {
    match classify(&err) {
        Some(kind) => Ok(Err(kind)),
        None => Err(err).context("moving file"),
    }
}

#[derive(Debug, Clone)]
pub struct TrashItem {
    pub id: i64,
    pub original_path: String,
    pub trash_path: String,
    pub name: String,
    pub kind: String,
    pub size: i64,
    pub mtime: i64,
    pub taken_at: i64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub orientation: Option<u16>,
    pub duration_ms: Option<i64>,
    pub deleted_at: i64,
}

pub const TRASH_COLUMNS: &str = "id, original_path, trash_path, name, kind, size, mtime, taken_at,
     width, height, orientation, duration_ms, deleted_at";

pub fn item_from_row(r: &rusqlite::Row) -> rusqlite::Result<TrashItem> {
    Ok(TrashItem {
        id: r.get(0)?,
        original_path: r.get(1)?,
        trash_path: r.get(2)?,
        name: r.get(3)?,
        kind: r.get(4)?,
        size: r.get(5)?,
        mtime: r.get(6)?,
        taken_at: r.get(7)?,
        width: r.get(8)?,
        height: r.get(9)?,
        orientation: r.get(10)?,
        duration_ms: r.get(11)?,
        deleted_at: r.get(12)?,
    })
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<TrashItem>> {
    Ok(conn
        .query_row(
            &format!("SELECT {TRASH_COLUMNS} FROM trash WHERE id = ?1"),
            [id],
            item_from_row,
        )
        .optional()?)
}

/// Moves a trashed file back. Returns where it went (with a numbered name if
/// something new took its place); the caller rescans that folder.
pub fn restore(
    conn: &Connection,
    item: &TrashItem,
) -> Result<std::result::Result<PathBuf, MoveError>> {
    let restored = match move_file(Path::new(&item.trash_path), Path::new(&item.original_path)) {
        Ok(path) => path,
        Err(err) => return classified(err),
    };
    conn.execute("DELETE FROM trash WHERE id = ?1", [item.id])?;
    Ok(Ok(restored))
}

/// Deletes a trashed file for good. A file that's already gone just drops its record.
pub fn delete_forever(conn: &Connection, item: &TrashItem) -> Result<Result<(), MoveError>> {
    match std::fs::remove_file(&item.trash_path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return classified(err),
    }
    conn.execute("DELETE FROM trash WHERE id = ?1", [item.id])?;
    Ok(Ok(()))
}

/// Deletes items trashed more than [`RETENTION`] ago. Returns how many were removed.
pub fn purge_expired(conn: &Connection) -> Result<usize> {
    let cutoff = crate::auth::now() - RETENTION.as_secs() as i64;
    let expired: Vec<TrashItem> = conn
        .prepare(&format!(
            "SELECT {TRASH_COLUMNS} FROM trash WHERE deleted_at <= ?1"
        ))?
        .query_map([cutoff], item_from_row)?
        .collect::<rusqlite::Result<_>>()?;
    let mut removed = 0;
    for item in &expired {
        match delete_forever(conn, item)? {
            Ok(()) => removed += 1,
            Err(err) => {
                tracing::warn!(path = %item.trash_path, ?err, "couldn't purge trashed file")
            }
        }
    }
    Ok(removed)
}

fn path_str(path: &Path) -> Result<&str> {
    path.to_str()
        .with_context(|| format!("path is not valid UTF-8: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    struct Fixture {
        _dir: tempfile::TempDir,
        media: tempfile::TempDir,
        db: Db,
        root_id: i64,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let media = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("t.db")).unwrap();
        let roots = crate::scanner::sync_roots(&db, &[media.path().to_path_buf()]).unwrap();
        Fixture {
            root_id: roots[0].0,
            _dir: dir,
            media,
            db,
        }
    }

    fn trash(fx: &Fixture, relative: &str) -> std::result::Result<i64, MoveError> {
        let original = fx.media.path().join(relative);
        let conn = fx.db.conn().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO users (id, username, password_hash, role, created_at)
             VALUES (1, 'u', 'x', 'admin', 0)",
            [],
        )
        .unwrap();
        move_to_trash(
            &conn,
            &Trashed {
                root_id: fx.root_id,
                root: fx.media.path(),
                original: &original,
                name: original.file_name().unwrap().to_str().unwrap(),
                kind: "image",
                size: 1,
                mtime: 0,
                taken_at: 0,
                width: None,
                height: None,
                orientation: None,
                duration_ms: None,
                deleted_by: 1,
            },
        )
        .unwrap()
    }

    fn write(fx: &Fixture, relative: &str, contents: &str) {
        let path = fx.media.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn trash_and_restore_round_trip() {
        let fx = fixture();
        write(&fx, "a/photo.jpg", "one");
        let id = trash(&fx, "a/photo.jpg").unwrap();
        assert!(!fx.media.path().join("a/photo.jpg").exists());
        let trashed = fx.media.path().join(".pixelgrove-trash/a/photo.jpg");
        assert_eq!(std::fs::read_to_string(&trashed).unwrap(), "one");

        let conn = fx.db.conn().unwrap();
        let item = get(&conn, id).unwrap().unwrap();
        let restored = restore(&conn, &item).unwrap().unwrap();
        assert_eq!(restored, fx.media.path().join("a/photo.jpg"));
        assert!(get(&conn, id).unwrap().is_none());
    }

    #[test]
    fn name_clashes_get_numbered() {
        let fx = fixture();
        write(&fx, "a/photo.jpg", "one");
        trash(&fx, "a/photo.jpg").unwrap();
        write(&fx, "a/photo.jpg", "two");
        let second = trash(&fx, "a/photo.jpg").unwrap();
        assert!(
            fx.media
                .path()
                .join(".pixelgrove-trash/a/photo (2).jpg")
                .exists()
        );

        // Restoring when a new file took the old name doesn't overwrite it.
        write(&fx, "a/photo.jpg", "three");
        let conn = fx.db.conn().unwrap();
        let item = get(&conn, second).unwrap().unwrap();
        let restored = restore(&conn, &item).unwrap().unwrap();
        assert_eq!(restored, fx.media.path().join("a/photo (2).jpg"));
        assert_eq!(
            std::fs::read_to_string(fx.media.path().join("a/photo.jpg")).unwrap(),
            "three"
        );
    }

    #[test]
    fn missing_file_is_reported() {
        let fx = fixture();
        assert_eq!(trash(&fx, "nope.jpg"), Err(MoveError::Missing));
    }

    #[test]
    fn delete_forever_and_purge() {
        let fx = fixture();
        write(&fx, "old.jpg", "x");
        write(&fx, "new.jpg", "y");
        let old = trash(&fx, "old.jpg").unwrap();
        let new = trash(&fx, "new.jpg").unwrap();
        let conn = fx.db.conn().unwrap();
        conn.execute(
            "UPDATE trash SET deleted_at = ?1 WHERE id = ?2",
            params![crate::auth::now() - RETENTION.as_secs() as i64 - 1, old],
        )
        .unwrap();
        assert_eq!(purge_expired(&conn).unwrap(), 1);
        assert!(get(&conn, old).unwrap().is_none());
        assert!(!fx.media.path().join(".pixelgrove-trash/old.jpg").exists());

        let item = get(&conn, new).unwrap().unwrap();
        delete_forever(&conn, &item).unwrap().unwrap();
        assert!(!fx.media.path().join(".pixelgrove-trash/new.jpg").exists());
    }

    #[test]
    fn read_only_files_are_reported() {
        use std::os::unix::fs::PermissionsExt;
        let fx = fixture();
        write(&fx, "locked/photo.jpg", "x");
        let locked = fx.media.path().join("locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = trash(&fx, "locked/photo.jpg");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Root ignores permissions, so only check when running unprivileged.
        if !rustix::process::geteuid().is_root() {
            assert_eq!(result, Err(MoveError::ReadOnly));
        }
    }
}
