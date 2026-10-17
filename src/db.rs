//! SQLite database: connection pool and schema migrations.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;

pub const DB_FILE_NAME: &str = "pixelgrove.db";

pub type Conn = PooledConnection<SqliteConnectionManager>;

/// Schema migrations, applied in order. `PRAGMA user_version` records how many have run.
const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE roots (
    id    INTEGER PRIMARY KEY,
    path  TEXT NOT NULL UNIQUE
);

-- Every directory under a root, including ones without media (File View needs them).
-- Aggregates cover media directly inside the folder and are maintained by the indexer.
CREATE TABLE folders (
    id              INTEGER PRIMARY KEY,
    root_id         INTEGER NOT NULL REFERENCES roots(id) ON DELETE CASCADE,
    parent_id       INTEGER REFERENCES folders(id) ON DELETE CASCADE,
    path            TEXT NOT NULL UNIQUE,
    name            TEXT NOT NULL,
    media_count     INTEGER NOT NULL DEFAULT 0,
    latest_taken_at INTEGER,
    cover_media_id  INTEGER
);
CREATE INDEX folders_parent ON folders(parent_id);
CREATE INDEX folders_latest ON folders(latest_taken_at DESC) WHERE media_count > 0;

CREATE TABLE media (
    id          INTEGER PRIMARY KEY,
    folder_id   INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('image', 'video')),
    size        INTEGER NOT NULL,
    mtime       INTEGER NOT NULL,
    -- Capture time as wall-clock seconds since the epoch (EXIF times usually have no zone);
    -- falls back to mtime.
    taken_at    INTEGER NOT NULL,
    tz_offset   INTEGER,
    width       INTEGER,
    height      INTEGER,
    orientation INTEGER,
    duration_ms INTEGER,
    camera_make  TEXT,
    camera_model TEXT,
    lens         TEXT,
    latitude    REAL,
    longitude   REAL,
    description TEXT,
    UNIQUE (folder_id, name)
);
CREATE INDEX media_folder_taken ON media(folder_id, taken_at DESC);
CREATE INDEX media_taken ON media(taken_at DESC);
"#,
    r#"
CREATE TABLE users (
    id            INTEGER PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash TEXT NOT NULL,
    role          TEXT NOT NULL CHECK (role IN ('admin', 'editor', 'viewer')),
    disabled      INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL
);

-- Only a SHA-256 of each session token is stored, so a leaked database can't be
-- used to hijack sessions.
CREATE TABLE sessions (
    token_hash   BLOB PRIMARY KEY,
    user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at   INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL
) WITHOUT ROWID;
CREATE INDEX sessions_user ON sessions(user_id);

-- Folders a user is limited to (they see these and everything below them).
-- No rows means the whole library.
CREATE TABLE user_folders (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    path    TEXT NOT NULL,
    PRIMARY KEY (user_id, path)
) WITHOUT ROWID;
"#,
    r#"
-- What search matches against, built at scan time (see scanner::search_text) so a
-- search is one pass over a single column.
ALTER TABLE media ADD COLUMN search_text TEXT NOT NULL DEFAULT '';
UPDATE media SET search_text = lower(
    name || ' ' ||
    coalesce((SELECT substr(f.path, length(r.path) + 2) FROM folders f
              JOIN roots r ON r.id = f.root_id WHERE f.id = media.folder_id), '') || ' ' ||
    coalesce(camera_make, '') || ' ' || coalesce(camera_model, '') || ' ' ||
    coalesce(lens, '') || ' ' || coalesce(description, '') || ' ' ||
    strftime('%Y-%m-%d', taken_at, 'unixepoch'));
"#,
    r#"
CREATE TABLE favorites (
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    media_id   INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, media_id)
) WITHOUT ROWID;
-- Needed so deleting media (e.g. a whole folder) doesn't scan favorites per row.
CREATE INDEX favorites_media ON favorites(media_id);

-- Items moved to the trash folder, with enough metadata to show and restore them.
CREATE TABLE trash (
    id            INTEGER PRIMARY KEY,
    root_id       INTEGER NOT NULL REFERENCES roots(id) ON DELETE CASCADE,
    original_path TEXT NOT NULL,
    trash_path    TEXT NOT NULL,
    name          TEXT NOT NULL,
    kind          TEXT NOT NULL,
    size          INTEGER NOT NULL,
    mtime         INTEGER NOT NULL,
    taken_at      INTEGER NOT NULL,
    width         INTEGER,
    height        INTEGER,
    orientation   INTEGER,
    duration_ms   INTEGER,
    deleted_at    INTEGER NOT NULL,
    deleted_by    INTEGER REFERENCES users(id) ON DELETE SET NULL
);
CREATE INDEX trash_deleted ON trash(deleted_at);
"#,
    r#"
-- Server-wide values, e.g. the secret that signs share-link unlock cookies.
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;

-- Public links to a single item or a folder (and everything below it).
CREATE TABLE shares (
    id             INTEGER PRIMARY KEY,
    token          TEXT NOT NULL UNIQUE,
    kind           TEXT NOT NULL CHECK (kind IN ('media', 'folder')),
    media_id       INTEGER REFERENCES media(id) ON DELETE CASCADE,
    -- Folders are shared by path, so the link survives rescans.
    folder_path    TEXT,
    created_by     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at     INTEGER NOT NULL,
    expires_at     INTEGER,
    password_hash  TEXT,
    allow_download INTEGER NOT NULL DEFAULT 1,
    views          INTEGER NOT NULL DEFAULT 0,
    last_viewed_at INTEGER,
    CHECK ((kind = 'media') = (media_id IS NOT NULL)),
    CHECK ((kind = 'folder') = (folder_path IS NOT NULL))
);
CREATE INDEX shares_media ON shares(media_id);
CREATE INDEX shares_creator ON shares(created_by);
"#,
];

#[derive(Clone)]
pub struct Db {
    pool: Pool<SqliteConnectionManager>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let manager = SqliteConnectionManager::file(path).with_init(configure);
        Self::from_manager(manager, 8)
            .with_context(|| format!("opening database {}", path.display()))
    }

    fn from_manager(manager: SqliteConnectionManager, size: u32) -> Result<Self> {
        let pool = Pool::builder()
            .max_size(size)
            .connection_timeout(Duration::from_secs(30))
            .build(manager)?;
        migrate(&mut *pool.get()?)?;
        Ok(Self { pool })
    }

    pub fn conn(&self) -> Result<Conn> {
        Ok(self.pool.get()?)
    }

    /// Runs `f` with a pooled connection on the blocking thread pool.
    pub async fn run<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let db = self.clone();
        tokio::task::spawn_blocking(move || f(&mut *db.conn()?)).await?
    }
}

fn configure(conn: &mut Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(Duration::from_secs(30))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA temp_store = MEMORY;
         PRAGMA cache_size = -4000;",
    )
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let applied: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(applied as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)
            .with_context(|| format!("applying migration {}", i + 1))?;
        tx.pragma_update(None, "user_version", (i + 1) as u32)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_and_are_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DB_FILE_NAME);
        Db::open(&path).unwrap();
        let db = Db::open(&path).unwrap();
        let version: u32 = db
            .conn()
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
    }
}
