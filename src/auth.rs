//! Users, passwords and sessions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Sessions expire after this long without use.
pub const SESSION_IDLE_LIFETIME: Duration = Duration::from_secs(30 * 24 * 3600);
/// `last_seen_at` is only rewritten when older than this, to avoid a write per request.
const SESSION_TOUCH_INTERVAL: i64 = 3600;
pub const MIN_PASSWORD_LEN: usize = 8;
const MAX_USERNAME_LEN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Browse and download only; no changeable options.
    Viewer,
    /// Viewer, plus favorites, sharing and trash.
    Editor,
    /// Everything, including settings and user management.
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Editor => "editor",
            Role::Admin => "admin",
        }
    }

    fn parse(s: &str) -> Result<Self> {
        match s {
            "viewer" => Ok(Role::Viewer),
            "editor" => Ok(Role::Editor),
            "admin" => Ok(Role::Admin),
            other => anyhow::bail!("unknown role {other}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub role: Role,
    pub disabled: bool,
    pub created_at: i64,
    /// Folders this user is limited to; empty means the whole library.
    pub folders: Vec<String>,
}

impl User {
    /// Whether this user may see `path` (a folder or file in the library).
    pub fn can_access(&self, path: &Path) -> bool {
        self.folders.is_empty() || self.folders.iter().any(|f| path.starts_with(f))
    }
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

// ---- Passwords ----

pub fn hash_password(password: &str) -> Result<String> {
    Ok(Argon2::default()
        .hash_password(password.as_bytes())
        .map_err(|e| anyhow::anyhow!("hashing password: {e}"))?
        .to_string())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

/// Verifies against a fixed hash so that logins for unknown users take as long as
/// logins with a wrong password, and don't reveal which usernames exist.
pub fn burn_password_check(password: &str) {
    static DUMMY: LazyLock<String> =
        LazyLock::new(|| hash_password("not-a-real-password").expect("hashing works"));
    verify_password(password, &DUMMY);
}

// ---- Validation ----

/// Trims and checks a username; returns the cleaned value or a user-facing error.
pub fn clean_username(username: &str) -> Result<String, &'static str> {
    let username = username.trim();
    if username.is_empty() {
        return Err("Username is required.");
    }
    if username.chars().count() > MAX_USERNAME_LEN {
        return Err("Username is too long (64 characters at most).");
    }
    if username.chars().any(char::is_control) {
        return Err("Username contains invalid characters.");
    }
    Ok(username.to_string())
}

pub fn check_password(password: &str) -> Result<(), &'static str> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err("Password must be at least 8 characters.");
    }
    Ok(())
}

/// Normalises folder restrictions: each must be an absolute path inside one of the
/// media roots. Returns a user-facing error naming the first bad path.
pub fn clean_folders(folders: &[String], roots: &[PathBuf]) -> Result<Vec<String>, String> {
    let mut cleaned = Vec::new();
    for folder in folders {
        let trimmed = folder.trim();
        let path = Path::new(trimmed);
        let inside_root = path.is_absolute()
            && !path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            && roots.iter().any(|r| path.starts_with(r));
        if !inside_root {
            return Err(format!("{trimmed} is not a folder in the library."));
        }
        let normalised = trimmed.trim_end_matches('/').to_string();
        if !cleaned.contains(&normalised) {
            cleaned.push(normalised);
        }
    }
    Ok(cleaned)
}

// ---- Users ----

pub fn user_count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT count(*) FROM users", [], |r| r.get(0))?)
}

pub struct NewUser<'a> {
    pub username: &'a str,
    pub password: &'a str,
    pub role: Role,
    pub folders: &'a [String],
}

/// Creates a user. Returns `None` if the username is taken. Inputs must already be
/// validated.
pub fn create_user(conn: &mut Connection, new: &NewUser) -> Result<Option<User>> {
    let hash = hash_password(new.password)?;
    let tx = conn.transaction()?;
    let inserted = tx.execute(
        "INSERT INTO users (username, password_hash, role, created_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (username) DO NOTHING",
        params![new.username, hash, new.role.as_str(), now()],
    )?;
    if inserted == 0 {
        return Ok(None);
    }
    let id = tx.last_insert_rowid();
    set_folders(&tx, id, new.folders)?;
    tx.commit()?;
    get_user(conn, id)
}

/// Creates the first admin, but only if there are no users yet (atomically, so two
/// concurrent setup requests can't both succeed).
pub fn create_first_admin(
    conn: &mut Connection,
    username: &str,
    password: &str,
) -> Result<Option<User>> {
    let hash = hash_password(password)?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if user_count(&tx)? > 0 {
        return Ok(None);
    }
    tx.execute(
        "INSERT INTO users (username, password_hash, role, created_at) VALUES (?1, ?2, 'admin', ?3)",
        params![username, hash, now()],
    )?;
    let id = tx.last_insert_rowid();
    tx.commit()?;
    get_user(conn, id)
}

