//! A single photo or video: details, previews and the original file.

use std::path::{Path, PathBuf};

use axum::Json;
use axum::body::Body;
use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use super::{ApiError, ApiResult, CurrentUser};
use crate::auth::User;
use crate::preview::{Key, Preview, Variant};
use crate::server::AppState;

pub(super) struct MediaRow {
    pub id: i64,
    pub folder_id: i64,
    pub folder_path: String,
    pub name: String,
    pub kind: String,
    pub size: i64,
    pub mtime: i64,
    pub taken_at: i64,
    pub tz_offset: Option<i32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub orientation: Option<u16>,
    pub duration_ms: Option<i64>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub description: Option<String>,
}

impl MediaRow {
    pub fn path(&self) -> PathBuf {
        Path::new(&self.folder_path).join(&self.name)
    }
}

pub(super) fn load(conn: &Connection, id: i64) -> rusqlite::Result<Option<MediaRow>> {
    conn.query_row(
        "SELECT m.id, m.folder_id, f.path, m.name, m.kind, m.size, m.mtime, m.taken_at,
                m.tz_offset, m.width, m.height, m.orientation, m.duration_ms, m.camera_make,
                m.camera_model, m.lens, m.latitude, m.longitude, m.description
         FROM media m JOIN folders f ON f.id = m.folder_id WHERE m.id = ?1",
        [id],
        |r| {
            Ok(MediaRow {
                id: r.get(0)?,
                folder_id: r.get(1)?,
                folder_path: r.get(2)?,
                name: r.get(3)?,
                kind: r.get(4)?,
                size: r.get(5)?,
                mtime: r.get(6)?,
                taken_at: r.get(7)?,
                tz_offset: r.get(8)?,
                width: r.get(9)?,
                height: r.get(10)?,
                orientation: r.get(11)?,
                duration_ms: r.get(12)?,
                camera_make: r.get(13)?,
                camera_model: r.get(14)?,
                lens: r.get(15)?,
                latitude: r.get(16)?,
                longitude: r.get(17)?,
                description: r.get(18)?,
            })
        },
    )
    .optional()
}

/// Loads media the user may see, or 404s (also for media they may not see).
pub(super) async fn visible(state: &AppState, user: User, id: i64) -> ApiResult<MediaRow> {
    state
        .db
        .run(move |conn| load(conn, id).map_err(Into::into))
        .await?
        .filter(|m| user.can_access(Path::new(&m.folder_path)))
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND, "Not found."))
}

#[derive(Serialize)]
pub struct MediaDetail {
    id: i64,
    folder_id: i64,
    name: String,
    /// Absolute path on the server, shown in the info panel.
    path: String,
    kind: String,
    size: i64,
    modified_at: i64,
    taken_at: i64,
    tz_offset: Option<i32>,
    width: Option<u32>,
    height: Option<u32>,
    duration_ms: Option<i64>,
    camera_make: Option<String>,
    camera_model: Option<String>,
    lens: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    description: Option<String>,
    /// Whether the signed-in user has favorited it.
    favorite: bool,
    v: i64,
}

pub async fn detail(
    State(state): State<AppState>,
    current: CurrentUser,
    UrlPath(id): UrlPath<i64>,
) -> ApiResult<Json<MediaDetail>> {
    let user_id = current.user.id;
    let m = visible(&state, current.user, id).await?;
    let favorite = state
        .db
        .run(move |conn| super::favorites::is_favorite(conn, user_id, id))
        .await?;
    let (width, height) = if m.orientation.is_some_and(|o| o >= 5) {
        (m.height, m.width)
    } else {
        (m.width, m.height)
    };
    Ok(Json(MediaDetail {
        path: m.path().to_string_lossy().into_owned(),
        id: m.id,
        folder_id: m.folder_id,
        name: m.name,
        kind: m.kind,
        size: m.size,
        modified_at: m.mtime,
        taken_at: m.taken_at,
        tz_offset: m.tz_offset,
        width,
        height,
        duration_ms: m.duration_ms,
        camera_make: m.camera_make,
        camera_model: m.camera_model,
        lens: m.lens,
        latitude: m.latitude,
        longitude: m.longitude,
        description: m.description,
        favorite,
        v: m.mtime,
    }))
}

