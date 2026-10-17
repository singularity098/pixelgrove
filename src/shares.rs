//! Public share links for a single item or a folder (including everything below it).

use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct Share {
    pub id: i64,
    pub token: String,
    pub media_id: Option<i64>,
    pub folder_path: Option<String>,
    pub created_by: i64,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub password_hash: Option<String>,
    pub allow_download: bool,
    pub views: i64,
    pub last_viewed_at: Option<i64>,
}

impl Share {
    pub fn is_expired(&self, now: i64) -> bool {
        self.expires_at.is_some_and(|at| at <= now)
    }

    /// Whether a library path (a folder, or a file's folder) is inside this share.
    pub fn covers_folder(&self, folder: &str) -> bool {
        self.folder_path
            .as_deref()
            .is_some_and(|root| Path::new(folder).starts_with(root))
    }
}

const COLUMNS: &str = "id, token, media_id, folder_path, created_by, created_at, expires_at,
     password_hash, allow_download, views, last_viewed_at";

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Share> {
    Ok(Share {
        id: r.get(0)?,
        token: r.get(1)?,
        media_id: r.get(2)?,
        folder_path: r.get(3)?,
        created_by: r.get(4)?,
        created_at: r.get(5)?,
        expires_at: r.get(6)?,
        password_hash: r.get(7)?,
        allow_download: r.get(8)?,
        views: r.get(9)?,
        last_viewed_at: r.get(10)?,
    })
}

/// 128 random bits as hex: unguessable, and URL-safe.
fn new_token() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("getrandom: {e}"))?;
    Ok(hex::encode(bytes))
}

pub struct NewShare<'a> {
    pub media_id: Option<i64>,
    pub folder_path: Option<&'a str>,
    pub created_by: i64,
    pub expires_at: Option<i64>,
    pub password: Option<&'a str>,
    pub allow_download: bool,
}

pub fn create(conn: &Connection, new: &NewShare) -> Result<Share> {
    let password_hash = new.password.map(crate::auth::hash_password).transpose()?;
    let kind = if new.media_id.is_some() {
        "media"
    } else {
        "folder"
    };
    conn.execute(
        "INSERT INTO shares (token, kind, media_id, folder_path, created_by, created_at,
             expires_at, password_hash, allow_download)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            new_token()?,
            kind,
            new.media_id,
            new.folder_path,
            new.created_by,
            crate::auth::now(),
            new.expires_at,
            password_hash,
            new.allow_download,
        ],
    )?;
    Ok(get(conn, conn.last_insert_rowid())?.expect("just inserted"))
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<Share>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM shares WHERE id = ?1"),
            [id],
            from_row,
        )
        .optional()?)
}

pub fn by_token(conn: &Connection, token: &str) -> Result<Option<Share>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM shares WHERE token = ?1"),
            [token],
            from_row,
        )
        .optional()?)
}

/// Shares created by `user_id`, or all shares when `None`; newest first.
pub fn list(conn: &Connection, user_id: Option<i64>) -> Result<Vec<Share>> {
    Ok(conn
        .prepare(&format!(
            "SELECT {COLUMNS} FROM shares WHERE ?1 IS NULL OR created_by = ?1
             ORDER BY created_at DESC, id DESC"
        ))?
        .query_map([user_id], from_row)?
        .collect::<rusqlite::Result<_>>()?)
}

pub struct Changes<'a> {
    /// `Some(None)` removes the expiry.
    pub expires_at: Option<Option<i64>>,
    /// `Some(None)` removes the password.
    pub password: Option<Option<&'a str>>,
    pub allow_download: Option<bool>,
}

pub fn update(conn: &Connection, id: i64, changes: &Changes) -> Result<()> {
    if let Some(expires_at) = changes.expires_at {
        conn.execute(
            "UPDATE shares SET expires_at = ?1 WHERE id = ?2",
            params![expires_at, id],
        )?;
    }
    if let Some(password) = changes.password {
        let hash = password.map(crate::auth::hash_password).transpose()?;
        conn.execute(
            "UPDATE shares SET password_hash = ?1 WHERE id = ?2",
            params![hash, id],
        )?;
    }
    if let Some(allow) = changes.allow_download {
        conn.execute(
            "UPDATE shares SET allow_download = ?1 WHERE id = ?2",
            params![allow, id],
        )?;
    }
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM shares WHERE id = ?1", [id])?;
    Ok(())
}

