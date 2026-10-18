//! Hiding photos and folders. Admins hide things for everyone (and only admins can show
//! or unhide those); other users hide things just for themselves. View-only accounts
//! can't hide anything.

use axum::Json;
use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use rusqlite::params;
use serde::Deserialize;

use super::browse::visible_folder;
use super::media::visible;
use super::{ApiError, ApiResult, CurrentUser};
use crate::auth::{self, Role, User};
use crate::server::AppState;

/// Hides go to everyone (no user) for admins, to the user themselves otherwise.
fn owner(user: &User) -> ApiResult<Option<i64>> {
    match user.role {
        Role::Viewer => Err(ApiError::forbidden()),
        Role::Admin => Ok(None),
        Role::Editor => Ok(Some(user.id)),
    }
}

/// Which hide rows a user may remove: admins remove everyone's-hides and their own;
/// others only their own.
fn removable(user: &User) -> String {
    match user.role {
        Role::Admin => format!("(user_id IS NULL OR user_id = {})", user.id),
        _ => format!("user_id = {}", user.id),
    }
}

pub async fn hide_media(
    State(state): State<AppState>,
    current: CurrentUser,
    UrlPath(id): UrlPath<i64>,
) -> ApiResult<StatusCode> {
    let owner = owner(&current.user)?;
    visible(&state, current.user, id).await?;
    state
        .db
        .run(move |conn| {
            conn.execute(
                "INSERT OR IGNORE INTO hidden (user_id, media_id, created_at) VALUES (?1, ?2, ?3)",
                params![owner, id, auth::now()],
            )?;
            Ok(())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unhide_media(
    State(state): State<AppState>,
    current: CurrentUser,
    UrlPath(id): UrlPath<i64>,
) -> ApiResult<StatusCode> {
    owner(&current.user)?;
    let rows = removable(&current.user);
    visible(&state, current.user, id).await?;
    state
        .db
        .run(move |conn| {
            conn.execute(
                &format!("DELETE FROM hidden WHERE media_id = ?1 AND {rows}"),
                [id],
            )?;
            Ok(())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn hide_folder(
    State(state): State<AppState>,
    current: CurrentUser,
    UrlPath(id): UrlPath<i64>,
) -> ApiResult<StatusCode> {
    let owner = owner(&current.user)?;
    let user = current.user;
    state
        .db
        .run(move |conn| {
            Ok(visible_folder(conn, &user, id).and_then(|folder| {
                conn.execute(
                    "INSERT OR IGNORE INTO hidden (user_id, folder_path, created_at) VALUES (?1, ?2, ?3)",
                    params![owner, folder.path, auth::now()],
                )?;
                Ok(())
            }))
        })
        .await??;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unhide_folder(
    State(state): State<AppState>,
    current: CurrentUser,
    UrlPath(id): UrlPath<i64>,
) -> ApiResult<StatusCode> {
    owner(&current.user)?;
    let rows = removable(&current.user);
    let user = current.user;
    state
        .db
        .run(move |conn| {
            Ok(visible_folder(conn, &user, id).and_then(|folder| {
                conn.execute(
                    &format!("DELETE FROM hidden WHERE folder_path = ?1 AND {rows}"),
                    [&folder.path],
                )?;
                Ok(())
            }))
        })
        .await??;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ShowHidden {
    show: bool,
}

/// Turns "show hidden" on or off for the signed-in user; returns the updated user.
pub async fn set_show_hidden(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(body): Json<ShowHidden>,
) -> ApiResult<Json<User>> {
    owner(&current.user)?;
    let id = current.user.id;
    let user = state
        .db
        .run(move |conn| {
            conn.execute(
                "UPDATE users SET show_hidden = ?1 WHERE id = ?2",
                params![body.show, id],
            )?;
            auth::get_user(conn, id)
        })
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    Ok(Json(user))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::{Value, json};

    use crate::api::test_support::TestApp;
    use crate::testdata;

    /// root/
    ///   trip/a.jpg (2024-01-01), trip/b.jpg (2024-01-02)
    ///   trip/day2/c.jpg (2024-01-03)
    ///   home/d.jpg (2023-01-01)
    fn library(app: &TestApp) {
        for (path, date) in [
            ("trip/a.jpg", "2024:01:01 10:00:00"),
            ("trip/b.jpg", "2024:01:02 10:00:00"),
            ("trip/day2/c.jpg", "2024:01:03 10:00:00"),
            ("home/d.jpg", "2023:01:01 10:00:00"),
        ] {
            app.write_media(path, &testdata::jpeg_taken_at(date));
        }
        app.scan();
    }

    async fn timeline(app: &TestApp, cookie: &str) -> Vec<String> {
        let page = app.send("GET", "/api/timeline", None, Some(cookie)).await;
        names(&page.body["items"])
    }

    fn names(items: &Value) -> Vec<String> {
        items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["name"].as_str().unwrap().to_string())
            .collect()
    }

    async fn show_hidden(app: &TestApp, cookie: &str, show: bool) -> StatusCode {
        app.send(
            "PUT",
            "/api/auth/show-hidden",
            Some(json!({ "show": show })),
            Some(cookie),
        )
        .await
        .status
    }

    #[tokio::test]
    async fn admin_hides_for_everyone() {
        let app = TestApp::new();
        library(&app);
        let admin = app.setup_admin().await;
        let ed = app.user_session(&admin, "ed", "editor").await;
        let viewer = app.user_session(&admin, "vi", "viewer").await;
        let b = app.media_id("b.jpg");

        let hide = app
            .send("PUT", &format!("/api/media/{b}/hidden"), None, Some(&admin))
            .await;
        assert_eq!(hide.status, StatusCode::NO_CONTENT);
        for cookie in [&admin, &ed, &viewer] {
            assert_eq!(timeline(&app, cookie).await, ["c.jpg", "a.jpg", "d.jpg"]);
        }
        // Not reachable directly either, and not unhideable, by others.
        for cookie in [&ed, &viewer] {
            let direct = app
                .send("GET", &format!("/api/media/{b}"), None, Some(cookie))
                .await;
            assert_eq!(direct.status, StatusCode::NOT_FOUND);
        }
        let unhide = app
            .send("DELETE", &format!("/api/media/{b}/hidden"), None, Some(&ed))
            .await;
        assert_eq!(unhide.status, StatusCode::NOT_FOUND);
        // An editor's "show hidden" doesn't reveal an admin's hides.
        assert_eq!(show_hidden(&app, &ed, true).await, StatusCode::OK);
        assert_eq!(timeline(&app, &ed).await, ["c.jpg", "a.jpg", "d.jpg"]);
        // Viewers have no "show hidden".
        assert_eq!(
            show_hidden(&app, &viewer, true).await,
            StatusCode::FORBIDDEN
        );

        // The admin sees it marked, and can unhide it.
        show_hidden(&app, &admin, true).await;
        let page = app.send("GET", "/api/timeline", None, Some(&admin)).await;
        assert_eq!(
            names(&page.body["items"]),
            ["c.jpg", "b.jpg", "a.jpg", "d.jpg"]
        );
        assert_eq!(page.body["items"][1]["hidden"], true);
        assert!(page.body["items"][0].get("hidden").is_none());
        let detail = app
            .send("GET", &format!("/api/media/{b}"), None, Some(&admin))
            .await;
        assert_eq!(detail.body["hidden"], "photo");
        let unhide = app
            .send(
                "DELETE",
                &format!("/api/media/{b}/hidden"),
                None,
                Some(&admin),
            )
            .await;
        assert_eq!(unhide.status, StatusCode::NO_CONTENT);
        show_hidden(&app, &admin, false).await;
        assert_eq!(
            timeline(&app, &ed).await,
            ["c.jpg", "b.jpg", "a.jpg", "d.jpg"]
        );
    }

    #[tokio::test]
    async fn users_hide_for_themselves() {
        let app = TestApp::new();
        library(&app);
        let admin = app.setup_admin().await;
        let ed = app.user_session(&admin, "ed", "editor").await;
        let viewer = app.user_session(&admin, "vi", "viewer").await;
        let a = app.media_id("a.jpg");

        // Viewers can't hide.
        let denied = app
            .send(
                "PUT",
                &format!("/api/media/{a}/hidden"),
                None,
                Some(&viewer),
            )
            .await;
        assert_eq!(denied.status, StatusCode::FORBIDDEN);

        app.send("PUT", &format!("/api/media/{a}/hidden"), None, Some(&ed))
            .await;
        assert_eq!(timeline(&app, &ed).await, ["c.jpg", "b.jpg", "d.jpg"]);
        assert_eq!(
            timeline(&app, &admin).await,
            ["c.jpg", "b.jpg", "a.jpg", "d.jpg"]
        );

        // They can show and unhide their own.
        show_hidden(&app, &ed, true).await;
        assert_eq!(
            timeline(&app, &ed).await,
            ["c.jpg", "b.jpg", "a.jpg", "d.jpg"]
        );
        app.send("DELETE", &format!("/api/media/{a}/hidden"), None, Some(&ed))
            .await;
        show_hidden(&app, &ed, false).await;
        assert_eq!(
            timeline(&app, &ed).await,
            ["c.jpg", "b.jpg", "a.jpg", "d.jpg"]
        );
    }

    #[tokio::test]
    async fn hiding_a_folder_hides_everything_below_it() {
        let app = TestApp::new();
        library(&app);
        let admin = app.setup_admin().await;
        let ed = app.user_session(&admin, "ed", "editor").await;
        let trip = app.folder_id("trip");
        let day2 = app.folder_id("trip/day2");
        let root = app.folder_id("");

        app.send(
            "PUT",
            &format!("/api/folders/{day2}/hidden"),
            None,
            Some(&admin),
        )
        .await;
        assert_eq!(timeline(&app, &ed).await, ["b.jpg", "a.jpg", "d.jpg"]);
        let gone = app
            .send("GET", &format!("/api/folders/{day2}"), None, Some(&ed))
            .await;
        assert_eq!(gone.status, StatusCode::NOT_FOUND);
        let c = app.media_id("c.jpg");
        let file = app
            .send("GET", &format!("/api/media/{c}"), None, Some(&ed))
            .await;
        assert_eq!(file.status, StatusCode::NOT_FOUND);
        // The parent's subfolder list, counts and the slideshow leave it out.
        let parent = app
            .send("GET", &format!("/api/folders/{trip}"), None, Some(&ed))
            .await;
        assert_eq!(parent.body["subfolders"].as_array().unwrap().len(), 0);
        let top = app
            .send("GET", &format!("/api/folders/{root}"), None, Some(&ed))
            .await;
        let trip_entry = top.body["subfolders"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == "trip")
            .unwrap()
            .clone();
        assert_eq!(trip_entry["count"], 2);
        let slideshow = app
            .send(
                "GET",
                &format!("/api/folders/{root}/media?recursive=true"),
                None,
                Some(&ed),
            )
            .await;
        assert_eq!(names(&slideshow.body["items"]), ["b.jpg", "a.jpg", "d.jpg"]);
        let albums = app.send("GET", "/api/albums", None, Some(&ed)).await;
        assert_eq!(albums.body["total"], 2);

        // Shown and marked for the admin with "show hidden"; then unhidden.
        show_hidden(&app, &admin, true).await;
        let detail = app
            .send("GET", &format!("/api/folders/{day2}"), None, Some(&admin))
            .await;
        assert_eq!(detail.body["hidden"], "this");
        let c = app.media_id("c.jpg");
        let inside = app
            .send("GET", &format!("/api/media/{c}"), None, Some(&admin))
            .await;
        assert_eq!(inside.body["hidden"], "folder");
        app.send(
            "DELETE",
            &format!("/api/folders/{day2}/hidden"),
            None,
            Some(&admin),
        )
        .await;
        assert_eq!(
            timeline(&app, &ed).await,
            ["c.jpg", "b.jpg", "a.jpg", "d.jpg"]
        );
    }

    #[tokio::test]
    async fn hidden_photos_are_not_folder_covers() {
        let app = TestApp::new();
        library(&app);
        let admin = app.setup_admin().await;
        let b = app.media_id("b.jpg");
        let a = app.media_id("a.jpg");
        // b.jpg is the trip folder's newest photo, so its cover.
        let albums = app.send("GET", "/api/albums", None, Some(&admin)).await;
        let trip = |albums: &Value| {
            albums["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["name"] == "trip")
                .cloned()
        };
        assert_eq!(trip(&albums.body).unwrap()["cover"]["id"], b);

        app.send("PUT", &format!("/api/media/{b}/hidden"), None, Some(&admin))
            .await;
        let albums = app.send("GET", "/api/albums", None, Some(&admin)).await;
        let entry = trip(&albums.body).unwrap();
        assert_eq!(entry["cover"]["id"], a);
        assert_eq!(entry["count"], 1);

        // A folder with everything hidden drops out of Photos.
        app.send("PUT", &format!("/api/media/{a}/hidden"), None, Some(&admin))
            .await;
        let albums = app.send("GET", "/api/albums", None, Some(&admin)).await;
        assert!(trip(&albums.body).is_none());
    }

    #[tokio::test]
    async fn share_links_leave_out_hidden_items() {
        let app = TestApp::new();
        library(&app);
        let admin = app.setup_admin().await;
        let ed = app.user_session(&admin, "ed", "editor").await;
        let trip = app.folder_id("trip");
        let share = app
            .send(
                "POST",
                "/api/shares",
                Some(json!({ "folder_id": trip })),
                Some(&ed),
            )
            .await;
        let token = share.body["token"].as_str().unwrap().to_string();
        let uri = format!("/api/s/{token}/folder?recursive=true");
        assert_eq!(
            app.send("GET", &uri, None, None).await.body["media"]["total"],
            3
        );

        // The creator's own hides and an admin's hides both apply.
        let a = app.media_id("a.jpg");
        let c = app.media_id("c.jpg");
        app.send("PUT", &format!("/api/media/{a}/hidden"), None, Some(&ed))
            .await;
        app.send("PUT", &format!("/api/media/{c}/hidden"), None, Some(&admin))
            .await;
        let listing = app.send("GET", &uri, None, None).await;
        assert_eq!(names(&listing.body["media"]["items"]), ["b.jpg"]);
        let file = app
            .send("GET", &format!("/api/s/{token}/media/{a}/file"), None, None)
            .await;
        assert_eq!(file.status, StatusCode::NOT_FOUND);

        // Hiding the shared folder itself ends the link.
        app.send(
            "PUT",
            &format!("/api/folders/{trip}/hidden"),
            None,
            Some(&admin),
        )
        .await;
        let open = app
            .send("GET", &format!("/api/s/{token}"), None, None)
            .await;
        assert_eq!(open.status, StatusCode::NOT_FOUND);
    }
}
