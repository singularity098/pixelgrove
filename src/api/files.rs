//! File View: the real folder tree, including folders without media.

use std::path::Path;

use axum::Json;
use axum::extract::{Path as UrlPath, State};
use rusqlite::{Connection, params_from_iter};
use serde::Serialize;

use super::browse::{load_folder, relative, subtree, visible_folder};
use super::{ApiResult, CurrentUser};
use crate::auth::User;
use crate::server::AppState;

#[derive(Serialize)]
pub struct TreeFolder {
    id: i64,
    name: String,
    path: String,
    /// Media directly inside.
    media_count: i64,
    /// Media in this folder and everything below it.
    total_count: i64,
    subfolder_count: i64,
    latest_taken_at: Option<i64>,
}

fn tree_folder(conn: &Connection, id: i64) -> anyhow::Result<Option<TreeFolder>> {
    let Some(folder) = load_folder(conn, id)? else {
        return Ok(None);
    };
    let (cond, params) = subtree("f.path", &folder.path);
    let (total_count, latest): (i64, Option<i64>) = conn.query_row(
        &format!(
            "SELECT coalesce(sum(f.media_count), 0), max(f.latest_taken_at) FROM folders f WHERE {cond}"
        ),
        params_from_iter(&params),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let subfolder_count: i64 = conn.query_row(
        "SELECT count(*) FROM folders WHERE parent_id = ?1",
        [id],
        |r| r.get(0),
    )?;
    Ok(Some(TreeFolder {
        id: folder.id,
        name: super::browse::display_name(&folder),
        path: relative(&folder.path, &folder.root),
        media_count: folder.media_count,
        total_count,
        subfolder_count,
        latest_taken_at: latest,
    }))
}

/// The top of the tree for this user: the media roots, or the folders they're
/// limited to.
pub async fn tops(
    State(state): State<AppState>,
    current: CurrentUser,
) -> ApiResult<Json<Vec<TreeFolder>>> {
    let user = current.user;
    let folders = state
        .db
        .run(move |conn| {
            let ids = top_ids(conn, &user)?;
            let mut folders = Vec::new();
            for id in ids {
                folders.extend(tree_folder(conn, id)?);
            }
            Ok(folders)
        })
        .await?;
    Ok(Json(folders))
}

fn top_ids(conn: &Connection, user: &User) -> anyhow::Result<Vec<i64>> {
    let paths: Vec<String> = if user.folders.is_empty() {
        conn.prepare("SELECT path FROM roots ORDER BY path")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    } else {
        user.folders.clone()
    };
    let mut ids = Vec::new();
    for path in paths {
        let id: Option<i64> = conn
            .query_row("SELECT id FROM folders WHERE path = ?1", [&path], |r| {
                r.get(0)
            })
            .ok();
        ids.extend(id);
    }
    Ok(ids)
}

/// A folder's child folders (all of them, alphabetically).
pub async fn children(
    State(state): State<AppState>,
    current: CurrentUser,
    UrlPath(id): UrlPath<i64>,
) -> ApiResult<Json<Vec<TreeFolder>>> {
    let user = current.user;
    let folders = state
        .db
        .run(move |conn| Ok(children_of(conn, &user, id)))
        .await??;
    Ok(Json(folders))
}

fn children_of(conn: &Connection, user: &User, id: i64) -> ApiResult<Vec<TreeFolder>> {
    let folder = visible_folder(conn, user, id)?;
    let ids: Vec<(i64, String)> = conn
        .prepare("SELECT id, path FROM folders WHERE parent_id = ?1 ORDER BY name COLLATE NOCASE")?
        .query_map([folder.id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut folders = Vec::new();
    for (child, path) in ids {
        if user.can_access(Path::new(&path)) {
            folders.extend(tree_folder(conn, child)?);
        }
    }
    Ok(folders)
}

#[cfg(test)]
mod tests {
    use crate::api::test_support::TestApp;
    use crate::testdata;

    #[tokio::test]
    async fn tree_includes_empty_folders() {
        let app = TestApp::new();
        app.write_media("photos/a/1.jpg", &testdata::jpeg_with_exif());
        std::fs::create_dir_all(app.media_root().join("photos/empty")).unwrap();
        app.scan();
        let admin = app.setup_admin().await;

        let tops = app.send("GET", "/api/files", None, Some(&admin)).await;
        assert_eq!(tops.body.as_array().unwrap().len(), 1);
        assert_eq!(tops.body[0]["total_count"], 1);
        let root = tops.body[0]["id"].as_i64().unwrap();

        let children = app
            .send("GET", &format!("/api/files/{root}"), None, Some(&admin))
            .await;
        assert_eq!(children.body[0]["name"], "photos");
        assert_eq!(children.body[0]["subfolder_count"], 2);
        let photos = children.body[0]["id"].as_i64().unwrap();
        let photos_children = app
            .send("GET", &format!("/api/files/{photos}"), None, Some(&admin))
            .await;
        let names: Vec<&str> = photos_children
            .body
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["a", "empty"]);
        assert_eq!(photos_children.body[1]["total_count"], 0);
    }

    #[tokio::test]
    async fn limited_users_start_at_their_folders() {
        let app = TestApp::new();
        app.write_media("kids/1.jpg", &testdata::jpeg_with_exif());
        app.write_media("work/2.jpg", &testdata::jpeg_with_exif());
        app.scan();
        let admin = app.setup_admin().await;
        let kids = format!("{}/kids", app.media_root().display());
        app.send(
            "POST",
            "/api/users",
            Some(serde_json::json!({"username": "k", "password": "kid-password", "role": "viewer", "folders": [kids]})),
            Some(&admin),
        )
        .await;
        let kid = app.login("k", "kid-password").await.cookie.unwrap();
        let tops = app.send("GET", "/api/files", None, Some(&kid)).await;
        assert_eq!(tops.body.as_array().unwrap().len(), 1);
        assert_eq!(tops.body[0]["name"], "kids");
    }
}