pub fn get_user(conn: &Connection, id: i64) -> Result<Option<User>> {
    let row = conn
        .query_row(
            "SELECT id, username, role, disabled, created_at FROM users WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, bool>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((id, username, role, disabled, created_at)) = row else {
        return Ok(None);
    };
    Ok(Some(User {
        id,
        username,
        role: Role::parse(&role)?,
        disabled,
        created_at,
        folders: folders_of(conn, id)?,
    }))
}

pub fn list_users(conn: &Connection) -> Result<Vec<User>> {
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM users ORDER BY username")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut users = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(user) = get_user(conn, id)? {
            users.push(user);
        }
    }
    Ok(users)
}

fn folders_of(conn: &Connection, user_id: i64) -> Result<Vec<String>> {
    Ok(conn
        .prepare("SELECT path FROM user_folders WHERE user_id = ?1 ORDER BY path")?
        .query_map([user_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

fn set_folders(conn: &Connection, user_id: i64, folders: &[String]) -> Result<()> {
    conn.execute("DELETE FROM user_folders WHERE user_id = ?1", [user_id])?;
    let mut insert = conn.prepare("INSERT INTO user_folders (user_id, path) VALUES (?1, ?2)")?;
    for folder in folders {
        insert.execute(params![user_id, folder])?;
    }
    Ok(())
}

/// Returns `(user id, password hash)` for an enabled user, looked up case-insensitively.
pub fn login_lookup(conn: &Connection, username: &str) -> Result<Option<(i64, String)>> {
    Ok(conn
        .query_row(
            "SELECT id, password_hash FROM users WHERE username = ?1 AND disabled = 0",
            [username],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

pub fn password_hash_of(conn: &Connection, user_id: i64) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT password_hash FROM users WHERE id = ?1",
            [user_id],
            |r| r.get(0),
        )
        .optional()?)
}

#[derive(Debug, Default, Deserialize)]
pub struct UserChanges {
    pub role: Option<Role>,
    pub password: Option<String>,
    pub disabled: Option<bool>,
    pub folders: Option<Vec<String>>,
}

#[derive(Debug, PartialEq)]
pub enum UpdateError {
    NotFound,
    /// The change would leave no enabled admin.
    LastAdmin,
}

/// Applies validated changes. Changing the password or disabling the user ends all of
/// their sessions.
pub fn update_user(
    conn: &mut Connection,
    id: i64,
    changes: &UserChanges,
) -> Result<Result<User, UpdateError>> {
    let password_hash = changes.password.as_deref().map(hash_password).transpose()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if get_user(&tx, id)?.is_none() {
        return Ok(Err(UpdateError::NotFound));
    }
    if let Some(role) = changes.role {
        tx.execute(
            "UPDATE users SET role = ?1 WHERE id = ?2",
            params![role.as_str(), id],
        )?;
    }
    if let Some(disabled) = changes.disabled {
        tx.execute(
            "UPDATE users SET disabled = ?1 WHERE id = ?2",
            params![disabled, id],
        )?;
    }
    if let Some(hash) = &password_hash {
        tx.execute(
            "UPDATE users SET password_hash = ?1 WHERE id = ?2",
            params![hash, id],
        )?;
    }
    if let Some(folders) = &changes.folders {
        set_folders(&tx, id, folders)?;
    }
    if enabled_admin_count(&tx)? == 0 {
        return Ok(Err(UpdateError::LastAdmin)); // Dropping `tx` rolls back.
    }
    if password_hash.is_some() || changes.disabled == Some(true) {
        tx.execute("DELETE FROM sessions WHERE user_id = ?1", [id])?;
    }
    tx.commit()?;
    Ok(get_user(conn, id)?.ok_or(UpdateError::NotFound))
}

pub fn delete_user(conn: &mut Connection, id: i64) -> Result<Result<(), UpdateError>> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if tx.execute("DELETE FROM users WHERE id = ?1", [id])? == 0 {
        return Ok(Err(UpdateError::NotFound));
    }
    if enabled_admin_count(&tx)? == 0 {
        return Ok(Err(UpdateError::LastAdmin));
    }
    tx.commit()?;
    Ok(Ok(()))
}

fn enabled_admin_count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM users WHERE role = 'admin' AND disabled = 0",
        [],
        |r| r.get(0),
    )?)
}

// ---- Sessions ----

/// A new random session token (hex), to be sent to the client as a cookie.
pub fn new_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("getrandom: {e}"))?;
    Ok(hex::encode(bytes))
}

fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

pub fn create_session(conn: &Connection, user_id: i64, token: &str) -> Result<()> {
    let now = now();
    conn.execute(
        "INSERT INTO sessions (token_hash, user_id, created_at, last_seen_at) VALUES (?1, ?2, ?3, ?3)",
        params![token_hash(token), user_id, now],
    )?;
    Ok(())
}

/// Returns the user for a live session token, refreshing its idle timer.
pub fn session_user(conn: &Connection, token: &str) -> Result<Option<User>> {
    let hash = token_hash(token);
    let now = now();
    let cutoff = now - SESSION_IDLE_LIFETIME.as_secs() as i64;
    let row: Option<(i64, i64)> = conn
        .query_row(
            "SELECT s.user_id, s.last_seen_at FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = ?1 AND s.last_seen_at > ?2 AND u.disabled = 0",
            params![hash, cutoff],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((user_id, last_seen)) = row else {
        return Ok(None);
    };
    if now - last_seen > SESSION_TOUCH_INTERVAL {
        conn.execute(
            "UPDATE sessions SET last_seen_at = ?1 WHERE token_hash = ?2",
            params![now, hash],
        )?;
    }
    get_user(conn, user_id)
}

pub fn delete_session(conn: &Connection, token: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM sessions WHERE token_hash = ?1",
        [token_hash(token)],
    )?;
    Ok(())
}

/// Ends all of a user's sessions except `keep` (the caller's own).
pub fn delete_other_sessions(conn: &Connection, user_id: i64, keep: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM sessions WHERE user_id = ?1 AND token_hash != ?2",
        params![user_id, token_hash(keep)],
    )?;
    Ok(())
}

pub fn set_password(conn: &Connection, user_id: i64, password: &str) -> Result<()> {
    conn.execute(
        "UPDATE users SET password_hash = ?1 WHERE id = ?2",
        params![hash_password(password)?, user_id],
    )
    .context("updating password")?;
    Ok(())
}

pub fn purge_expired_sessions(conn: &Connection) -> Result<usize> {
    let cutoff = now() - SESSION_IDLE_LIFETIME.as_secs() as i64;
    Ok(conn.execute("DELETE FROM sessions WHERE last_seen_at <= ?1", [cutoff])?)
}

// ---- Login throttling ----

/// Limits failed logins per username, to slow down password guessing.
#[derive(Default)]
pub struct LoginThrottle {
    failures: Mutex<HashMap<String, (u32, Instant)>>,
}

const MAX_FAILURES: u32 = 10;
const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);

impl LoginThrottle {
    pub fn is_locked(&self, username: &str) -> bool {
        let failures = self.failures.lock().unwrap();
        matches!(
            failures.get(&username.to_lowercase()),
            Some((count, since)) if *count >= MAX_FAILURES && since.elapsed() < FAILURE_WINDOW
        )
    }

    pub fn record_failure(&self, username: &str) {
        let mut failures = self.failures.lock().unwrap();
        failures.retain(|_, (_, since)| since.elapsed() < FAILURE_WINDOW);
        let entry = failures
            .entry(username.to_lowercase())
            .or_insert((0, Instant::now()));
        entry.0 += 1;
    }

