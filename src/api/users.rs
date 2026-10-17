//! User management (admins only).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;

use super::{AdminUser, ApiError, ApiResult};
use crate::auth::{self, NewUser, Role, UpdateError, User, UserChanges};
use crate::server::AppState;

pub async fn list(State(state): State<AppState>, _: AdminUser) -> ApiResult<Json<Vec<User>>> {
    Ok(Json(state.db.run(|conn| auth::list_users(conn)).await?))
}

#[derive(Deserialize)]
pub struct CreateUser {
    username: String,
    password: String,
    role: Role,
    #[serde(default)]
    folders: Vec<String>,
}

pub async fn create(
    State(state): State<AppState>,
    _: AdminUser,
    Json(body): Json<CreateUser>,
) -> ApiResult<(StatusCode, Json<User>)> {
    let username = auth::clean_username(&body.username).map_err(ApiError::bad_request)?;
    auth::check_password(&body.password).map_err(ApiError::bad_request)?;
    let folders = auth::clean_folders(&body.folders, &state.config.media_roots)
        .map_err(ApiError::bad_request)?;
    let user = state
        .db
        .run(move |conn| {
            auth::create_user(
                conn,
                &NewUser {
                    username: &username,
                    password: &body.password,
                    role: body.role,
                    folders: &folders,
                },
            )
        })
        .await?
        .ok_or_else(|| ApiError::status(StatusCode::CONFLICT, "That username is taken."))?;
    Ok((StatusCode::CREATED, Json(user)))
}

pub async fn update(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
    Json(mut changes): Json<UserChanges>,
) -> ApiResult<Json<User>> {
    if id == admin.user.id && (changes.role.is_some() || changes.disabled.is_some()) {
        return Err(ApiError::bad_request(
            "You can't change your own role or disable your own account.",
        ));
    }
    if let Some(password) = &changes.password {
        auth::check_password(password).map_err(ApiError::bad_request)?;
    }
    if let Some(folders) = &changes.folders {
        changes.folders = Some(
            auth::clean_folders(folders, &state.config.media_roots)
                .map_err(ApiError::bad_request)?,
        );
    }
    let result = state
        .db
        .run(move |conn| auth::update_user(conn, id, &changes))
        .await?;
    result.map(Json).map_err(update_error)
}

pub async fn delete(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    if id == admin.user.id {
        return Err(ApiError::bad_request("You can't delete your own account."));
    }
    let result = state
        .db
        .run(move |conn| auth::delete_user(conn, id))
        .await?;
    result
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(update_error)
}

fn update_error(err: UpdateError) -> ApiError {
    match err {
        UpdateError::NotFound => ApiError::status(StatusCode::NOT_FOUND, "No such user."),
        UpdateError::LastAdmin => {
            ApiError::bad_request("There must be at least one enabled admin.")
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::json;

    use crate::api::test_support::TestApp;

    #[tokio::test]
    async fn admin_manages_users() {
        let app = TestApp::new();
        let admin = app.setup_admin().await;

        let created = app
            .send(
                "POST",
                "/api/users",
                Some(json!({
                    "username": "kid", "password": "kid-password", "role": "viewer",
                    "folders": ["/media/kids/"]
                })),
                Some(&admin),
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED);
        assert_eq!(created.body["folders"], json!(["/media/kids"]));
        let id = created.body["id"].as_i64().unwrap();

        let dup = app
            .send(
                "POST",
                "/api/users",
                Some(json!({"username": "KID", "password": "kid-password", "role": "viewer"})),
                Some(&admin),
            )
            .await;
        assert_eq!(dup.status, StatusCode::CONFLICT);

        let outside = app
            .send(
                "POST",
                "/api/users",
                Some(json!({
                    "username": "x", "password": "x-password", "role": "viewer",
                    "folders": ["/etc"]
                })),
                Some(&admin),
            )
            .await;
        assert_eq!(outside.status, StatusCode::BAD_REQUEST);

        let updated = app
            .send(
                "PATCH",
                &format!("/api/users/{id}"),
                Some(json!({"role": "editor", "folders": []})),
                Some(&admin),
            )
            .await;
        assert_eq!(updated.body["role"], "editor");
        assert_eq!(updated.body["folders"], json!([]));

        let list = app.send("GET", "/api/users", None, Some(&admin)).await;
        assert_eq!(list.body.as_array().unwrap().len(), 2);
        assert!(
            list.body[0].get("password_hash").is_none(),
            "hashes never leave the server"
        );

        let deleted = app
            .send("DELETE", &format!("/api/users/{id}"), None, Some(&admin))
            .await;
        assert_eq!(deleted.status, StatusCode::NO_CONTENT);
        assert_eq!(
            app.login("kid", "kid-password").await.status,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn non_admins_are_refused() {
        let app = TestApp::new();
        let admin = app.setup_admin().await;
        let editor = app.user_session(&admin, "ed", "editor").await;
        assert_eq!(
            app.send("GET", "/api/users", None, Some(&editor))
                .await
                .status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            app.send("GET", "/api/users", None, None).await.status,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn admins_cannot_lock_themselves_out() {
        let app = TestApp::new();
        let admin = app.setup_admin().await;
        let me = app.send("GET", "/api/auth/me", None, Some(&admin)).await;
        let id = me.body["id"].as_i64().unwrap();
        let demote = app
            .send(
                "PATCH",
                &format!("/api/users/{id}"),
                Some(json!({"role": "viewer"})),
                Some(&admin),
            )
            .await;
        assert_eq!(demote.status, StatusCode::BAD_REQUEST);
        let delete = app
            .send("DELETE", &format!("/api/users/{id}"), None, Some(&admin))
            .await;
        assert_eq!(delete.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn disabled_users_are_signed_out() {
        let app = TestApp::new();
        let admin = app.setup_admin().await;
        let session = app.user_session(&admin, "ed", "editor").await;
        let list = app.send("GET", "/api/users", None, Some(&admin)).await;
        let id = list
            .body
            .as_array()
            .unwrap()
            .iter()
            .find(|u| u["username"] == "ed")
            .unwrap()["id"]
            .as_i64()
            .unwrap();
        app.send(
            "PATCH",
            &format!("/api/users/{id}"),
            Some(json!({"disabled": true})),
            Some(&admin),
        )
        .await;
        assert_eq!(
            app.send("GET", "/api/auth/me", None, Some(&session))
                .await
                .status,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            app.login("ed", "user-password").await.status,
            StatusCode::UNAUTHORIZED
        );
    }
}
