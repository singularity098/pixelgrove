//! Per-user favorites. View-only accounts have no changeable options, so favoriting
//! needs the editor role.

use axum::Json;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::StatusCode;
use rusqlite::{Connection, params, params_from_iter};

use super::browse::{MEDIA_ITEM_COLUMNS, MediaItem, Page, Paged, scope};
use super::{ApiResult, CurrentUser};
use crate::auth::Role;
use crate::server::AppState;

pub fn is_favorite(conn: &Connection, user_id: i64, media_id: i64) -> anyhow::Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM favorites WHERE user_id = ?1 AND media_id = ?2)",
        [user_id, media_id],
        |r| r.get(0),
    )?)
}

pub async fn add(
    State(state): State<AppState>,
    current: CurrentUser,
    UrlPath(id): UrlPath<i64>,
) -> ApiResult<StatusCode> {
    current.require(Role::Editor)?;
    let user_id = current.user.id;
    // Checks the media exists and is visible to this user.
    super::media::visible(&state, current.user, id).await?;
    state
        .db
        .run(move |conn| {
            conn.execute(
                "INSERT OR IGNORE INTO favorites (user_id, media_id, created_at) VALUES (?1, ?2, ?3)",
                params![user_id, id, crate::auth::now()],
            )?;
            Ok(())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove(
    State(state): State<AppState>,
    current: CurrentUser,
    UrlPath(id): UrlPath<i64>,
) -> ApiResult<StatusCode> {
    current.require(Role::Editor)?;
    let user_id = current.user.id;
    state
        .db
        .run(move |conn| {
            conn.execute(
                "DELETE FROM favorites WHERE user_id = ?1 AND media_id = ?2",
                [user_id, id],
            )?;
            Ok(())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The signed-in user's favorites, newest photo first.
pub async fn list(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(page): Query<Page>,
) -> ApiResult<Json<Paged<MediaItem>>> {
    let user = current.user;
    let (offset, limit) = page.clamped();
    let result = state
        .db
        .run(move |conn| {
            // Folder limits still apply, in case they changed after favoriting.
            let (where_scope, scope_params) = scope(&user, "f.path");
            let mut params = vec![user.id.to_string()];
            params.extend(scope_params);
            let from = format!(
                "FROM favorites fav JOIN media m ON m.id = fav.media_id
                 JOIN folders f ON f.id = m.folder_id
                 WHERE fav.user_id = CAST(? AS INTEGER) AND {where_scope}"
            );
            let total: i64 = conn.query_row(
                &format!("SELECT count(*) {from}"),
                params_from_iter(&params),
                |r| r.get(0),
            )?;
            params.push(limit.to_string());
            params.push(offset.to_string());
            let items = conn
                .prepare(&format!(
                    "SELECT {MEDIA_ITEM_COLUMNS} {from}
                     ORDER BY m.taken_at DESC, m.id DESC
                     LIMIT CAST(? AS INTEGER) OFFSET CAST(? AS INTEGER)"
                ))?
                .query_map(params_from_iter(&params), MediaItem::from_row)?
                .collect::<rusqlite::Result<_>>()?;
            Ok(Paged { items, total })
        })
        .await?;
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use crate::api::test_support::TestApp;
    use crate::testdata;

    #[tokio::test]
    async fn favorites_are_per_user() {
        let app = TestApp::new();
        app.write_media("a/1.jpg", &testdata::jpeg_taken_at("2024:01:01 10:00:00"));
        app.write_media("a/2.jpg", &testdata::jpeg_taken_at("2024:01:02 10:00:00"));
        app.scan();
        let admin = app.setup_admin().await;
        let editor = app.user_session(&admin, "ed", "editor").await;
        let viewer = app.user_session(&admin, "vi", "viewer").await;
        let one = app.media_id("1.jpg");

        let add = app
            .send(
                "PUT",
                &format!("/api/media/{one}/favorite"),
                None,
                Some(&editor),
            )
            .await;
        assert_eq!(add.status, StatusCode::NO_CONTENT);
        // Adding twice is fine.
        app.send(
            "PUT",
            &format!("/api/media/{one}/favorite"),
            None,
            Some(&editor),
        )
        .await;

        let mine = app.send("GET", "/api/favorites", None, Some(&editor)).await;
        assert_eq!(mine.body["total"], 1);
        assert_eq!(mine.body["items"][0]["name"], "1.jpg");
        let detail = app
            .send("GET", &format!("/api/media/{one}"), None, Some(&editor))
            .await;
        assert_eq!(detail.body["favorite"], true);

        let admins = app.send("GET", "/api/favorites", None, Some(&admin)).await;
        assert_eq!(admins.body["total"], 0, "favorites are per user");

        let refused = app
            .send(
                "PUT",
                &format!("/api/media/{one}/favorite"),
                None,
                Some(&viewer),
            )
            .await;
        assert_eq!(refused.status, StatusCode::FORBIDDEN);
        let missing = app
            .send("PUT", "/api/media/9999/favorite", None, Some(&editor))
            .await;
        assert_eq!(missing.status, StatusCode::NOT_FOUND);

        app.send(
            "DELETE",
            &format!("/api/media/{one}/favorite"),
            None,
            Some(&editor),
        )
        .await;
        let mine = app.send("GET", "/api/favorites", None, Some(&editor)).await;
        assert_eq!(mine.body["total"], 0);
    }
}
