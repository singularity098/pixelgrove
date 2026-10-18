//! Library-wide information.

use std::path::{Path, PathBuf};

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
            let visible = super::browse::not_hidden(&user.hidden, "m.id");
            Ok(conn.query_row(
                &format!(
                    "SELECT (SELECT count(*) FROM folders f WHERE {scope}),
                            (SELECT coalesce(sum(m.kind = 'image'), 0) FROM media m
                             JOIN folders f ON f.id = m.folder_id WHERE {scope} AND {visible}),
                            (SELECT coalesce(sum(m.kind = 'video'), 0) FROM media m
                             JOIN folders f ON f.id = m.folder_id WHERE {scope} AND {visible})"
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
    /// Where the volume is mounted (or the media folder itself), for display.
    paths: Vec<String>,
    total: u64,
    free: u64,
    used: u64,
}

/// Mount points, from `/proc/self/mountinfo` (empty where that doesn't exist).
fn mount_points() -> Vec<PathBuf> {
    std::fs::read_to_string("/proc/self/mountinfo")
        .map(|info| {
            info.lines()
                .filter_map(|line| line.split(' ').nth(4))
                .map(|p| PathBuf::from(unescape_mount_path(p)))
                .collect()
        })
        .unwrap_or_default()
}

/// Mount paths escape spaces, tabs, newlines and backslashes as octal (`\040`).
fn unescape_mount_path(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && let Some(code) = std::str::from_utf8(&bytes[i + 1..i + 4])
                .ok()
                .and_then(|octal| u8::from_str_radix(octal, 8).ok())
        {
            out.push(code);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The paths whose filesystems hold a media folder's files: the volumes mounted at or
/// below it. In Docker the default `/media` is a plain folder in the container with
/// the libraries mounted under it, so its own filesystem (the container's disk) is
/// left out; with nothing mounted there, the folder's own filesystem is used.
fn volume_paths(root: &Path, mounts: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = mounts
        .iter()
        .filter(|m| m.starts_with(root) && *m != root)
        .cloned()
        .collect();
    if paths.is_empty() || mounts.iter().any(|m| m == root) {
        paths.insert(0, root.to_path_buf());
    }
    paths.dedup();
    paths
}

/// Disk usage of the volumes holding the media folders (one entry per filesystem).
pub async fn storage(
    State(state): State<AppState>,
    _: CurrentUser,
) -> ApiResult<Json<Vec<Volume>>> {
    let roots: Vec<PathBuf> = state.indexer.roots().into_iter().map(|(_, r)| r).collect();
    let volumes = tokio::task::spawn_blocking(move || {
        let mounts = mount_points();
        let mut volumes: Vec<(u64, Volume)> = Vec::new();
        for path in roots.iter().flat_map(|root| volume_paths(root, &mounts)) {
            let (Ok(stats), Ok(meta)) = (rustix::fs::statvfs(&path), std::fs::metadata(&path))
            else {
                continue;
            };
            let device = std::os::unix::fs::MetadataExt::dev(&meta);
            let name = path.to_string_lossy().into_owned();
            if let Some((_, volume)) = volumes.iter_mut().find(|(d, _)| *d == device) {
                if !volume.paths.contains(&name) {
                    volume.paths.push(name);
                }
                continue;
            }
            let block = stats.f_frsize;
            let total = stats.f_blocks * block;
            if total == 0 {
                continue;
            }
            volumes.push((
                device,
                Volume {
                    paths: vec![name],
                    total,
                    free: stats.f_bavail * block,
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

    use std::path::{Path, PathBuf};

    use super::{unescape_mount_path, volume_paths};
    use crate::api::test_support::TestApp;

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn volumes_are_the_mounts_under_a_media_folder() {
        // Docker: /media is a plain folder with libraries mounted under it.
        let mounts = paths(&["/", "/config", "/media/photos", "/media/videos", "/mediax"]);
        assert_eq!(
            volume_paths(Path::new("/media"), &mounts),
            paths(&["/media/photos", "/media/videos"])
        );
        // The media folder is itself a mount, with another one inside.
        let mounts = paths(&["/", "/data", "/data/archive"]);
        assert_eq!(
            volume_paths(Path::new("/data"), &mounts),
            paths(&["/data", "/data/archive"])
        );
        // Nothing mounted there: its own filesystem.
        assert_eq!(
            volume_paths(Path::new("/home/me/Pictures"), &paths(&["/"])),
            paths(&["/home/me/Pictures"])
        );
    }

    #[test]
    fn mount_paths_are_unescaped() {
        assert_eq!(
            unescape_mount_path(r"/media/My\040Photos"),
            "/media/My Photos"
        );
        assert_eq!(unescape_mount_path("/media/plain"), "/media/plain");
        assert_eq!(unescape_mount_path(r"/media/\é"), r"/media/\é");
    }

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