pub fn record_view(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE shares SET views = views + 1, last_viewed_at = ?1 WHERE id = ?2",
        params![crate::auth::now(), id],
    )?;
    Ok(())
}

/// The server's secret for signing unlock cookies, created on first use.
pub fn secret(conn: &Connection) -> Result<String> {
    if let Some(secret) = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'share_secret'",
            [],
            |r| r.get(0),
        )
        .optional()?
    {
        return Ok(secret);
    }
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("getrandom: {e}"))?;
    conn.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('share_secret', ?1)",
        [hex::encode(bytes)],
    )?;
    // Re-read in case another request created it first.
    Ok(conn.query_row(
        "SELECT value FROM settings WHERE key = 'share_secret'",
        [],
        |r| r.get(0),
    )?)
}

/// The cookie value proving a visitor entered the password for `share`. Derived from
/// the current password hash, so changing the password locks out earlier visitors.
pub fn unlock_value(secret: &str, share: &Share) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(share.id.to_le_bytes());
    hasher.update(share.password_hash.as_deref().unwrap_or("").as_bytes());
    hex::encode(hasher.finalize())
}

pub fn cookie_name(share: &Share) -> String {
    format!("pixelgrove_share_{}", share.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("t.db")).unwrap();
        db.conn()
            .unwrap()
            .execute(
                "INSERT INTO users (id, username, password_hash, role, created_at)
                 VALUES (1, 'u', 'x', 'admin', 0)",
                [],
            )
            .unwrap();
        (dir, db)
    }

    fn folder_share(conn: &Connection, password: Option<&str>) -> Share {
        create(
            conn,
            &NewShare {
                media_id: None,
                folder_path: Some("/media/trip"),
                created_by: 1,
                expires_at: None,
                password,
                allow_download: true,
            },
        )
        .unwrap()
    }

    #[test]
    fn tokens_are_unique_and_unguessable() {
        let (_dir, db) = db();
        let conn = db.conn().unwrap();
        let a = folder_share(&conn, None);
        let b = folder_share(&conn, None);
        assert_ne!(a.token, b.token);
        assert_eq!(a.token.len(), 32);
        assert_eq!(by_token(&conn, &a.token).unwrap().unwrap().id, a.id);
    }

    #[test]
    fn folder_coverage() {
        let (_dir, db) = db();
        let share = folder_share(&db.conn().unwrap(), None);
        assert!(share.covers_folder("/media/trip"));
        assert!(share.covers_folder("/media/trip/day 1"));
        assert!(!share.covers_folder("/media/trips"));
        assert!(!share.covers_folder("/media"));
    }

    #[test]
    fn changing_the_password_changes_the_unlock_cookie() {
        let (_dir, db) = db();
        let conn = db.conn().unwrap();
        let secret = secret(&conn).unwrap();
        assert_eq!(secret, super::secret(&conn).unwrap(), "secret is stable");
        let share = folder_share(&conn, Some("first"));
        let before = unlock_value(&secret, &share);
        update(
            &conn,
            share.id,
            &Changes {
                expires_at: None,
                password: Some(Some("second")),
                allow_download: None,
            },
        )
        .unwrap();
        let share = get(&conn, share.id).unwrap().unwrap();
        assert_ne!(before, unlock_value(&secret, &share));
        assert!(crate::auth::verify_password(
            "second",
            share.password_hash.as_deref().unwrap()
        ));
    }

    #[test]
    fn expiry() {
        let (_dir, db) = db();
        let mut share = folder_share(&db.conn().unwrap(), None);
        assert!(!share.is_expired(100));
        share.expires_at = Some(100);
        assert!(share.is_expired(100));
        assert!(!share.is_expired(99));
    }
}
