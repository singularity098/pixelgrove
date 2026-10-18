//! Library-wide search: folders whose name contains the query, and media whose file
//! name, folder path or metadata contains every word of it.

use axum::Json;
use axum::extract::{Query, State};
use rusqlite::{Connection, params_from_iter};
use serde::{Deserialize, Serialize};

use super::browse::{
    Cover, MEDIA_ITEM_COLUMNS, MediaItem, Paged, cover, not_hidden, relative, scope, summarize,
};
use super::{ApiResult, CurrentUser};
use crate::auth::User;
use crate::server::AppState;

#[derive(Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    offset: i64,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    120
}

#[derive(Serialize)]
pub struct FolderHit {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub count: i64,
    pub cover: Option<Cover>,
}

#[derive(Serialize)]
pub struct SearchResults {
    /// Only on the first page.
    folders: Vec<FolderHit>,
    media: Paged<MediaItem>,
}

/// Splits a query into lower-cased words, each turned into a `LIKE` pattern.
fn like_patterns(q: &str) -> Vec<String> {
    q.split_whitespace()
        .take(8)
        .map(|word| {
            let escaped = word
                .to_lowercase()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            format!("%{escaped}%")
        })
        .collect()
}

pub async fn search(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<SearchQuery>,
) -> ApiResult<Json<SearchResults>> {
    let user = current.user;
    let offset = query.offset.max(0);
    let limit = query.limit.clamp(1, 500);
    let patterns = like_patterns(&query.q);
    if patterns.is_empty() {
        return Ok(Json(SearchResults {
            folders: Vec::new(),
            media: Paged {
                items: Vec::new(),
                total: 0,
            },
        }));
    }
    let results = state
        .db
        .run(move |conn| {
            let folders = if offset == 0 {
                matching_folders(conn, &user, &patterns, 50)?
            } else {
                Vec::new()
            };
            let media = matching_media(conn, &user, &patterns, offset, limit)?;
            Ok(SearchResults { folders, media })
        })
        .await?;
    Ok(Json(results))
}

