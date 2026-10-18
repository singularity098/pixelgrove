//! Moving media to the trash, and the Trash page (restore, delete forever, empty).

use std::path::{Path, PathBuf};

use axum::Json;
use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Serialize;

use super::browse::{Page, Paged};
use super::media::{FileRef, Versioned, file_response, preview_response, visible};
use super::{ApiError, ApiResult, CurrentUser};
use crate::auth::{Role, User};
use crate::server::AppState;
use crate::trash::{self, MoveError, RETENTION, TRASH_COLUMNS, TrashItem, Trashed};

fn move_error(err: MoveError) -> ApiError {
    match err {
        MoveError::ReadOnly => ApiError::status(
            StatusCode::CONFLICT,
            "This folder is read-only, so PixelGrove can't move files in it. Mount it read-write to use Trash.",
        ),
        MoveError::Missing => {
            ApiError::status(StatusCode::NOT_FOUND, "The file is no longer on disk.")
        }
    }
}

/// Moves an item to the trash.
pub async fn trash_media(
    State(state): State<AppState>,
    current: CurrentUser,
    UrlPath(id): UrlPath<i64>,
) -> ApiResult<StatusCode> {
    current.require(Role::Editor)?;
    let user_id = current.user.id;
    let m = visible(&state, current.user, id).await?;
    let original = m.path();
    let (root_id, root) = state
        .indexer
        .roots()
        .into_iter()
        .find(|(_, root)| original.starts_with(root))
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND, "Not found."))?;
    let moved_from = original.clone();
    let result = state
        .db
        .run(move |conn| {
            trash::move_to_trash(
                conn,
                &Trashed {
                    root_id,
                    root: &root,
                    original: &original,
                    name: &m.name,
                    kind: &m.kind,
                    size: m.size,
                    mtime: m.mtime,
                    taken_at: m.taken_at,
                    width: m.width,
                    height: m.height,
                    orientation: m.orientation,
                    duration_ms: m.duration_ms,
                    deleted_by: user_id,
                },
            )
        })
        .await?;
    result.map_err(move_error)?;
    // Drop it from the index now rather than waiting for the watcher.
    state.indexer.rescan(vec![moved_from]).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct TrashEntry {
    id: i64,
    name: String,
    kind: String,
    /// Where it will be restored to, relative to its media root.
    original: String,
    width: Option<u32>,
    height: Option<u32>,
    taken_at: i64,
    duration_ms: Option<i64>,
    size: i64,
    deleted_at: i64,
    /// When it will be deleted for good.
    expires_at: i64,
    v: i64,
}

impl TrashEntry {
    fn new(item: TrashItem, roots: &[(i64, PathBuf)]) -> Self {
        let original = roots
            .iter()
            .find_map(|(_, root)| Path::new(&item.original_path).strip_prefix(root).ok())
            .map_or_else(
                || item.original_path.clone(),
                |p| p.to_string_lossy().into_owned(),
            );
        let (width, height) = if item.orientation.is_some_and(|o| o >= 5) {
            (item.height, item.width)
        } else {
            (item.width, item.height)
        };
        Self {
            id: item.id,
            name: item.name,
            kind: item.kind,
            original,
            width,
            height,
            taken_at: item.taken_at,
            duration_ms: item.duration_ms,
            size: item.size,
            deleted_at: item.deleted_at,
            expires_at: item.deleted_at + RETENTION.as_secs() as i64,
            v: item.mtime,
        }
    }
}

fn can_see(user: &User, item: &TrashItem) -> bool {
    let folder = Path::new(&item.original_path)
        .parent()
        .unwrap_or(Path::new("/"));
    user.can_access(folder)
}