    pub fn clear(&self, username: &str) {
        self.failures
            .lock()
            .unwrap()
            .remove(&username.to_lowercase());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("t.db")).unwrap();
        (dir, db)
    }

    fn add(conn: &mut Connection, name: &str, role: Role) -> User {
        create_user(
            conn,
            &NewUser {
                username: name,
                password: "password1",
                role,
                folders: &[],
            },
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    fn password_round_trip() {
        let hash = hash_password("correct horse").unwrap();
        assert!(verify_password("correct horse", &hash));
        assert!(!verify_password("wrong horse", &hash));
        assert!(!verify_password("x", "not a hash"));
    }

    #[test]
    fn first_admin_only_once() {
        let (_dir, db) = db();
        let mut conn = db.conn().unwrap();
        let admin = create_first_admin(&mut conn, "admin", "password1")
            .unwrap()
            .unwrap();
        assert_eq!(admin.role, Role::Admin);
        assert!(
            create_first_admin(&mut conn, "other", "password1")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn usernames_are_case_insensitive_and_unique() {
        let (_dir, db) = db();
        let mut conn = db.conn().unwrap();
        add(&mut conn, "Alice", Role::Viewer);
        let dup = create_user(
            &mut conn,
            &NewUser {
                username: "alice",
                password: "password1",
                role: Role::Viewer,
                folders: &[],
            },
        )
        .unwrap();
        assert!(dup.is_none());
        assert!(login_lookup(&conn, "ALICE").unwrap().is_some());
    }

    #[test]
    fn sessions_resolve_and_end() {
        let (_dir, db) = db();
        let mut conn = db.conn().unwrap();
        let user = add(&mut conn, "bob", Role::Editor);
        let token = new_token().unwrap();
        create_session(&conn, user.id, &token).unwrap();
        assert_eq!(session_user(&conn, &token).unwrap().unwrap().id, user.id);
        assert!(session_user(&conn, "bogus").unwrap().is_none());
        delete_session(&conn, &token).unwrap();
        assert!(session_user(&conn, &token).unwrap().is_none());
    }

    #[test]
    fn expired_sessions_are_rejected() {
        let (_dir, db) = db();
        let mut conn = db.conn().unwrap();
        let user = add(&mut conn, "bob", Role::Editor);
        let token = new_token().unwrap();
        create_session(&conn, user.id, &token).unwrap();
        conn.execute(
            "UPDATE sessions SET last_seen_at = ?1",
            [now() - SESSION_IDLE_LIFETIME.as_secs() as i64 - 1],
        )
        .unwrap();
        assert!(session_user(&conn, &token).unwrap().is_none());
        assert_eq!(purge_expired_sessions(&conn).unwrap(), 1);
    }

    #[test]
    fn disabling_or_new_password_ends_sessions() {
        let (_dir, db) = db();
        let mut conn = db.conn().unwrap();
        add(&mut conn, "admin", Role::Admin);
        let user = add(&mut conn, "bob", Role::Editor);
        for changes in [
            UserChanges {
                disabled: Some(true),
                ..Default::default()
            },
            UserChanges {
                disabled: Some(false),
                password: Some("new-password".into()),
                ..Default::default()
            },
        ] {
            let token = new_token().unwrap();
            create_session(&conn, user.id, &token).unwrap();
            update_user(&mut conn, user.id, &changes).unwrap().unwrap();
            assert!(session_user(&conn, &token).unwrap().is_none());
        }
        let (_, hash) = login_lookup(&conn, "bob").unwrap().unwrap();
        assert!(verify_password("new-password", &hash));
    }

    #[test]
    fn last_admin_is_protected() {
        let (_dir, db) = db();
        let mut conn = db.conn().unwrap();
        let admin = add(&mut conn, "admin", Role::Admin);
        let demote = UserChanges {
            role: Some(Role::Viewer),
            ..Default::default()
        };
        assert_eq!(
            update_user(&mut conn, admin.id, &demote).unwrap(),
            Err(UpdateError::LastAdmin)
        );
        assert_eq!(
            delete_user(&mut conn, admin.id).unwrap(),
            Err(UpdateError::LastAdmin)
        );
        // Rolled back: still an admin.
        assert_eq!(
            get_user(&conn, admin.id).unwrap().unwrap().role,
            Role::Admin
        );

        add(&mut conn, "second", Role::Admin);
        assert!(update_user(&mut conn, admin.id, &demote).unwrap().is_ok());
    }

    #[test]
    fn folder_restrictions() {
        let roots = vec![PathBuf::from("/media")];
        assert_eq!(
            clean_folders(&["/media/a/".into(), "/media/a".into()], &roots).unwrap(),
            ["/media/a"]
        );
        assert!(clean_folders(&["/etc".into()], &roots).is_err());
        assert!(clean_folders(&["/media/../etc".into()], &roots).is_err());
        assert!(clean_folders(&["media/a".into()], &roots).is_err());

        let (_dir, db) = db();
        let mut conn = db.conn().unwrap();
        let folders = vec!["/media/kids".to_string()];
        let user = create_user(
            &mut conn,
            &NewUser {
                username: "kid",
                password: "password1",
                role: Role::Viewer,
                folders: &folders,
            },
        )
        .unwrap()
        .unwrap();
        assert!(user.can_access(Path::new("/media/kids/2024/a.jpg")));
        assert!(!user.can_access(Path::new("/media/kidsstuff/a.jpg")));
        assert!(!user.can_access(Path::new("/media/private")));
    }

    #[test]
    fn throttle_locks_after_repeated_failures() {
        let throttle = LoginThrottle::default();
        for _ in 0..MAX_FAILURES {
            assert!(!throttle.is_locked("Bob"));
            throttle.record_failure("bob");
        }
        assert!(throttle.is_locked("BOB"));
        throttle.clear("bob");
        assert!(!throttle.is_locked("bob"));
    }

    #[test]
    fn validation() {
        assert_eq!(clean_username("  ann "), Ok("ann".into()));
        assert!(clean_username("   ").is_err());
        assert!(clean_username("a\nb").is_err());
        assert!(check_password("short").is_err());
        assert!(check_password("long enough").is_ok());
    }
}
