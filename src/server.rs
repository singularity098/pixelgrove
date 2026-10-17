//! HTTP server setup.

use std::sync::Arc;

use axum::Router;
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

use crate::assets;
use crate::auth::LoginThrottle;
use crate::config::Config;
use crate::db::Db;
use crate::indexer::Indexer;
use crate::preview::Previews;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Db,
    pub indexer: Indexer,
    pub login_throttle: Arc<LoginThrottle>,
    pub previews: Arc<Previews>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .nest("/api", crate::api::router())
        .fallback(assets::serve)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::VERSION;
    use crate::api::test_support::TestApp;

    #[tokio::test]
    async fn health_is_public() {
        let app = TestApp::new();
        let response = app.send("GET", "/api/health", None, None).await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body["version"], VERSION);
    }

    #[tokio::test]
    async fn unknown_api_route_is_json_404() {
        let app = TestApp::new();
        let response = app.send("GET", "/api/nope", None, None).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        assert!(response.body["error"].is_string());
    }
}