/// Every trashed item the user may see, most recently deleted first. (Trash holds few
/// items, so filtering in Rust is fine.)
async fn visible_items(state: &AppState, user: &User) -> ApiResult<Vec<TrashItem>> {
    let items: Vec<TrashItem> = state
        .db
        .run(|conn| {
            Ok(conn
                .prepare(&format!(
                    "SELECT {TRASH_COLUMNS} FROM trash ORDER BY deleted_at DESC, id DESC"
                ))?
                .query_map([], trash::item_from_row)?
                .collect::<rusqlite::Result<_>>()?)
        })
        .await?;
    Ok(items.into_iter().filter(|i| can_see(user, i)).collect())
}

async fn visible_item(state: &AppState, user: &User, id: i64) -> ApiResult<TrashItem> {
    state
        .db
        .run(move |conn| trash::get(conn, id))
        .await?
        .filter(|item| can_see(user, item))
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND, "Not in the trash."))
}

pub async fn list(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(page): Query<Page>,
) -> ApiResult<Json<Paged<TrashEntry>>> {
    current.require(Role::Editor)?;
    let (offset, limit) = page.clamped();
    let items = visible_items(&state, &current.user).await?;
    let total = items.len() as i64;
    let roots = state.indexer.roots();
    let items = items
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .map(|item| TrashEntry::new(item, &roots))
        .collect();
    Ok(Json(Paged { items, total }))
}

fn file_ref(item: &TrashItem) -> FileRef<'_> {
    FileRef {
        // Negative, so cached previews never collide with library media ids.
        cache_id: -item.id,
        path: Path::new(&item.trash_path),
        name: &item.name,
        kind: &item.kind,
        mtime: item.mtime,
        size: item.size,
        orientation: item.orientation,
        duration_ms: item.duration_ms,
        strip_location: false,
    }
}

pub async fn preview(
    State(state): State<AppState>,
    current: CurrentUser,
    UrlPath((id, variant)): UrlPath<(i64, String)>,
    Query(versioned): Query<Versioned>,
    request: Request,
) -> ApiResult<Response> {
    current.require(Role::Editor)?;
    let item = visible_item(&state, &current.user, id).await?;
    preview_response(&state, file_ref(&item), &variant, &versioned, request).await
}

pub async fn file(
    State(state): State<AppState>,
    current: CurrentUser,
    UrlPath(id): UrlPath<i64>,
    Query(versioned): Query<Versioned>,
    request: Request,
) -> ApiResult<Response> {
    current.require(Role::Editor)?;
    let item = visible_item(&state, &current.user, id).await?;
    file_response(&state, file_ref(&item), &versioned, request).await
}