#[derive(Deserialize)]
pub struct Versioned {
    /// The media's mtime as the client knew it; a match means the response can be
    /// cached forever (the URL changes when the file does).
    pub v: Option<i64>,
    #[serde(default)]
    pub download: bool,
}

fn cache_control(versioned: &Versioned, mtime: i64) -> HeaderValue {
    HeaderValue::from_static(if versioned.v == Some(mtime) {
        "private, max-age=31536000, immutable"
    } else {
        "private, no-cache"
    })
}

/// A file to serve a preview or the original of (library media or a trashed item).
pub(super) struct FileRef<'a> {
    /// Identifies cached previews; trashed items use negative ids.
    pub cache_id: i64,
    pub path: &'a Path,
    pub name: &'a str,
    pub kind: &'a str,
    pub mtime: i64,
    pub size: i64,
    pub orientation: Option<u16>,
}

pub(super) async fn preview_response(
    state: &AppState,
    file: FileRef<'_>,
    variant: &str,
    versioned: &Versioned,
    request: Request,
) -> ApiResult<Response> {
    let variant = Variant::parse(variant)
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND, "Unknown preview size."))?;
    if file.kind != "image" {
        return Err(ApiError::status(
            StatusCode::NOT_FOUND,
            "Videos have no preview image.",
        ));
    }
    let key = Key {
        media_id: file.cache_id,
        variant,
        mtime: file.mtime,
        size: file.size,
    };
    let preview = match state.previews.get(key, file.path, file.orientation).await {
        Ok(preview) => preview,
        Err(err) => {
            // A corrupt or unusual file: let the browser try the original.
            tracing::debug!(path = %file.path.display(), "preview failed, serving original: {err:#}");
            Preview::Original
        }
    };
    let mut response = match preview {
        Preview::Jpeg(bytes) => (
            [(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"))],
            Body::from(bytes),
        )
            .into_response(),
        Preview::Original => serve_file(file.path, request).await?,
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, cache_control(versioned, file.mtime));
    Ok(response)
}

/// The original file, with HTTP range support (needed for video seeking).
pub(super) async fn file_response(
    file: FileRef<'_>,
    versioned: &Versioned,
    request: Request,
) -> ApiResult<Response> {
    let mut response = serve_file(file.path, request).await?;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, cache_control(versioned, file.mtime));
    if file.name.to_ascii_lowercase().ends_with(".mov") {
        // Most .mov files are H.264/HEVC that browsers play fine, but only when labelled
        // as MP4.
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("video/mp4"));
    }
    if versioned.download {
        headers.insert(header::CONTENT_DISPOSITION, attachment(file.name));
    }
    Ok(response)
}

impl MediaRow {
    fn file_ref<'a>(&'a self, path: &'a Path) -> FileRef<'a> {
        FileRef {
            cache_id: self.id,
            path,
            name: &self.name,
            kind: &self.kind,
            mtime: self.mtime,
            size: self.size,
            orientation: self.orientation,
        }
    }
}

pub async fn preview(
    State(state): State<AppState>,
    current: CurrentUser,
    UrlPath((id, variant)): UrlPath<(i64, String)>,
    Query(versioned): Query<Versioned>,
    request: Request,
) -> ApiResult<Response> {
    let m = visible(&state, current.user, id).await?;
    let path = m.path();
    preview_response(&state, m.file_ref(&path), &variant, &versioned, request).await
}

pub async fn file(
    State(state): State<AppState>,
    current: CurrentUser,
    UrlPath(id): UrlPath<i64>,
    Query(versioned): Query<Versioned>,
    request: Request,
) -> ApiResult<Response> {
    let m = visible(&state, current.user, id).await?;
    let path = m.path();
    file_response(m.file_ref(&path), &versioned, request).await
}

