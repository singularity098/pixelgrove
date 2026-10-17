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
const MIGRATIONS: &[&str] = &[r#"
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
"#];

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
