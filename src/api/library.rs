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
    _: CurrentUser,
) -> ApiResult<Json<LibraryStatus>> {
    let (folders, images, videos) = state
        .db
        .run(|conn| {
            Ok(conn.query_row(
                "SELECT (SELECT count(*) FROM folders),
                        (SELECT count(*) FROM media WHERE kind = 'image'),
                        (SELECT count(*) FROM media WHERE kind = 'video')",
                [],
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
    }
}
