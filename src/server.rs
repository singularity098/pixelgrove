//! HTTP routing.

use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

use crate::assets;
use crate::config::Config;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct AppState {
    #[allow(dead_code)] // Read by upcoming API handlers.
    pub config: Arc<Config>,
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
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

    fn app() -> Router {
        router(AppState {
            config: Arc::new(Config::with_config_dir("/tmp/unused".into())),
        })
    }

    async fn get_json(uri: &str) -> (StatusCode, serde_json::Value) {
        let response = app()
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
    async fn unknown_api_route_is_json_404() {
        let (status, body) = get_json("/api/nope").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "not found");
    }
}
