//! Admin settings: the media folders in the library.

use std::path::PathBuf;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use super::{AdminUser, ApiError, ApiResult};
use crate::indexer::RootError;
use crate::server::AppState;

#[derive(Serialize)]
pub struct MediaRoot {
    path: String,
    /// Set in the config file or environment, so it can't be removed here.
    configured: bool,
    folders: i64,
    media: i64,
    /// False when the directory is missing (e.g. a volume isn't mounted).
    available: bool,
    /// False for read-only mounts, where Trash is unavailable.
    writable: bool,
}

pub async fn roots(State(state): State<AppState>, _: AdminUser) -> ApiResult<Json<Vec<MediaRoot>>> {
    Ok(Json(list(&state).await?))
}

async fn list(state: &AppState) -> ApiResult<Vec<MediaRoot>> {
    let roots = state.indexer.roots();
    let ids: Vec<i64> = roots.iter().map(|(id, _)| *id).collect();
    let counts = state
        .db
        .run(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT (SELECT count(*) FROM folders WHERE root_id = ?1),
                        (SELECT count(*) FROM media m JOIN folders f ON f.id = m.folder_id
                         WHERE f.root_id = ?1)",
            )?;
            ids.iter()
                .map(|id| Ok(stmt.query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))?))
                .collect::<anyhow::Result<Vec<(i64, i64)>>>()
        })
        .await?;
    let indexer = state.indexer.clone();
    Ok(tokio::task::spawn_blocking(move || {
        roots
            .into_iter()
            .zip(counts)
            .map(|((_, path), (folders, media))| MediaRoot {
                configured: indexer.is_configured(&path),
                available: path.is_dir(),
                writable: rustix::fs::access(&path, rustix::fs::Access::WRITE_OK).is_ok(),
                path: path.to_string_lossy().into_owned(),
                folders,
                media,
            })
            .collect()
    })
    .await?)
}

#[derive(Deserialize)]
pub struct RootBody {
    path: String,
}

fn root_error(err: RootError) -> ApiError {
    match err {
        RootError::Invalid(message) => ApiError::bad_request(message),
        RootError::Internal(err) => ApiError::Internal(err),
    }
}

pub async fn add_root(
    State(state): State<AppState>,
    _: AdminUser,
    Json(body): Json<RootBody>,
) -> ApiResult<(StatusCode, Json<Vec<MediaRoot>>)> {
    state
        .indexer
        .add_root(&body.path)
        .await
        .map_err(root_error)?;
    Ok((StatusCode::CREATED, Json(list(&state).await?)))
}

pub async fn remove_root(
    State(state): State<AppState>,
    _: AdminUser,
    Json(body): Json<RootBody>,
) -> ApiResult<Json<Vec<MediaRoot>>> {
    state
        .indexer
        .remove_root(&body.path)
        .await
        .map_err(root_error)?;
    Ok(Json(list(&state).await?))
}

#[derive(Deserialize)]
pub struct DirectoriesQuery {
    #[serde(default)]
    path: String,
}

#[derive(Serialize)]
pub struct Directories {
    path: String,
    parent: Option<String>,
    /// Subdirectory names, sorted; hidden ones are left out.
    dirs: Vec<String>,
}

/// Lists directories inside the container, to pick a media folder from.
pub async fn directories(
    _: AdminUser,
    Query(query): Query<DirectoriesQuery>,
) -> ApiResult<Json<Directories>> {
    let path = PathBuf::from(if query.path.trim().is_empty() {
        "/"
    } else {
        query.path.trim()
    });
    if !path.is_absolute() {
        return Err(ApiError::bad_request("Enter a full path, such as /photos."));
    }
    let listing = tokio::task::spawn_blocking(move || {
        let entries = std::fs::read_dir(&path).ok()?;
        let mut dirs: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|name| !name.starts_with('.'))
            .collect();
        dirs.sort_by_key(|name| name.to_lowercase());
        dirs.truncate(1000);
        Some(Directories {
            parent: path.parent().map(|p| p.to_string_lossy().into_owned()),
            path: path.to_string_lossy().into_owned(),
            dirs,
        })
    })
    .await?;
    listing
        .map(Json)
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND, "That folder can't be opened."))
}

