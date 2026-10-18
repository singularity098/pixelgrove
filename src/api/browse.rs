//! Browsing folders ("albums") and their media. Every query is limited to the folders
//! the signed-in user may see.

use std::path::Path;

use axum::Json;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::StatusCode;
use rusqlite::{Connection, OptionalExtension, Row, params_from_iter};
use serde::{Deserialize, Serialize};

use super::{ApiError, ApiResult, CurrentUser};
use crate::auth::{Hidden, User};
use crate::server::AppState;

/// A SQL condition limiting `column` (a folder path) to what `user` may see, with its
/// parameters: inside the folders they're limited to (each with everything below it),
/// and not in a hidden folder.
pub fn scope(user: &User, column: &str) -> (String, Vec<String>) {
    let (hidden, mut params) = not_in_hidden_folders(&user.hidden, column);
    if user.folders.is_empty() {
        return (hidden, params);
    }
    let mut clauses = Vec::new();
    let mut allowed = Vec::new();
    for folder in &user.folders {
        let (cond, folder_params) = subtree(column, folder);
        clauses.push(cond);
        allowed.extend(folder_params);
    }
    allowed.append(&mut params);
    (format!("({}) AND {hidden}", clauses.join(" OR ")), allowed)
}

/// A condition on `column` (a folder path) leaving out hidden folders and everything
/// below them.
pub fn not_in_hidden_folders(hidden: &Hidden, column: &str) -> (String, Vec<String>) {
    if hidden.folders.is_empty() {
        return ("1".into(), Vec::new());
    }
    let mut clauses = Vec::new();
    let mut params = Vec::new();
    for folder in &hidden.folders {
        let (cond, folder_params) = subtree(column, folder);
        clauses.push(cond);
        params.extend(folder_params);
    }
    (format!("NOT ({})", clauses.join(" OR ")), params)
}

/// A condition on `column` (a media id) leaving out hidden photos and videos. No
/// parameters, so it can go anywhere in a query.
pub fn not_hidden(hidden: &Hidden, column: &str) -> String {
    match &hidden.media_rule {
        None => "1".into(),
        Some(rule) => format!(
            "{column} NOT IN (SELECT media_id FROM hidden WHERE media_id IS NOT NULL AND {rule})"
        ),
    }
}

