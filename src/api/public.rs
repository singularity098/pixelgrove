//! Share links, opened without signing in: `/api/s/{token}/...`.

use std::path::{Component, Path};

use axum::Json;
use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::browse::{MEDIA_ITEM_COLUMNS, MediaItem, Paged, Subfolder, load_folder, subfolders_of};
use super::media::{FileRef, MediaRow, Versioned, file_response, load, preview_response};
use super::{ApiError, ApiResult, is_https};
use crate::server::AppState;
use crate::shares::{self, Share};

fn gone() -> ApiError {
    ApiError::status(
        StatusCode::NOT_FOUND,
        "This link has expired or been removed.",
    )
}

fn locked() -> ApiError {
    ApiError::status(StatusCode::FORBIDDEN, "This link needs a password.")
}

/// Loads a live share and whether this visitor has unlocked it.
async fn resolve(state: &AppState, token: &str, jar: &CookieJar) -> ApiResult<(Share, bool)> {
    let token = token.to_string();
    let (share, secret) = state
        .db
        .run(move |conn| Ok((shares::by_token(conn, &token)?, shares::secret(conn)?)))
        .await?;
    let share = share
        .filter(|s| !s.is_expired(crate::auth::now()))
        .ok_or_else(gone)?;
    let unlocked = share.password_hash.is_none()
        || jar
            .get(&shares::cookie_name(&share))
            .is_some_and(|c| c.value() == shares::unlock_value(&secret, &share));
    Ok((share, unlocked))
}

async fn unlocked(state: &AppState, token: &str, jar: &CookieJar) -> ApiResult<Share> {
    match resolve(state, token, jar).await? {
        (share, true) => Ok(share),
        (_, false) => Err(locked()),
    }
}

#[derive(Serialize)]
pub struct ShareInfo {
    kind: &'static str,
    locked: bool,
    /// Hidden until unlocked.
    name: Option<String>,
    allow_download: bool,
    expires_at: Option<i64>,
    /// For single-item links.
    media: Option<MediaItem>,
}

fn info(conn: &Connection, share: &Share, unlocked: bool) -> anyhow::Result<ShareInfo> {
    let kind = if share.media_id.is_some() {
        "media"
    } else {
        "folder"
    };
    if !unlocked {
        return Ok(ShareInfo {
            kind,
            locked: true,
            name: None,
            allow_download: share.allow_download,
            expires_at: share.expires_at,
            media: None,
        });
    }
    let (name, media) = match share.media_id {
        Some(id) => {
            let item = conn.query_row(
                &format!("SELECT {MEDIA_ITEM_COLUMNS} FROM media m WHERE m.id = ?1"),
                [id],
                MediaItem::from_row,
            )?;
            (item.name.clone(), Some(item))
        }
        None => (
            Path::new(share.folder_path.as_deref().unwrap_or(""))
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            None,
        ),
    };
    Ok(ShareInfo {
        kind,
        locked: false,
        name: Some(name),
        allow_download: share.allow_download,
        expires_at: share.expires_at,
        media,
    })
}

pub async fn open(
    State(state): State<AppState>,
    UrlPath(token): UrlPath<String>,
    jar: CookieJar,
) -> ApiResult<Json<ShareInfo>> {
    let (share, unlocked) = resolve(&state, &token, &jar).await?;
    let info = state
        .db
        .run(move |conn| {
            if unlocked {
                shares::record_view(conn, share.id)?;
            }
            info(conn, &share, unlocked)
        })
        .await?;
    Ok(Json(info))
}

#[derive(Deserialize)]
pub struct Unlock {
    password: String,
}

