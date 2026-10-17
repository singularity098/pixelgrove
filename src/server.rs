//! HTTP routing.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

use crate::assets;
use crate::config::Config;
use crate::db::Db;
use crate::indexer::{IndexStatus, Indexer};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct AppState {
    #[allow(dead_code)] // Read by upcoming API handlers.
    pub config: Arc<Config>,
    pub db: Db,
    pub indexer: Indexer,
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/library/status", get(library_status))
        .fallback(api_not_found);

    Router::new()
        .nest("/api", api)
        .fallback(assets::serve)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: VERSION,
    })
}

#[derive(Serialize)]
struct LibraryStatus {
    folders: i64,
    images: i64,
    videos: i64,
    #[serde(flatten)]
    index: IndexStatus,
}

async fn library_status(State(state): State<AppState>) -> Result<Json<LibraryStatus>, ApiError> {
    let db = state.db.clone();
    let (folders, images, videos) = tokio::task::spawn_blocking(move || {
        let conn = db.conn()?;
        let counts = conn.query_row(
            "SELECT (SELECT count(*) FROM folders),
                    (SELECT count(*) FROM media WHERE kind = 'image'),
                    (SELECT count(*) FROM media WHERE kind = 'video')",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        anyhow::Ok(counts)
    })
    .await??;
    Ok(Json(LibraryStatus {
        folders,
        images,
        videos,
        index: state.indexer.status(),
    }))
}

/// An internal error, logged and reported to the client as a generic 500.
pub struct ApiError(anyhow::Error);

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("request failed: {:#}", self.0);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "internal error" })),
        )
            .into_response()
    }
}

async fn api_not_found() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "not found" })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    struct TestApp {
        _dir: tempfile::TempDir,
        router: Router,
    }

    fn app() -> TestApp {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::with_config_dir(dir.path().to_path_buf());
        config.media_roots = vec![];
        let db = Db::open(&dir.path().join("t.db")).unwrap();
        let indexer = Indexer::new(db.clone(), &config).unwrap();
        let router = router(AppState {
            config: Arc::new(config),
            db,
            indexer,
        });
        TestApp { _dir: dir, router }
    }

    async fn get_json(uri: &str) -> (StatusCode, serde_json::Value) {
        let app = app();
        let response = app
            .router
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn health_reports_version() {
        let (status, body) = get_json("/api/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["version"], VERSION);
    }

    #[tokio::test]
    async fn library_status_reports_counts() {
        let (status, body) = get_json("/api/library/status").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["images"], 0);
        assert_eq!(body["scanning"], false);
    }

    #[tokio::test]
    async fn unknown_api_route_is_json_404() {
        let (status, body) = get_json("/api/nope").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "not found");
    }
}