/// Count, newest capture time and cover of the visible media in the folders matching
/// `cond` (on `f`, with `params`). Uses the folders' stored totals unless hidden photos
/// apply, which then have to be left out one by one.
pub(super) fn summarize(
    conn: &Connection,
    cond: &str,
    params: &[String],
    hidden: &Hidden,
) -> rusqlite::Result<(i64, Option<i64>, Option<Cover>)> {
    if hidden.media_rule.is_none() {
        let (count, latest): (i64, Option<i64>) = conn.query_row(
            &format!(
                "SELECT coalesce(sum(f.media_count), 0), max(f.latest_taken_at) FROM folders f WHERE {cond}"
            ),
            params_from_iter(params),
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let cover = if count == 0 {
            None
        } else {
            conn.query_row(
                &format!(
                    "SELECT m.id, m.kind, m.mtime FROM folders f
                     JOIN media m ON m.id = f.cover_media_id
                     WHERE f.media_count > 0 AND {cond}
                     ORDER BY f.latest_taken_at DESC LIMIT 1"
                ),
                params_from_iter(params),
                |r| cover(r, 0),
            )
            .optional()?
            .flatten()
        };
        return Ok((count, latest, cover));
    }
    let visible = not_hidden(hidden, "m.id");
    let from =
        format!("FROM media m JOIN folders f ON f.id = m.folder_id WHERE {cond} AND {visible}");
    let count: i64 = conn.query_row(
        &format!("SELECT count(*) {from}"),
        params_from_iter(params),
        |r| r.get(0),
    )?;
    let newest = conn
        .query_row(
            &format!("SELECT m.taken_at, m.id, m.kind, m.mtime {from} ORDER BY m.taken_at DESC, m.id DESC LIMIT 1"),
            params_from_iter(params),
            |r| Ok((r.get::<_, i64>(0)?, cover(r, 1)?)),
        )
        .optional()?;
    Ok(match newest {
        Some((latest, cover)) => (count, Some(latest), cover),
        None => (0, None, None),
    })
}

/// The same subtree condition, for one folder.
pub(super) fn subtree(column: &str, path: &str) -> (String, Vec<String>) {
    (
        format!("({column} = ? OR ({column} >= ? AND {column} < ?))"),
        vec![path.to_string(), format!("{path}/"), format!("{path}0")],
    )
}

#[derive(Serialize)]
pub struct Cover {
    id: i64,
    kind: String,
    v: i64,
}

#[derive(Serialize)]
pub struct Album {
    id: i64,
    name: String,
    /// Path relative to its media root, for display.
    path: String,
    count: i64,
    latest_taken_at: Option<i64>,
    cover: Option<Cover>,
    /// Shown only because "show hidden" is on.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    hidden: bool,
}

#[derive(Deserialize)]
pub struct Page {
    #[serde(default)]
    pub offset: i64,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    100
}

impl Page {
    pub fn clamped(&self) -> (i64, i64) {
        (self.offset.max(0), self.limit.clamp(1, 500))
    }
}

#[derive(Serialize)]
pub struct Paged<T> {
    pub items: Vec<T>,
    pub total: i64,
}

/// The default Photos view: every folder that directly contains media, newest first.
pub async fn albums(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(page): Query<Page>,
) -> ApiResult<Json<Paged<Album>>> {
    let user = current.user;
    let (offset, limit) = page.clamped();
    let result = state
        .db
        .run(move |conn| {
            let (where_scope, mut params) = scope(&user, "f.path");
            // Each folder's count, newest time and cover: the stored totals, or worked
            // out without the hidden photos when some apply.
            let folders = if user.hidden.media_rule.is_none() {
                "SELECT f.id, f.name, f.path, f.root_id, f.media_count AS n,
                        f.latest_taken_at AS latest, f.cover_media_id AS cover_id
                 FROM folders f"
                    .to_string()
            } else {
                let visible = not_hidden(&user.hidden, "x.id");
                format!(
                    "SELECT f.id, f.name, f.path, f.root_id,
                            (SELECT count(*) FROM media x WHERE x.folder_id = f.id AND {visible}) AS n,
                            (SELECT max(x.taken_at) FROM media x WHERE x.folder_id = f.id AND {visible}) AS latest,
                            (SELECT x.id FROM media x WHERE x.folder_id = f.id AND {visible}
                             ORDER BY x.taken_at DESC, x.id DESC LIMIT 1) AS cover_id
                     FROM folders f"
                )
            };
            let with = format!(
                "WITH v AS ({folders} WHERE f.media_count > 0 AND {where_scope})"
            );
            let total: i64 = conn.query_row(
                &format!("{with} SELECT count(*) FROM v WHERE v.n > 0"),
                params_from_iter(&params),
                |r| r.get(0),
            )?;
            params.push(limit.to_string());
            params.push(offset.to_string());
            let mut stmt = conn.prepare(&format!(
                "{with}
                 SELECT v.id, v.name, v.path, r.path, v.n, v.latest, m.id, m.kind, m.mtime
                 FROM v
                 JOIN roots r ON r.id = v.root_id
                 LEFT JOIN media m ON m.id = v.cover_id
                 WHERE v.n > 0
                 ORDER BY v.latest DESC, v.id DESC
                 LIMIT CAST(? AS INTEGER) OFFSET CAST(? AS INTEGER)"
            ))?;
            let items = stmt
                .query_map(params_from_iter(&params), |r| {
                    let path: String = r.get(2)?;
                    Ok(Album {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        hidden: user.hidden.marks_folder(&path),
                        path: relative(&path, &r.get::<_, String>(3)?),
                        count: r.get(4)?,
                        latest_taken_at: r.get(5)?,
                        cover: cover(r, 6)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
            Ok(Paged { items, total })
        })
        .await?;
    Ok(Json(result))
}

pub(super) fn cover(row: &Row, first: usize) -> rusqlite::Result<Option<Cover>> {
    Ok(match row.get::<_, Option<i64>>(first)? {
        Some(id) => Some(Cover {
            id,
            kind: row.get(first + 1)?,
            v: row.get(first + 2)?,
        }),
        None => None,
    })
}

/// `path` relative to `root`, with the root itself shown by its directory name.
pub(super) fn relative(path: &str, root: &str) -> String {
    match Path::new(path).strip_prefix(root) {
        Ok(rel) if rel.as_os_str().is_empty() => root_name(root),
        Ok(rel) => rel.to_string_lossy().into_owned(),
        Err(_) => path.to_string(),
    }
}

fn root_name(root: &str) -> String {
    Path::new(root)
        .file_name()
        .map_or_else(|| root.to_string(), |n| n.to_string_lossy().into_owned())
}

#[derive(Serialize)]
pub struct Crumb {
    id: i64,
    name: String,
}

#[derive(Serialize)]
pub struct Subfolder {
    pub id: i64,
    name: String,
    /// Media in this folder and everything below it.
    count: i64,
    latest_taken_at: Option<i64>,
    cover: Option<Cover>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    hidden: bool,
}

#[derive(Serialize)]
pub struct FolderDetail {
    id: i64,
    name: String,
    path: String,
    media_count: i64,
    /// Ancestors the user may see, outermost first.
    breadcrumbs: Vec<Crumb>,
    /// Shown only because "show hidden" is on: "this" folder is hidden, or a "parent".
    #[serde(skip_serializing_if = "Option::is_none")]
    hidden: Option<&'static str>,
    /// Child folders with media somewhere below them, newest first.
    subfolders: Vec<Subfolder>,
}

pub(super) struct FolderRow {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub path: String,
    pub name: String,
    pub root: String,
    pub media_count: i64,
}

pub(super) fn load_folder(conn: &Connection, id: i64) -> rusqlite::Result<Option<FolderRow>> {
    conn.query_row(
        "SELECT f.id, f.parent_id, f.path, f.name, r.path, f.media_count
         FROM folders f JOIN roots r ON r.id = f.root_id WHERE f.id = ?1",
        [id],
        |r| {
            Ok(FolderRow {
                id: r.get(0)?,
                parent_id: r.get(1)?,
                path: r.get(2)?,
                name: r.get(3)?,
                root: r.get(4)?,
                media_count: r.get(5)?,
            })
        },
    )
    .optional()
}

/// Loads a folder the user may see, or a 404 (also for folders they may not see, so
/// their existence isn't revealed).
pub(super) fn visible_folder(conn: &Connection, user: &User, id: i64) -> ApiResult<FolderRow> {
    match load_folder(conn, id)? {
        Some(folder)
            if user.can_access(Path::new(&folder.path))
                && !user.hidden.hides_folder(&folder.path) =>
        {
            Ok(folder)
        }
        _ => Err(ApiError::status(StatusCode::NOT_FOUND, "Folder not found.")),
    }
}

pub async fn folder(
    State(state): State<AppState>,
    current: CurrentUser,
    UrlPath(id): UrlPath<i64>,
) -> ApiResult<Json<FolderDetail>> {
    let user = current.user;
    let detail = state
        .db
        .run(move |conn| Ok(folder_detail(conn, &user, id)))
        .await??;
    Ok(Json(detail))
}

fn folder_detail(conn: &Connection, user: &User, id: i64) -> ApiResult<FolderDetail> {
    let folder = visible_folder(conn, user, id)?;

    let mut breadcrumbs = Vec::new();
    let mut parent = folder.parent_id;
    while let Some(parent_id) = parent {
        let Some(ancestor) = load_folder(conn, parent_id)? else {
            break;
        };
        if !user.can_access(Path::new(&ancestor.path)) {
            break;
        }
        breadcrumbs.push(Crumb {
            id: ancestor.id,
            name: display_name(&ancestor),
        });
        parent = ancestor.parent_id;
    }
    breadcrumbs.reverse();

    let subfolders = subfolders_of(conn, folder.id, &user.hidden)?;
    let media_count = if user.hidden.media_rule.is_none() {
        folder.media_count
    } else {
        conn.query_row(
            &format!(
                "SELECT count(*) FROM media m WHERE m.folder_id = ?1 AND {}",
                not_hidden(&user.hidden, "m.id")
            ),
            [folder.id],
            |r| r.get(0),
        )?
    };

    Ok(FolderDetail {
        id: folder.id,
        name: display_name(&folder),
        path: relative(&folder.path, &folder.root),
        media_count,
        breadcrumbs,
        subfolders,
        hidden: if user.hidden.marked_folders.contains(&folder.path) {
            Some("this")
        } else if user.hidden.marks_folder(&folder.path) {
            Some("parent")
        } else {
            None
        },
    })
}

/// Child folders that have visible media somewhere below them, with nested counts and
/// the newest cover, newest first. Hidden folders and photos are left out.
pub(super) fn subfolders_of(
    conn: &Connection,
    folder_id: i64,
    hidden: &Hidden,
) -> anyhow::Result<Vec<Subfolder>> {
    let children: Vec<(i64, String, String)> = conn
        .prepare("SELECT id, name, path FROM folders WHERE parent_id = ?1")?
        .query_map([folder_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut subfolders = Vec::new();
    for (child_id, name, path) in children {
        if hidden.hides_folder(&path) {
            continue;
        }
        let (below, mut params) = subtree("f.path", &path);
        let (visible, visible_params) = not_in_hidden_folders(hidden, "f.path");
        params.extend(visible_params);
        let (count, latest, cover) =
            summarize(conn, &format!("{below} AND {visible}"), &params, hidden)?;
        if count == 0 {
            continue;
        }
        subfolders.push(Subfolder {
            id: child_id,
            hidden: hidden.marks_folder(&path),
            name,
            count,
            latest_taken_at: latest,
            cover,
        });
    }
    subfolders.sort_by_key(|s| std::cmp::Reverse(s.latest_taken_at));
    Ok(subfolders)
}

pub(super) fn display_name(folder: &FolderRow) -> String {
    if folder.path == folder.root {
        root_name(&folder.root)
    } else {
        folder.name.clone()
    }
}

#[derive(Serialize)]
pub struct MediaItem {
    id: i64,
    pub name: String,
    kind: String,
    /// Display dimensions (already swapped for rotated photos).
    width: Option<u32>,
    height: Option<u32>,
    taken_at: i64,
    duration_ms: Option<i64>,
    size: i64,
    /// Changes whenever the file does; used to version preview URLs.
    v: i64,
    /// Shown only because "show hidden" is on.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    hidden: bool,
}

impl MediaItem {
    /// Flags items that are shown only because "show hidden" is on.
    pub(super) fn mark_hidden(items: &mut [MediaItem], hidden: &Hidden) {
        if hidden.marked_media.is_empty() {
            return;
        }
        for item in items {
            item.hidden = hidden.marked_media.contains(&item.id);
        }
    }

    pub(super) fn from_row(r: &Row) -> rusqlite::Result<Self> {
        let (width, height): (Option<u32>, Option<u32>) = (r.get(3)?, r.get(4)?);
        let orientation: Option<u16> = r.get(5)?;
        let (width, height) = if orientation.is_some_and(|o| o >= 5) {
            (height, width)
        } else {
            (width, height)
        };
        Ok(Self {
            id: r.get(0)?,
            name: r.get(1)?,
            kind: r.get(2)?,
            width,
            height,
            taken_at: r.get(6)?,
            duration_ms: r.get(7)?,
            v: r.get(8)?,
            size: r.get(9)?,
            hidden: false,
        })
    }
}

/// Columns read by [`MediaItem::from_row`], for a `media` table aliased `m`.
pub(super) const MEDIA_ITEM_COLUMNS: &str = "m.id, m.name, m.kind, m.width, m.height, m.orientation, m.taken_at, m.duration_ms, m.mtime, m.size";

#[derive(Deserialize)]
pub struct FolderMediaQuery {
    #[serde(default)]
    offset: i64,
    #[serde(default = "default_limit")]
    limit: i64,
    /// Also include everything in subfolders (for slideshows).
    #[serde(default)]
    recursive: bool,
}

pub async fn folder_media(
    State(state): State<AppState>,
    current: CurrentUser,
    UrlPath(id): UrlPath<i64>,
    Query(query): Query<FolderMediaQuery>,
) -> ApiResult<Json<Paged<MediaItem>>> {
    let user = current.user;
    let (offset, limit) = Page {
        offset: query.offset,
        limit: query.limit,
    }
    .clamped();
    let result = state
        .db
        .run(move |conn| {
            Ok(visible_folder(conn, &user, id).and_then(|folder| {
                let recursive = query.recursive.then(|| scope(&user, "f.path"));
                let mut page = media_page(conn, &folder, recursive, &user.hidden, offset, limit)?;
                MediaItem::mark_hidden(&mut page.items, &user.hidden);
                Ok(page)
            }))
        })
        .await??;
    Ok(Json(result))
}

/// A page of a folder's media, newest first, without hidden photos. With `recursive`
/// (a condition on `f.path` for the subfolders that may be included, which should
/// leave out hidden folders), media below it is included too.
pub(super) fn media_page(
    conn: &Connection,
    folder: &FolderRow,
    recursive: Option<(String, Vec<String>)>,
    hidden: &Hidden,
    offset: i64,
    limit: i64,
) -> rusqlite::Result<Paged<MediaItem>> {
    let visible = not_hidden(hidden, "m.id");
    let Some((where_scope, scope_params)) = recursive else {
        let items = conn
            .prepare(&format!(
                "SELECT {MEDIA_ITEM_COLUMNS} FROM media m WHERE m.folder_id = ?1 AND {visible}
                 ORDER BY m.taken_at DESC, m.id DESC LIMIT ?2 OFFSET ?3"
            ))?
            .query_map([folder.id, limit, offset], MediaItem::from_row)?
            .collect::<rusqlite::Result<_>>()?;
        let total = if hidden.media_rule.is_none() {
            folder.media_count
        } else {
            conn.query_row(
                &format!("SELECT count(*) FROM media m WHERE m.folder_id = ?1 AND {visible}"),
                [folder.id],
                |r| r.get(0),
            )?
        };
        return Ok(Paged { items, total });
    };
    let (below, mut params) = subtree("f.path", &folder.path);
    params.extend(scope_params);
    let from = format!(
        "FROM media m JOIN folders f ON f.id = m.folder_id WHERE {below} AND {where_scope} AND {visible}"
    );
    let total = conn.query_row(
        &format!("SELECT count(*) {from}"),
        params_from_iter(&params),
        |r| r.get(0),
    )?;
    params.push(limit.to_string());
    params.push(offset.to_string());
    let items = conn
        .prepare(&format!(
            "SELECT {MEDIA_ITEM_COLUMNS} {from}
             ORDER BY m.taken_at DESC, m.id DESC
             LIMIT CAST(? AS INTEGER) OFFSET CAST(? AS INTEGER)"
        ))?
        .query_map(params_from_iter(&params), MediaItem::from_row)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Paged { items, total })
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::{Value, json};

    use crate::api::test_support::TestApp;
    use crate::testdata;

    /// root/
    ///   2023/trip/a.jpg (2023-06-15), b.jpg (2023-06-16)
    ///   2023/notes/      (no media)
    ///   2024/c.jpg       (2024-01-01)
    ///   2024/deep/x/d.jpg (2022-01-01)
    fn library(app: &TestApp) {
        for (path, date) in [
            ("2023/trip/a.jpg", "2023:06:15 10:00:00"),
            ("2023/trip/b.jpg", "2023:06:16 10:00:00"),
            ("2024/c.jpg", "2024:01:01 10:00:00"),
            ("2024/deep/x/d.jpg", "2022:01:01 10:00:00"),
        ] {
            app.write_media(path, &testdata::jpeg_taken_at(date));
        }
        std::fs::create_dir_all(app.media_root().join("2023/notes")).unwrap();
        app.scan();
    }

    fn names(items: &Value) -> Vec<String> {
        items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["name"].as_str().unwrap().to_string())
            .collect()
    }

    #[tokio::test]
    async fn albums_newest_first() {
        let app = TestApp::new();
        library(&app);
        let admin = app.setup_admin().await;
        let albums = app.send("GET", "/api/albums", None, Some(&admin)).await;
        assert_eq!(albums.status, StatusCode::OK);
        assert_eq!(names(&albums.body["items"]), ["2024", "trip", "x"]);
        assert_eq!(albums.body["total"], 3);
        assert_eq!(albums.body["items"][1]["path"], "2023/trip");
        assert_eq!(albums.body["items"][1]["count"], 2);
        assert!(albums.body["items"][1]["cover"]["id"].is_i64());

        let page = app
            .send("GET", "/api/albums?offset=1&limit=1", None, Some(&admin))
            .await;
        assert_eq!(names(&page.body["items"]), ["trip"]);
    }

    #[tokio::test]
    async fn folder_detail_and_media() {
        let app = TestApp::new();
        library(&app);
        let admin = app.setup_admin().await;
        let root_id = app.folder_id("");
        let root = app
            .send(
                "GET",
                &format!("/api/folders/{root_id}"),
                None,
                Some(&admin),
            )
            .await;
        // Subfolders with media below them, newest first; "notes" has none.
        assert_eq!(names(&root.body["subfolders"]), ["2024", "2023"]);
        assert_eq!(
            root.body["subfolders"][0]["count"], 2,
            "counts include nested media"
        );

        let trip_id = app.folder_id("2023/trip");
        let trip = app
            .send(
                "GET",
                &format!("/api/folders/{trip_id}"),
                None,
                Some(&admin),
            )
            .await;
        assert_eq!(names(&trip.body["breadcrumbs"]).len(), 2);
        assert_eq!(trip.body["breadcrumbs"][1]["name"], "2023");
        let media = app
            .send(
                "GET",
                &format!("/api/folders/{trip_id}/media"),
                None,
                Some(&admin),
            )
            .await;
        assert_eq!(names(&media.body["items"]), ["b.jpg", "a.jpg"]);
        assert_eq!(media.body["items"][0]["width"], 4);
    }

    #[tokio::test]
    async fn recursive_folder_media() {
        let app = TestApp::new();
        library(&app);
        let admin = app.setup_admin().await;
        let root_id = app.folder_id("");
        let uri = format!("/api/folders/{root_id}/media?recursive=true");
        let all = app.send("GET", &uri, None, Some(&admin)).await;
        assert_eq!(
            names(&all.body["items"]),
            ["c.jpg", "b.jpg", "a.jpg", "d.jpg"]
        );
        assert_eq!(all.body["total"], 4);
        let page = app
            .send(
                "GET",
                &format!("{uri}&offset=1&limit=2"),
                None,
                Some(&admin),
            )
            .await;
        assert_eq!(names(&page.body["items"]), ["b.jpg", "a.jpg"]);

        let y2024 = app.folder_id("2024");
        let nested = app
            .send(
                "GET",
                &format!("/api/folders/{y2024}/media?recursive=true"),
                None,
                Some(&admin),
            )
            .await;
        assert_eq!(names(&nested.body["items"]), ["c.jpg", "d.jpg"]);
    }

    #[tokio::test]
    async fn folder_limits_are_enforced() {
        let app = TestApp::new();
        library(&app);
        let admin = app.setup_admin().await;
        let limited = format!("{}/2023", app.media_root().display());
        let response = app
            .send(
                "POST",
                "/api/users",
                Some(json!({
                    "username": "kid", "password": "kid-password", "role": "viewer",
                    "folders": [limited]
                })),
                Some(&admin),
            )
            .await;
        assert_eq!(response.status, StatusCode::CREATED);
        let kid = app.login("kid", "kid-password").await.cookie.unwrap();

        let albums = app.send("GET", "/api/albums", None, Some(&kid)).await;
        assert_eq!(names(&albums.body["items"]), ["trip"]);

        let other = app.folder_id("2024");
        for uri in [
            format!("/api/folders/{other}"),
            format!("/api/folders/{other}/media"),
        ] {
            let response = app.send("GET", &uri, None, Some(&kid)).await;
            assert_eq!(response.status, StatusCode::NOT_FOUND, "{uri}");
        }

        // Breadcrumbs stop at the user's top folder.
        let trip = app.folder_id("2023/trip");
        let detail = app
            .send("GET", &format!("/api/folders/{trip}"), None, Some(&kid))
            .await;
        assert_eq!(names(&detail.body["breadcrumbs"]), ["2023"]);
    }
}