pub async fn unlock(
    State(state): State<AppState>,
    UrlPath(token): UrlPath<String>,
    headers: axum::http::HeaderMap,
    jar: CookieJar,
    Json(body): Json<Unlock>,
) -> ApiResult<(CookieJar, Json<ShareInfo>)> {
    let throttle_key = format!("share:{token}");
    if state.login_throttle.is_locked(&throttle_key) {
        return Err(ApiError::status(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many attempts. Try again in a few minutes.",
        ));
    }
    let (share, _) = resolve(&state, &token, &jar).await?;
    let hash = share.password_hash.clone().unwrap_or_default();
    let password = body.password;
    let correct = tokio::task::spawn_blocking(move || {
        !hash.is_empty() && crate::auth::verify_password(&password, &hash)
    })
    .await?;
    if !correct {
        state.login_throttle.record_failure(&throttle_key);
        return Err(ApiError::status(
            StatusCode::FORBIDDEN,
            "Incorrect password.",
        ));
    }
    state.login_throttle.clear(&throttle_key);
    let secure = is_https(&state, &headers);
    let (jar, info) = state
        .db
        .run(move |conn| {
            let cookie = Cookie::build((
                shares::cookie_name(&share),
                shares::unlock_value(&shares::secret(conn)?, &share),
            ))
            .path("/")
            .http_only(true)
            .same_site(SameSite::Lax)
            .secure(secure)
            .max_age(cookie::time::Duration::days(30))
            .build();
            shares::record_view(conn, share.id)?;
            Ok((jar.add(cookie), info(conn, &share, true)?))
        })
        .await?;
    Ok((jar, Json(info)))
}

#[derive(Deserialize)]
pub struct FolderQuery {
    /// Path below the shared folder ("" for the shared folder itself).
    #[serde(default)]
    path: String,
    #[serde(default)]
    offset: i64,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    200
}

#[derive(Serialize)]
pub struct SharedFolder {
    name: String,
    path: String,
    /// Only on the first page.
    subfolders: Vec<Subfolder>,
    media: Paged<MediaItem>,
}

pub async fn folder(
    State(state): State<AppState>,
    UrlPath(token): UrlPath<String>,
    Query(query): Query<FolderQuery>,
    jar: CookieJar,
) -> ApiResult<Json<SharedFolder>> {
    let share = unlocked(&state, &token, &jar).await?;
    let Some(root) = share.folder_path.clone() else {
        return Err(ApiError::status(
            StatusCode::NOT_FOUND,
            "Not a folder link.",
        ));
    };
    // Only plain names below the shared folder; no "..", no absolute paths.
    let relative = Path::new(query.path.trim_matches('/'));
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(ApiError::status(StatusCode::NOT_FOUND, "Not found."));
    }
    let full = Path::new(&root).join(relative);
    let full = full.to_string_lossy().trim_end_matches('/').to_string();
    let offset = query.offset.max(0);
    let limit = query.limit.clamp(1, 500);
    let display_path = relative.to_string_lossy().into_owned();
    let result = state
        .db
        .run(move |conn| {
            let folder_id: Option<i64> = rusqlite::OptionalExtension::optional(conn.query_row(
                "SELECT id FROM folders WHERE path = ?1",
                [&full],
                |r| r.get(0),
            ))?;
            let Some(folder) = folder_id
                .map(|id| load_folder(conn, id))
                .transpose()?
                .flatten()
            else {
                return Ok(None);
            };
            let subfolders = if offset == 0 {
                subfolders_of(conn, folder.id)?
            } else {
                Vec::new()
            };
            let items = conn
                .prepare(&format!(
                    "SELECT {MEDIA_ITEM_COLUMNS} FROM media m WHERE m.folder_id = ?1
                     ORDER BY m.taken_at DESC, m.id DESC LIMIT ?2 OFFSET ?3"
                ))?
                .query_map([folder.id, limit, offset], MediaItem::from_row)?
                .collect::<rusqlite::Result<_>>()?;
            Ok(Some(SharedFolder {
                name: folder.name,
                path: display_path,
                subfolders,
                media: Paged {
                    items,
                    total: folder.media_count,
                },
            }))
        })
        .await?
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND, "Not found."))?;
    Ok(Json(result))
}

