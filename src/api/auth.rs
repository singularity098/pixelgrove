//! First-run setup, sign-in and the signed-in user's own account.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use serde::{Deserialize, Serialize};

use super::{ApiError, ApiResult, CurrentUser, SESSION_COOKIE, is_https};
use crate::auth::{self, Role, User};
use crate::server::AppState;

#[derive(Serialize)]
pub struct SetupStatus {
    needs_setup: bool,
}

pub async fn setup_status(State(state): State<AppState>) -> ApiResult<Json<SetupStatus>> {
    let count = state.db.run(|conn| auth::user_count(conn)).await?;
    Ok(Json(SetupStatus {
        needs_setup: count == 0,
    }))
}

#[derive(Deserialize)]
pub struct Credentials {
    username: String,
    password: String,
}

/// Creates the first admin account and signs it in. Only works while there are no users.
pub async fn setup(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<Credentials>,
) -> ApiResult<(CookieJar, Json<User>)> {
    let username = auth::clean_username(&body.username).map_err(ApiError::bad_request)?;
    auth::check_password(&body.password).map_err(ApiError::bad_request)?;
    let user = state
        .db
        .run(move |conn| auth::create_first_admin(conn, &username, &body.password))
        .await?
        .ok_or_else(|| ApiError::status(StatusCode::CONFLICT, "Setup is already complete."))?;
    tracing::info!(username = %user.username, "created admin account during setup");
    start_session(&state, &headers, jar, user).await
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<Credentials>,
) -> ApiResult<(CookieJar, Json<User>)> {
    let username = body.username.trim().to_string();
    if state.login_throttle.is_locked(&username) {
        return Err(ApiError::status(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many failed attempts. Try again in a few minutes.",
        ));
    }
    let lookup = username.clone();
    let password = body.password;
    let user = state
        .db
        .run(move |conn| {
            let Some((id, hash)) = auth::login_lookup(conn, &lookup)? else {
                auth::burn_password_check(&password);
                return Ok(None);
            };
            if !auth::verify_password(&password, &hash) {
                return Ok(None);
            }
            auth::get_user(conn, id)
        })
        .await?;
    let Some(user) = user else {
        state.login_throttle.record_failure(&username);
        return Err(ApiError::status(
            StatusCode::UNAUTHORIZED,
            "Incorrect username or password.",
        ));
    };
    state.login_throttle.clear(&username);
    start_session(&state, &headers, jar, user).await
}

async fn start_session(
    state: &AppState,
    headers: &HeaderMap,
    jar: CookieJar,
    user: User,
) -> ApiResult<(CookieJar, Json<User>)> {
    let token = auth::new_token()?;
    let (user_id, session_token) = (user.id, token.clone());
    state
        .db
        .run(move |conn| auth::create_session(conn, user_id, &session_token))
        .await?;
    let cookie = Cookie::build((SESSION_COOKIE, token))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(is_https(state, headers))
        // The server expires idle sessions; the cookie just needs to outlive them.
        .max_age(cookie::time::Duration::days(365))
        .build();
    Ok((jar.add(cookie), Json(user)))
}

pub async fn logout(State(state): State<AppState>, jar: CookieJar) -> ApiResult<CookieJar> {
    if let Some(token) = jar.get(SESSION_COOKIE).map(|c| c.value().to_string()) {
        state
            .db
            .run(move |conn| auth::delete_session(conn, &token))
            .await?;
    }
    Ok(jar.remove(Cookie::build(SESSION_COOKIE).path("/")))
}

pub async fn me(current: CurrentUser) -> Json<User> {
    Json(current.user)
}

#[derive(Deserialize)]
pub struct PasswordChange {
    current_password: String,
    new_password: String,
}

