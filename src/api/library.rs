//! Library-wide information.

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use super::{ApiResult, CurrentUser};
use crate::indexer::IndexStatus;
use crate::server::AppState;

#[derive(Serialize)]
pub struct LibraryStatus {
    folders: i64,
    images: i64,
    videos: i64,
    #[serde(flatten)]
    index: IndexStatus,
}

pub async fn status(
    State(state): State<AppState>,
    current: CurrentUser,
) -> ApiResult<Json<LibraryStatus>> {
    let user = current.user;
    let (folders, images, videos) = state
        .db
        .run(move |conn| {
            // Counts cover only the folders this user may see.
            let (scope, params) = super::browse::scope(&user, "f.path");
            Ok(conn.query_row(
                &format!(
                    "SELECT (SELECT count(*) FROM folders f WHERE {scope}),
                            (SELECT coalesce(sum(m.kind = 'image'), 0) FROM media m
                             JOIN folders f ON f.id = m.folder_id WHERE {scope}),
                            (SELECT coalesce(sum(m.kind = 'video'), 0) FROM media m
                             JOIN folders f ON f.id = m.folder_id WHERE {scope})"
                ),
                rusqlite::params_from_iter(params.iter().chain(&params).chain(&params)),
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .await?;
    Ok(Json(LibraryStatus {
        folders,
        images,
        videos,
        index: state.indexer.status(),
    }))
}

#[derive(Serialize)]
pub struct Volume {
    /// The media roots on this volume.
    roots: Vec<String>,
    total: u64,
    free: u64,
    used: u64,
}

/// Disk usage of the volumes holding the media roots (one entry per filesystem).
pub async fn storage(
    State(state): State<AppState>,
    _: CurrentUser,
) -> ApiResult<Json<Vec<Volume>>> {
    let roots: Vec<std::path::PathBuf> =
        state.indexer.roots().into_iter().map(|(_, r)| r).collect();
    let volumes = tokio::task::spawn_blocking(move || {
        let mut volumes: Vec<(u64, Volume)> = Vec::new();
        for root in roots {
            let (Ok(stats), Ok(meta)) = (rustix::fs::statvfs(&root), std::fs::metadata(&root))
            else {
                continue;
            };
            let device = std::os::unix::fs::MetadataExt::dev(&meta);
            let name = root.to_string_lossy().into_owned();
            if let Some((_, volume)) = volumes.iter_mut().find(|(d, _)| *d == device) {
                volume.roots.push(name);
                continue;
            }
            let block = stats.f_frsize;
            let total = stats.f_blocks * block;
            let free = stats.f_bavail * block;
            volumes.push((
                device,
                Volume {
                    roots: vec![name],
                    total,
                    free,
                    used: total.saturating_sub(stats.f_bfree * block),
                },
            ));
        }
        volumes.into_iter().map(|(_, v)| v).collect::<Vec<_>>()
    })
    .await?;
    Ok(Json(volumes))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use crate::api::test_support::TestApp;

    #[tokio::test]
    async fn status_requires_sign_in() {
        let app = TestApp::new();
        assert_eq!(
            app.send("GET", "/api/library/status", None, None)
                .await
                .status,
            StatusCode::UNAUTHORIZED
        );
        let admin = app.setup_admin().await;
        let response = app
            .send("GET", "/api/library/status", None, Some(&admin))
            .await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body["images"], 0);
        assert_eq!(response.body["scanning"], false);

        let storage = app.send("GET", "/api/storage", None, Some(&admin)).await;
        assert_eq!(storage.status, StatusCode::OK);
        let volume = &storage.body[0];
        assert!(volume["total"].as_u64().unwrap() > 0);
        assert!(volume["used"].as_u64().unwrap() <= volume["total"].as_u64().unwrap());
    }
}