/// Media in the share: the shared item itself, or anything below the shared folder.
async fn shared_media(state: &AppState, share: &Share, id: i64) -> ApiResult<MediaRow> {
    let media = state
        .db
        .run(move |conn| load(conn, id).map_err(Into::into))
        .await?
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND, "Not found."))?;
    let included = share.media_id == Some(id) || share.covers_folder(&media.folder_path);
    if included {
        Ok(media)
    } else {
        Err(ApiError::status(StatusCode::NOT_FOUND, "Not found."))
    }
}

/// Details for the viewer's info panel. Location and server paths are left out: the
/// visitor isn't a library user.
#[derive(Serialize)]
pub struct SharedDetail {
    id: i64,
    name: String,
    kind: String,
    size: i64,
    taken_at: i64,
    modified_at: i64,
    width: Option<u32>,
    height: Option<u32>,
    duration_ms: Option<i64>,
    camera_make: Option<String>,
    camera_model: Option<String>,
    lens: Option<String>,
    description: Option<String>,
    v: i64,
}

pub async fn media_detail(
    State(state): State<AppState>,
    UrlPath((token, id)): UrlPath<(String, i64)>,
    jar: CookieJar,
) -> ApiResult<Json<SharedDetail>> {
    let share = unlocked(&state, &token, &jar).await?;
    let m = shared_media(&state, &share, id).await?;
    let (width, height) = if m.orientation.is_some_and(|o| o >= 5) {
        (m.height, m.width)
    } else {
        (m.width, m.height)
    };
    Ok(Json(SharedDetail {
        id: m.id,
        name: m.name,
        kind: m.kind,
        size: m.size,
        taken_at: m.taken_at,
        modified_at: m.mtime,
        width,
        height,
        duration_ms: m.duration_ms,
        camera_make: m.camera_make,
        camera_model: m.camera_model,
        lens: m.lens,
        description: m.description,
        v: m.mtime,
    }))
}

fn file_ref<'a>(m: &'a MediaRow, path: &'a Path) -> FileRef<'a> {
    FileRef {
        cache_id: m.id,
        path,
        name: &m.name,
        kind: &m.kind,
        mtime: m.mtime,
        size: m.size,
        orientation: m.orientation,
    }
}

pub async fn preview(
    State(state): State<AppState>,
    UrlPath((token, id, variant)): UrlPath<(String, i64, String)>,
    Query(versioned): Query<Versioned>,
    jar: CookieJar,
    request: Request,
) -> ApiResult<Response> {
    let share = unlocked(&state, &token, &jar).await?;
    let m = shared_media(&state, &share, id).await?;
    let path = m.path();
    preview_response(&state, file_ref(&m, &path), &variant, &versioned, request).await
}

