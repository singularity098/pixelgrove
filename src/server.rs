//! HTTP server setup.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::HeaderValue;
use axum::middleware::{self, Next};
use axum::response::Response;
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
    /// Location data to remove from files served through share links.
    pub locations: Arc<crate::location::Cache>,
}

/// Restricts what pages may load and do: scripts and data only from this server, map
/// tiles from OpenStreetMap, no framing by other sites (clickjacking), no inline
/// scripts. Inline styles stay allowed; the UI and Leaflet position things with them.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; \
     style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https://tile.openstreetmap.org; \
     media-src 'self'; connect-src 'self'; font-src 'self'; worker-src 'self'; \
     manifest-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; \
     frame-ancestors 'none'";

/// Security headers on every response.
async fn security_headers(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let https = crate::api::is_https(&state, request.headers());
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    for (name, value) in [
        ("content-security-policy", CONTENT_SECURITY_POLICY),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        // Share links carry their token in the URL; never pass it on to other sites.
        ("referrer-policy", "no-referrer"),
        ("cross-origin-opener-policy", "same-origin"),
        ("cross-origin-resource-policy", "same-origin"),
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=(), payment=(), usb=()",
        ),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    if https {
        headers.insert(
            "strict-transport-security",
            HeaderValue::from_static("max-age=31536000"),
        );
    }
    response
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .nest("/api", crate::api::router())
        .fallback(assets::serve)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security_headers,
        ))
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
    async fn security_headers_on_every_response() {
        let app = TestApp::new();
        for uri in ["/api/health", "/api/nope", "/", "/timeline"] {
            let headers = app.get_raw(uri, "", None).await.headers().clone();
            let csp = headers["content-security-policy"].to_str().unwrap();
            assert!(csp.contains("frame-ancestors 'none'"), "{uri}");
            assert!(
                csp.contains("script-src 'self';"),
                "{uri}: no inline scripts"
            );
            assert_eq!(headers["x-content-type-options"], "nosniff");
            assert_eq!(headers["referrer-policy"], "no-referrer");
            assert_eq!(headers["x-frame-options"], "DENY");
            assert!(
                !headers.contains_key("strict-transport-security"),
                "plain HTTP"
            );
        }
        let behind_https = app
            .request(
                axum::http::Request::get("/")
                    .header("x-forwarded-proto", "https")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await;
        assert!(
            behind_https
                .headers()
                .contains_key("strict-transport-security")
        );
    }

    #[tokio::test]
    async fn unknown_api_route_is_json_404() {
        let app = TestApp::new();
        let response = app.send("GET", "/api/nope", None, None).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        assert!(response.body["error"].is_string());
    }
}
