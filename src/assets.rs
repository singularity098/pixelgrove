//! The web UI, embedded into the binary at build time from `web/dist`.

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/dist/"]
#[allow_missing = true]
struct WebAssets;

/// Serves a file from the embedded UI, falling back to `index.html` so client-side
/// routes (e.g. `/timeline`) load the app.
pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if !path.is_empty()
        && let Some(response) = file_response(path)
    {
        return response;
    }
    file_response("index.html").unwrap_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "PixelGrove web UI was not built into this binary (run `npm run build` in web/).",
        )
            .into_response()
    })
}

fn file_response(path: &str) -> Option<Response> {
    let file = WebAssets::get(path)?;
    // Vite fingerprints everything under assets/, so those can be cached forever.
    let cache_control = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let mime = HeaderValue::from_str(file.metadata.mimetype()).ok()?;
    let mut response = Response::new(Body::from(file.data.into_owned()));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, mime);
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control),
    );
    Some(response)
}