/// The original. With downloads off, only videos stream (they have no preview); photos
/// are shown through previews alone.
pub async fn file(
    State(state): State<AppState>,
    UrlPath((token, id)): UrlPath<(String, i64)>,
    Query(versioned): Query<Versioned>,
    jar: CookieJar,
    request: Request,
) -> ApiResult<Response> {
    let share = unlocked(&state, &token, &jar).await?;
    let m = shared_media(&state, &share, id).await?;
    if !share.allow_download && (versioned.download || m.kind != "video") {
        return Err(ApiError::status(
            StatusCode::FORBIDDEN,
            "Downloads are turned off for this link.",
        ));
    }
    let path = m.path();
    file_response(file_ref(&m, &path), &versioned, request).await
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::{Value, json};

    use crate::api::test_support::TestApp;
    use crate::testdata;

    fn library(app: &TestApp) {
        app.write_media(
            "trip/a.jpg",
            &testdata::jpeg_taken_at("2024:01:01 10:00:00"),
        );
        app.write_media(
            "trip/day2/b.jpg",
            &testdata::jpeg_taken_at("2024:01:02 10:00:00"),
        );
        app.write_media(
            "private/c.jpg",
            &testdata::jpeg_taken_at("2024:01:03 10:00:00"),
        );
        app.scan();
    }

    async fn create(app: &TestApp, cookie: &str, body: Value) -> Value {
        let response = app
            .send("POST", "/api/shares", Some(body), Some(cookie))
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
        response.body
    }

    #[tokio::test]
    async fn folder_link_without_login() {
        let app = TestApp::new();
        library(&app);
        let admin = app.setup_admin().await;
        let trip = app.folder_id("trip");
        let share = create(&app, &admin, json!({"folder_id": trip})).await;
        assert_eq!(share["name"], "trip");
        assert_eq!(
            share["url"],
            format!("/s/{}", share["token"].as_str().unwrap())
        );
        let token = share["token"].as_str().unwrap();

        let info = app
            .send("GET", &format!("/api/s/{token}"), None, None)
            .await;
        assert_eq!(info.status, StatusCode::OK);
        assert_eq!(info.body["kind"], "folder");
        assert_eq!(info.body["locked"], false);

        let root = app
            .send("GET", &format!("/api/s/{token}/folder"), None, None)
            .await;
        assert_eq!(root.body["media"]["items"][0]["name"], "a.jpg");
        assert_eq!(root.body["subfolders"][0]["name"], "day2");
        let day2 = app
            .send(
                "GET",
                &format!("/api/s/{token}/folder?path=day2"),
                None,
                None,
            )
            .await;
        assert_eq!(day2.body["media"]["items"][0]["name"], "b.jpg");

        // Nothing outside the shared folder.
        let escape = app
            .send(
                "GET",
                &format!("/api/s/{token}/folder?path=../private"),
                None,
                None,
            )
            .await;
        assert_eq!(escape.status, StatusCode::NOT_FOUND);
        let private = app.media_id("c.jpg");
        for uri in [
            format!("/api/s/{token}/media/{private}"),
            format!("/api/s/{token}/media/{private}/file"),
            format!("/api/s/{token}/media/{private}/preview/thumb"),
        ] {
            assert_eq!(
                app.send("GET", &uri, None, None).await.status,
                StatusCode::NOT_FOUND,
                "{uri}"
            );
        }
        let b = app.media_id("b.jpg");
        let detail = app
            .send("GET", &format!("/api/s/{token}/media/{b}"), None, None)
            .await;
        assert_eq!(detail.body["name"], "b.jpg");
        assert!(
            detail.body.get("path").is_none(),
            "server paths aren't exposed"
        );

        // Views are counted when the link is opened.
        let list = app.send("GET", "/api/shares", None, Some(&admin)).await;
        assert_eq!(list.body[0]["views"], 1);

        // Revoked links stop working.
        let id = share["id"].as_i64().unwrap();
        app.send("DELETE", &format!("/api/shares/{id}"), None, Some(&admin))
            .await;
        assert_eq!(
            app.send("GET", &format!("/api/s/{token}"), None, None)
                .await
                .status,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn password_protected_link() {
        let app = TestApp::new();
        library(&app);
        let admin = app.setup_admin().await;
        let a = app.media_id("a.jpg");
        let share = create(
            &app,
            &admin,
            json!({"media_id": a, "password": "open sesame"}),
        )
        .await;
        assert_eq!(share["has_password"], true);
        let token = share["token"].as_str().unwrap();

        let info = app
            .send("GET", &format!("/api/s/{token}"), None, None)
            .await;
        assert_eq!(info.body["locked"], true);
        assert!(
            info.body["name"].is_null(),
            "nothing leaks before unlocking"
        );
        assert_eq!(
            app.send(
                "GET",
                &format!("/api/s/{token}/media/{a}/preview/thumb"),
                None,
                None
            )
            .await
            .status,
            StatusCode::FORBIDDEN
        );

        let wrong = app
            .send(
                "POST",
                &format!("/api/s/{token}/unlock"),
                Some(json!({"password": "nope"})),
                None,
            )
            .await;
        assert_eq!(wrong.status, StatusCode::FORBIDDEN);
        let right = app
            .send(
                "POST",
                &format!("/api/s/{token}/unlock"),
                Some(json!({"password": "open sesame"})),
                None,
            )
            .await;
        assert_eq!(right.status, StatusCode::OK);
        assert_eq!(right.body["media"]["name"], "a.jpg");
        let cookie = right.cookie.unwrap();
        let preview = app
            .get_raw(
                &format!("/api/s/{token}/media/{a}/preview/thumb"),
                &cookie,
                None,
            )
            .await;
        assert_eq!(preview.status(), StatusCode::OK);

        // Changing the password locks out the old cookie.
        let id = share["id"].as_i64().unwrap();
        app.send(
            "PATCH",
            &format!("/api/shares/{id}"),
            Some(json!({"password": "new secret"})),
            Some(&admin),
        )
        .await;
        let info = app
            .send("GET", &format!("/api/s/{token}"), None, Some(&cookie))
            .await;
        assert_eq!(info.body["locked"], true);
    }

    #[tokio::test]
    async fn downloads_can_be_turned_off_and_links_expire() {
        let app = TestApp::new();
        library(&app);
        let admin = app.setup_admin().await;
        let a = app.media_id("a.jpg");
        let share = create(
            &app,
            &admin,
            json!({"media_id": a, "allow_download": false, "expires_in_days": 7}),
        )
        .await;
        let token = share["token"].as_str().unwrap();
        assert!(share["expires_at"].is_i64());
        assert_eq!(
            app.send("GET", &format!("/api/s/{token}/media/{a}/file"), None, None)
                .await
                .status,
            StatusCode::FORBIDDEN
        );

        // Expire it.
        let id = share["id"].as_i64().unwrap();
        let bad = app
            .send(
                "PATCH",
                &format!("/api/shares/{id}"),
                Some(json!({"expires_in_days": 0})),
                Some(&admin),
            )
            .await;
        assert_eq!(bad.status, StatusCode::BAD_REQUEST);
        app.send(
            "PATCH",
            &format!("/api/shares/{id}"),
            Some(json!({"expires_in_days": null, "allow_download": true})),
            Some(&admin),
        )
        .await;
        let list = app.send("GET", "/api/shares", None, Some(&admin)).await;
        assert!(list.body[0]["expires_at"].is_null());
        assert_eq!(list.body[0]["allow_download"], true);
    }

    #[tokio::test]
    async fn sharing_needs_editor_and_access() {
        let app = TestApp::new();
        library(&app);
        let admin = app.setup_admin().await;
        let viewer = app.user_session(&admin, "vi", "viewer").await;
        let trip = app.folder_id("trip");
        let refused = app
            .send(
                "POST",
                "/api/shares",
                Some(json!({"folder_id": trip})),
                Some(&viewer),
            )
            .await;
        assert_eq!(refused.status, StatusCode::FORBIDDEN);

        let editor = app.user_session(&admin, "ed", "editor").await;
        create(&app, &editor, json!({"folder_id": trip})).await;
        create(&app, &admin, json!({"folder_id": trip})).await;
        let editors = app.send("GET", "/api/shares", None, Some(&editor)).await;
        assert_eq!(
            editors.body.as_array().unwrap().len(),
            1,
            "editors see their own links"
        );
        let admins = app.send("GET", "/api/shares", None, Some(&admin)).await;
        assert_eq!(
            admins.body.as_array().unwrap().len(),
            2,
            "admins see all links"
        );

        let both = app
            .send(
                "POST",
                "/api/shares",
                Some(json!({"folder_id": trip, "media_id": 1})),
                Some(&admin),
            )
            .await;
        assert_eq!(both.status, StatusCode::BAD_REQUEST);
    }
}