async fn serve_file(path: &Path, request: Request) -> ApiResult<Response> {
    let response = ServeFile::new(path).oneshot(request).await?;
    if response.status() == StatusCode::NOT_FOUND {
        return Err(ApiError::status(
            StatusCode::NOT_FOUND,
            "The file is no longer on disk.",
        ));
    }
    Ok(response.map(Body::new))
}

/// `Content-Disposition: attachment` with an ASCII fallback name and the real name
/// percent-encoded (RFC 6266 / 5987).
fn attachment(name: &str) -> HeaderValue {
    let fallback: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    HeaderValue::from_str(&format!(
        "attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::attachment;
    use crate::api::test_support::TestApp;
    use crate::testdata;

    #[tokio::test]
    async fn detail_preview_and_file() {
        let app = TestApp::new();
        app.write_media("trip/a.jpg", &testdata::jpeg_with_exif());
        app.scan();
        let admin = app.setup_admin().await;
        let id = app.media_id("a.jpg");

        let detail = app
            .send("GET", &format!("/api/media/{id}"), None, Some(&admin))
            .await;
        assert_eq!(detail.body["camera_make"], "PixelCam");
        let v = detail.body["v"].as_i64().unwrap();

        // Tiny file: the preview is the original.
        let preview = app
            .get_raw(
                &format!("/api/media/{id}/preview/thumb?v={v}"),
                &admin,
                None,
            )
            .await;
        assert_eq!(preview.status(), StatusCode::OK);
        assert_eq!(
            preview.headers()["cache-control"],
            "private, max-age=31536000, immutable"
        );

        let ranged = app
            .get_raw(
                &format!("/api/media/{id}/file?download=true"),
                &admin,
                Some("bytes=0-1"),
            )
            .await;
        assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT);
        assert!(
            ranged.headers()["content-disposition"]
                .to_str()
                .unwrap()
                .starts_with("attachment")
        );
        assert_eq!(ranged.headers()["cache-control"], "private, no-cache");

        assert_eq!(
            app.send("GET", "/api/media/9999", None, Some(&admin))
                .await
                .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            app.get_raw(&format!("/api/media/{id}/preview/huge"), &admin, None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn media_outside_folder_limit_is_hidden() {
        let app = TestApp::new();
        app.write_media("public/a.jpg", &testdata::jpeg_with_exif());
        app.write_media("private/b.jpg", &testdata::jpeg_with_exif());
        app.scan();
        let admin = app.setup_admin().await;
        let public = format!("{}/public", app.media_root().display());
        app.send(
            "POST",
            "/api/users",
            Some(serde_json::json!({
                "username": "guest", "password": "guest-password", "role": "viewer",
                "folders": [public]
            })),
            Some(&admin),
        )
        .await;
        let guest = app.login("guest", "guest-password").await.cookie.unwrap();
        let private = app.media_id("b.jpg");
        for uri in [
            format!("/api/media/{private}"),
            format!("/api/media/{private}/file"),
            format!("/api/media/{private}/preview/thumb"),
        ] {
            assert_eq!(
                app.get_raw(&uri, &guest, None).await.status(),
                StatusCode::NOT_FOUND,
                "{uri}"
            );
        }
        let public_id = app.media_id("a.jpg");
        assert_eq!(
            app.get_raw(&format!("/api/media/{public_id}/file"), &guest, None)
                .await
                .status(),
            StatusCode::OK
        );
    }

    #[test]
    fn attachment_names() {
        assert_eq!(
            attachment("IMG 1.jpg"),
            "attachment; filename=\"IMG 1.jpg\"; filename*=UTF-8''IMG%201.jpg"
        );
        assert_eq!(
            attachment("été\".jpg"),
            "attachment; filename=\"_t__.jpg\"; filename*=UTF-8''%C3%A9t%C3%A9%22.jpg"
        );
    }
}