pub async fn restore(
    State(state): State<AppState>,
    current: CurrentUser,
    UrlPath(id): UrlPath<i64>,
) -> ApiResult<StatusCode> {
    current.require(Role::Editor)?;
    let item = visible_item(&state, &current.user, id).await?;
    let restored = state
        .db
        .run(move |conn| trash::restore(conn, &item))
        .await?
        .map_err(move_error)?;
    state.indexer.rescan(vec![restored]).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_forever(
    State(state): State<AppState>,
    current: CurrentUser,
    UrlPath(id): UrlPath<i64>,
) -> ApiResult<StatusCode> {
    current.require(Role::Editor)?;
    let item = visible_item(&state, &current.user, id).await?;
    state
        .db
        .run(move |conn| trash::delete_forever(conn, &item))
        .await?
        .map_err(move_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes everything in the trash that this user can see.
pub async fn empty(State(state): State<AppState>, current: CurrentUser) -> ApiResult<StatusCode> {
    current.require(Role::Editor)?;
    let items = visible_items(&state, &current.user).await?;
    state
        .db
        .run(move |conn| {
            for item in &items {
                if let Err(err) = trash::delete_forever(conn, item)? {
                    tracing::warn!(path = %item.trash_path, ?err, "couldn't delete trashed file");
                }
            }
            Ok(())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use crate::api::test_support::TestApp;
    use crate::testdata;

    #[tokio::test]
    async fn trash_restore_and_delete() {
        let app = TestApp::new();
        app.write_media(
            "trip/a.jpg",
            &testdata::jpeg_taken_at("2024:01:01 10:00:00"),
        );
        app.write_media(
            "trip/b.jpg",
            &testdata::jpeg_taken_at("2024:01:02 10:00:00"),
        );
        app.scan();
        let admin = app.setup_admin().await;
        let editor = app.user_session(&admin, "ed", "editor").await;
        let viewer = app.user_session(&admin, "vi", "viewer").await;
        let a = app.media_id("a.jpg");

        let refused = app
            .send(
                "POST",
                &format!("/api/media/{a}/trash"),
                None,
                Some(&viewer),
            )
            .await;
        assert_eq!(refused.status, StatusCode::FORBIDDEN);

        let trashed = app
            .send(
                "POST",
                &format!("/api/media/{a}/trash"),
                None,
                Some(&editor),
            )
            .await;
        assert_eq!(trashed.status, StatusCode::NO_CONTENT, "{:?}", trashed.body);
        assert!(!app.media_root().join("trip/a.jpg").exists());
        assert!(
            app.media_root()
                .join(".pixelgrove-trash/trip/a.jpg")
                .exists()
        );
        // Gone from the library right away.
        let trip = app.folder_id("trip");
        let media = app
            .send(
                "GET",
                &format!("/api/folders/{trip}/media"),
                None,
                Some(&editor),
            )
            .await;
        assert_eq!(media.body["total"], 1);

        let list = app.send("GET", "/api/trash", None, Some(&editor)).await;
        assert_eq!(list.body["total"], 1);
        let entry = &list.body["items"][0];
        assert_eq!(entry["original"], "trip/a.jpg");
        let id = entry["id"].as_i64().unwrap();
        let preview = app
            .get_raw(&format!("/api/trash/{id}/preview/thumb"), &editor, None)
            .await;
        assert_eq!(preview.status(), StatusCode::OK);

        let restored = app
            .send(
                "POST",
                &format!("/api/trash/{id}/restore"),
                None,
                Some(&editor),
            )
            .await;
        assert_eq!(restored.status, StatusCode::NO_CONTENT);
        assert!(app.media_root().join("trip/a.jpg").exists());
        let media = app
            .send(
                "GET",
                &format!("/api/folders/{trip}/media"),
                None,
                Some(&editor),
            )
            .await;
        assert_eq!(media.body["total"], 2, "restored items are indexed again");

        // Trash again, then delete for good.
        let a = app.media_id("a.jpg");
        app.send(
            "POST",
            &format!("/api/media/{a}/trash"),
            None,
            Some(&editor),
        )
        .await;
        let list = app.send("GET", "/api/trash", None, Some(&editor)).await;
        let id = list.body["items"][0]["id"].as_i64().unwrap();
        let deleted = app
            .send("DELETE", &format!("/api/trash/{id}"), None, Some(&editor))
            .await;
        assert_eq!(deleted.status, StatusCode::NO_CONTENT);
        assert!(
            !app.media_root()
                .join(".pixelgrove-trash/trip/a.jpg")
                .exists()
        );
        assert_eq!(
            app.send("GET", "/api/trash", None, Some(&editor))
                .await
                .body["total"],
            0
        );
    }

    #[tokio::test]
    async fn empty_trash() {
        let app = TestApp::new();
        app.write_media("x/1.jpg", &testdata::jpeg_with_exif());
        app.write_media("x/2.jpg", &testdata::jpeg_with_exif());
        app.scan();
        let admin = app.setup_admin().await;
        for name in ["1.jpg", "2.jpg"] {
            let id = app.media_id(name);
            app.send(
                "POST",
                &format!("/api/media/{id}/trash"),
                None,
                Some(&admin),
            )
            .await;
        }
        assert_eq!(
            app.send("GET", "/api/trash", None, Some(&admin)).await.body["total"],
            2
        );
        let emptied = app.send("DELETE", "/api/trash", None, Some(&admin)).await;
        assert_eq!(emptied.status, StatusCode::NO_CONTENT);
        assert_eq!(
            app.send("GET", "/api/trash", None, Some(&admin)).await.body["total"],
            0
        );
    }
}