/// Folders whose name contains every word, newest first.
pub fn matching_folders(
    conn: &Connection,
    user: &User,
    patterns: &[String],
    limit: i64,
) -> anyhow::Result<Vec<FolderHit>> {
    let (where_scope, mut params) = scope(user, "f.path");
    let mut conditions = vec![where_scope];
    for pattern in patterns {
        conditions.push("lower(f.name) LIKE ? ESCAPE '\\'".into());
        params.push(pattern.clone());
    }
    params.push(limit.to_string());
    let sql = format!(
        "SELECT f.id, f.name, f.path, r.path, f.media_count, m.id, m.kind, m.mtime
         FROM folders f
         JOIN roots r ON r.id = f.root_id
         LEFT JOIN media m ON m.id = f.cover_media_id
         WHERE f.path != r.path AND {}
         ORDER BY f.latest_taken_at DESC NULLS LAST, f.name
         LIMIT CAST(? AS INTEGER)",
        conditions.join(" AND ")
    );
    let mut hits: Vec<FolderHit> = conn
        .prepare(&sql)?
        .query_map(params_from_iter(&params), |r| {
            Ok(FolderHit {
                id: r.get(0)?,
                name: r.get(1)?,
                path: relative(&r.get::<_, String>(2)?, &r.get::<_, String>(3)?),
                count: r.get(4)?,
                cover: cover(r, 5)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    // The stored count and cover may include hidden photos.
    if user.hidden.media_rule.is_some() {
        for hit in &mut hits {
            let (count, _, cover) = summarize(
                conn,
                "f.id = CAST(? AS INTEGER)",
                &[hit.id.to_string()],
                &user.hidden,
            )?;
            hit.count = count;
            hit.cover = cover;
        }
    }
    Ok(hits)
}

fn matching_media(
    conn: &Connection,
    user: &User,
    patterns: &[String],
    offset: i64,
    limit: i64,
) -> anyhow::Result<Paged<MediaItem>> {
    let (where_scope, mut params) = scope(user, "f.path");
    // Folders are only needed to apply folder limits and hidden folders.
    let join = if user.folders.is_empty() && user.hidden.folders.is_empty() {
        ""
    } else {
        "JOIN folders f ON f.id = m.folder_id"
    };
    let mut conditions = vec![where_scope, not_hidden(&user.hidden, "m.id")];
    for pattern in patterns {
        conditions.push("m.search_text LIKE ? ESCAPE '\\'".into());
        params.push(pattern.clone());
    }
    let from = format!("FROM media m {join} WHERE {}", conditions.join(" AND "));
    let total: i64 = conn.query_row(
        &format!("SELECT count(*) {from}"),
        params_from_iter(&params),
        |r| r.get(0),
    )?;
    params.push(limit.to_string());
    params.push(offset.to_string());
    let mut items: Vec<MediaItem> = conn
        .prepare(&format!(
            "SELECT {MEDIA_ITEM_COLUMNS} {from}
             ORDER BY m.taken_at DESC, m.id DESC
             LIMIT CAST(? AS INTEGER) OFFSET CAST(? AS INTEGER)"
        ))?
        .query_map(params_from_iter(&params), MediaItem::from_row)?
        .collect::<rusqlite::Result<_>>()?;
    MediaItem::mark_hidden(&mut items, &user.hidden);
    Ok(Paged { items, total })
}

/// Folder filter for File View: folders (with or without media) whose name contains
/// every word.
pub async fn filter_folders(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<SearchQuery>,
) -> ApiResult<Json<Vec<FolderHit>>> {
    let user = current.user;
    let patterns = like_patterns(&query.q);
    if patterns.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let limit = query.limit.clamp(1, 500);
    let hits = state
        .db
        .run(move |conn| matching_folders(conn, &user, &patterns, limit))
        .await?;
    Ok(Json(hits))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::{Value, json};

    use super::like_patterns;
    use crate::api::test_support::TestApp;
    use crate::testdata;

    fn names(items: &Value) -> Vec<String> {
        let mut names: Vec<String> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["name"].as_str().unwrap().to_string())
            .collect();
        names.sort();
        names
    }

    fn library(app: &TestApp) {
        app.write_media(
            "Japan 2022/tokyo.jpg",
            &testdata::jpeg_taken_at("2022:04:01 10:00:00"),
        );
        app.write_media(
            "Japan 2022/kyoto.jpg",
            &testdata::jpeg_taken_at("2022:04:05 10:00:00"),
        );
        app.write_media(
            "Family/beach.jpg",
            &testdata::jpeg_taken_at("2023:07:01 10:00:00"),
        );
        app.write_media(
            "Family/100%_fun.jpg",
            &testdata::jpeg_taken_at("2023:07:02 10:00:00"),
        );
        std::fs::create_dir_all(app.media_root().join("Family/Empty Japan Notes")).unwrap();
        app.scan();
    }

    async fn search(app: &TestApp, cookie: &str, q: &str) -> Value {
        let uri = format!("/api/search?q={}", urlencode(q));
        let response = app.send("GET", &uri, None, Some(cookie)).await;
        assert_eq!(response.status, StatusCode::OK);
        response.body
    }

    fn urlencode(s: &str) -> String {
        s.bytes()
            .map(|b| match b {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' => (b as char).to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect()
    }

    #[tokio::test]
    async fn finds_by_name_folder_metadata_and_date() {
        let app = TestApp::new();
        library(&app);
        let admin = app.setup_admin().await;

        let japan = search(&app, &admin, "JAPAN").await;
        assert_eq!(
            names(&japan["folders"]),
            ["Empty Japan Notes", "Japan 2022"]
        );
        assert_eq!(names(&japan["media"]["items"]), ["kyoto.jpg", "tokyo.jpg"]);

        assert_eq!(
            names(&search(&app, &admin, "kyo").await["media"]["items"]),
            ["kyoto.jpg", "tokyo.jpg"]
        );
        assert_eq!(
            names(&search(&app, &admin, "kyoto japan").await["media"]["items"]),
            ["kyoto.jpg"]
        );
        assert_eq!(search(&app, &admin, "pixelcam").await["media"]["total"], 4);
        assert_eq!(
            names(&search(&app, &admin, "2023-07-02").await["media"]["items"]),
            ["100%_fun.jpg"]
        );
        // % and _ are literal, not wildcards.
        assert_eq!(
            names(&search(&app, &admin, "0%_").await["media"]["items"]),
            ["100%_fun.jpg"]
        );
        assert_eq!(search(&app, &admin, "   ").await["media"]["total"], 0);
    }

    #[tokio::test]
    async fn search_respects_folder_limits() {
        let app = TestApp::new();
        library(&app);
        let admin = app.setup_admin().await;
        let family = format!("{}/Family", app.media_root().display());
        app.send(
            "POST",
            "/api/users",
            Some(json!({"username": "g", "password": "guest-password", "role": "viewer", "folders": [family]})),
            Some(&admin),
        )
        .await;
        let guest = app.login("g", "guest-password").await.cookie.unwrap();
        let japan = search(&app, &guest, "japan").await;
        assert_eq!(names(&japan["folders"]), ["Empty Japan Notes"]);
        assert_eq!(japan["media"]["total"], 0);
    }

    #[tokio::test]
    async fn folder_filter() {
        let app = TestApp::new();
        library(&app);
        let admin = app.setup_admin().await;
        let response = app
            .send("GET", "/api/folders/filter?q=japan", None, Some(&admin))
            .await;
        assert_eq!(names(&response.body), ["Empty Japan Notes", "Japan 2022"]);
        assert_eq!(
            response.body[0]["path"].as_str().unwrap().contains('/'),
            response.body[0]["name"] == "Empty Japan Notes"
        );
    }

    #[test]
    fn patterns_escape_wildcards() {
        assert_eq!(like_patterns(" A_b  50% "), ["%a\\_b%", "%50\\%%"]);
    }
}