/// Starts a full rescan now, instead of waiting for the next scheduled one.
pub async fn rescan(State(state): State<AppState>, _: AdminUser) -> StatusCode {
    if !state.indexer.status().scanning {
        let indexer = state.indexer.clone();
        tokio::spawn(async move { indexer.full_scan().await });
    }
    StatusCode::ACCEPTED
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::http::StatusCode;
    use serde_json::json;

    use crate::api::test_support::TestApp;
    use crate::testdata;

    #[tokio::test(flavor = "multi_thread")]
    async fn add_and_remove_media_folders() {
        let app = TestApp::new();
        let admin = app.setup_admin().await;
        let editor = app.user_session(&admin, "ed", "editor").await;
        assert_eq!(
            app.send("GET", "/api/settings/roots", None, Some(&editor))
                .await
                .status,
            StatusCode::FORBIDDEN
        );

        let roots = app
            .send("GET", "/api/settings/roots", None, Some(&admin))
            .await;
        assert_eq!(roots.body.as_array().unwrap().len(), 1);
        assert_eq!(roots.body[0]["configured"], true);
        assert_eq!(roots.body[0]["writable"], true);

        let extra = tempfile::tempdir().unwrap();
        std::fs::create_dir(extra.path().join("album")).unwrap();
        std::fs::write(extra.path().join("album/a.jpg"), testdata::jpeg_with_exif()).unwrap();
        let inside = app.media_root().join("sub");
        std::fs::create_dir(&inside).unwrap();
        for bad in [
            "relative/path".to_string(),
            "/".to_string(),
            "/definitely/not/here".to_string(),
            inside.to_string_lossy().into_owned(),
            app.media_root().to_string_lossy().into_owned(),
        ] {
            let response = app
                .send(
                    "POST",
                    "/api/settings/roots",
                    Some(json!({ "path": bad })),
                    Some(&admin),
                )
                .await;
            assert_eq!(response.status, StatusCode::BAD_REQUEST, "{bad}");
            assert!(response.body["error"].is_string());
        }

        let path = format!("{}/", extra.path().display());
        let added = app
            .send(
                "POST",
                "/api/settings/roots",
                Some(json!({ "path": path })),
                Some(&admin),
            )
            .await;
        assert_eq!(added.status, StatusCode::CREATED);
        assert_eq!(
            added.body[1]["path"],
            extra.path().to_string_lossy().as_ref()
        );
        assert_eq!(added.body[1]["configured"], false);
        assert_eq!(
            crate::indexer::load_extra_roots(app.db()).unwrap(),
            [extra.path()]
        );

        // It's indexed in the background.
        let mut images = 0;
        for _ in 0..100 {
            let status = app
                .send("GET", "/api/library/status", None, Some(&admin))
                .await;
            images = status.body["images"].as_i64().unwrap();
            if images == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(images, 1);

        // Configured folders can only be removed in the configuration.
        let configured = app
            .send(
                "DELETE",
                "/api/settings/roots",
                Some(json!({ "path": app.media_root() })),
                Some(&admin),
            )
            .await;
        assert_eq!(configured.status, StatusCode::BAD_REQUEST);

        let removed = app
            .send(
                "DELETE",
                "/api/settings/roots",
                Some(json!({ "path": extra.path() })),
                Some(&admin),
            )
            .await;
        assert_eq!(removed.status, StatusCode::OK);
        assert_eq!(removed.body.as_array().unwrap().len(), 1);
        let status = app
            .send("GET", "/api/library/status", None, Some(&admin))
            .await;
        assert_eq!(status.body["images"], 0);
        assert!(
            extra.path().join("album/a.jpg").exists(),
            "files are untouched"
        );
    }

    #[tokio::test]
    async fn directory_browser() {
        let app = TestApp::new();
        let admin = app.setup_admin().await;
        std::fs::create_dir(app.media_root().join("Photos")).unwrap();
        std::fs::create_dir(app.media_root().join(".hidden")).unwrap();
        std::fs::write(app.media_root().join("file.txt"), b"x").unwrap();
        let uri = format!(
            "/api/settings/directories?path={}",
            app.media_root().display()
        );
        let listing = app.send("GET", &uri, None, Some(&admin)).await;
        assert_eq!(listing.status, StatusCode::OK);
        assert_eq!(listing.body["dirs"], json!(["Photos"]));
        assert!(listing.body["parent"].is_string());

        let missing = app
            .send(
                "GET",
                "/api/settings/directories?path=/no/such/dir",
                None,
                Some(&admin),
            )
            .await;
        assert_eq!(missing.status, StatusCode::NOT_FOUND);
    }
}