/// Changes the signed-in user's password and signs out their other sessions.
/// View-only accounts have no changeable options, so they can't do this themselves.
pub async fn change_password(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(body): Json<PasswordChange>,
) -> ApiResult<StatusCode> {
    current.require(Role::Editor)?;
    auth::check_password(&body.new_password).map_err(ApiError::bad_request)?;
    let (user_id, token) = (current.user.id, current.token);
    let changed = state
        .db
        .run(move |conn| {
            let hash = auth::password_hash_of(conn, user_id)?.unwrap_or_default();
            if !auth::verify_password(&body.current_password, &hash) {
                return Ok(false);
            }
            auth::set_password(conn, user_id, &body.new_password)?;
            auth::delete_other_sessions(conn, user_id, &token)?;
            Ok(true)
        })
        .await?;
    if changed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::bad_request("Current password is incorrect."))
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::json;

    use crate::api::test_support::TestApp;

    #[tokio::test]
    async fn setup_flow() {
        let app = TestApp::new();
        let status = app.send("GET", "/api/setup", None, None).await;
        assert_eq!(status.body["needs_setup"], true);

        let weak = app
            .send(
                "POST",
                "/api/setup",
                Some(json!({"username": "admin", "password": "short"})),
                None,
            )
            .await;
        assert_eq!(weak.status, StatusCode::BAD_REQUEST);

        let cookie = app.setup_admin().await;
        let me = app.send("GET", "/api/auth/me", None, Some(&cookie)).await;
        assert_eq!(me.body["role"], "admin");

        let again = app
            .send(
                "POST",
                "/api/setup",
                Some(json!({"username": "evil", "password": "long-enough"})),
                None,
            )
            .await;
        assert_eq!(again.status, StatusCode::CONFLICT);
        let status = app.send("GET", "/api/setup", None, None).await;
        assert_eq!(status.body["needs_setup"], false);
    }

    #[tokio::test]
    async fn login_and_logout() {
        let app = TestApp::new();
        app.setup_admin().await;

        let wrong = app.login("admin", "nope-nope").await;
        assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
        assert!(wrong.cookie.is_none());
        let unknown = app.login("nobody", "admin-password").await;
        assert_eq!(
            unknown.body, wrong.body,
            "unknown users look like bad passwords"
        );

        let ok = app.login("ADMIN", "admin-password").await;
        assert_eq!(ok.status, StatusCode::OK);
        let cookie = ok.cookie.unwrap();
        assert_eq!(
            app.send("GET", "/api/auth/me", None, Some(&cookie))
                .await
                .status,
            StatusCode::OK
        );

        app.send("POST", "/api/auth/logout", None, Some(&cookie))
            .await;
        assert_eq!(
            app.send("GET", "/api/auth/me", None, Some(&cookie))
                .await
                .status,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn login_is_throttled() {
        let app = TestApp::new();
        app.setup_admin().await;
        for _ in 0..10 {
            app.login("admin", "wrong-password").await;
        }
        let locked = app.login("admin", "admin-password").await;
        assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn change_own_password() {
        let app = TestApp::new();
        let admin = app.setup_admin().await;
        let other_session = app.login("admin", "admin-password").await.cookie.unwrap();

        let wrong = app
            .send(
                "POST",
                "/api/auth/password",
                Some(json!({"current_password": "bad", "new_password": "new-password"})),
                Some(&admin),
            )
            .await;
        assert_eq!(wrong.status, StatusCode::BAD_REQUEST);

        let ok = app
            .send(
                "POST",
                "/api/auth/password",
                Some(json!({"current_password": "admin-password", "new_password": "new-password"})),
                Some(&admin),
            )
            .await;
        assert_eq!(ok.status, StatusCode::NO_CONTENT);
        assert_eq!(
            app.send("GET", "/api/auth/me", None, Some(&admin))
                .await
                .status,
            StatusCode::OK,
            "the current session survives"
        );
        assert_eq!(
            app.send("GET", "/api/auth/me", None, Some(&other_session))
                .await
                .status,
            StatusCode::UNAUTHORIZED,
            "other sessions are signed out"
        );
        assert_eq!(
            app.login("admin", "new-password").await.status,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn viewers_cannot_change_their_password() {
        let app = TestApp::new();
        let admin = app.setup_admin().await;
        let viewer = app.user_session(&admin, "guest", "viewer").await;
        let response = app
            .send(
                "POST",
                "/api/auth/password",
                Some(json!({"current_password": "user-password", "new_password": "new-password"})),
                Some(&viewer),
            )
            .await;
        assert_eq!(response.status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn state_changes_require_csrf_header() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let app = TestApp::new();
        let request = Request::post("/api/setup")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"username": "admin", "password": "long-enough"}).to_string(),
            ))
            .unwrap();
        let response = app.router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
