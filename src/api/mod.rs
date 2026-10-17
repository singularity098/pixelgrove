//! JSON API under `/api`.

mod auth;
mod library;
mod users;

use axum::extract::{FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use axum_extra::extract::CookieJar;
use serde::Serialize;

use crate::auth::{Role, User};
use crate::server::{AppState, VERSION};

pub const SESSION_COOKIE: &str = "pixelgrove_session";
/// Header required on every state-changing request. Browsers won't add a custom header
/// to a cross-site request without a CORS preflight, which this server never approves,
/// so requiring it blocks cross-site request forgery.
pub const CSRF_HEADER: &str = "x-pixelgrove";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/setup", get(auth::setup_status).post(auth::setup))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .route("/auth/password", post(auth::change_password))
        .route("/users", get(users::list).post(users::create))
        .route("/users/{id}", patch(users::update).delete(users::delete))
        .route("/library/status", get(library::status))
        .fallback(not_found)
        .layer(middleware::from_fn(require_csrf_header))
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: VERSION,
    })
}

async fn not_found() -> ApiError {
    ApiError::status(StatusCode::NOT_FOUND, "Not found.")
}

async fn require_csrf_header(request: Request, next: Next) -> Response {
    let safe = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    if !safe && !request.headers().contains_key(CSRF_HEADER) {
        return ApiError::status(StatusCode::FORBIDDEN, "Missing request header.").into_response();
    }
    next.run(request).await
}

/// An API error. Client errors carry a user-facing message; internal errors are
/// logged and reported generically.
pub enum ApiError {
    Status(StatusCode, String),
    Internal(anyhow::Error),
}

impl ApiError {
    pub fn status(code: StatusCode, message: impl Into<String>) -> Self {
        Self::Status(code, message.into())
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::status(StatusCode::BAD_REQUEST, message)
    }

    pub fn unauthorized() -> Self {
        Self::status(StatusCode::UNAUTHORIZED, "Please sign in.")
    }

    pub fn forbidden() -> Self {
        Self::status(
            StatusCode::FORBIDDEN,
            "You don't have permission to do that.",
        )
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(err: E) -> Self {
        Self::Internal(err.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (code, message) = match self {
            ApiError::Status(code, message) => (code, message),
            ApiError::Internal(err) => {
                tracing::error!("request failed: {err:#}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Something went wrong.".to_string(),
                )
            }
        };
        (code, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

/// The signed-in user, from the session cookie. Rejects with 401 otherwise.
pub struct CurrentUser {
    pub user: User,
    pub token: String,
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> ApiResult<Self> {
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar
            .get(SESSION_COOKIE)
            .map(|c| c.value().to_string())
            .ok_or_else(ApiError::unauthorized)?;
        let lookup = token.clone();
        let user = state
            .db
            .run(move |conn| crate::auth::session_user(conn, &lookup))
            .await?
            .ok_or_else(ApiError::unauthorized)?;
        Ok(Self { user, token })
    }
}

impl CurrentUser {
    pub fn require(&self, role: Role) -> ApiResult<()> {
        if self.user.role >= role {
            Ok(())
        } else {
            Err(ApiError::forbidden())
        }
    }
}

/// A signed-in admin. Rejects with 401 or 403 otherwise.
pub struct AdminUser(pub CurrentUser);

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> ApiResult<Self> {
        let current = CurrentUser::from_request_parts(parts, state).await?;
        current.require(Role::Admin)?;
        Ok(Self(current))
    }
}

/// Whether the client reached us over HTTPS (directly or via a reverse proxy), which
/// decides whether cookies get the `Secure` flag.
fn is_https(state: &AppState, headers: &HeaderMap) -> bool {
    state
        .config
        .public_url
        .as_deref()
        .is_some_and(|u| u.starts_with("https://"))
        || headers
            .get("x-forwarded-proto")
            .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"https"))
}

#[cfg(test)]
pub mod test_support {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use http_body_util::BodyExt;
    use serde_json::Value;
    use tower::ServiceExt;

    use crate::config::Config;
    use crate::db::Db;
    use crate::indexer::Indexer;
    use crate::server::{AppState, router};

    pub struct TestApp {
        _dir: tempfile::TempDir,
        pub router: Router,
    }

    pub struct TestResponse {
        pub status: StatusCode,
        pub body: Value,
        pub cookie: Option<String>,
    }

    impl TestApp {
        pub fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let mut config = Config::with_config_dir(dir.path().to_path_buf());
            config.media_roots = vec!["/media".into()];
            let db = Db::open(&dir.path().join("t.db")).unwrap();
            let indexer = Indexer::new(db.clone(), &config).unwrap();
            let state = AppState {
                config: Arc::new(config),
                db,
                indexer,
                login_throttle: Default::default(),
            };
            Self {
                router: router(state),
                _dir: dir,
            }
        }

        /// Sends a request; `cookie` is a `name=value` session cookie.
        pub async fn send(
            &self,
            method: &str,
            uri: &str,
            body: Option<Value>,
            cookie: Option<&str>,
        ) -> TestResponse {
            let mut request = Request::builder()
                .method(method)
                .uri(uri)
                .header(super::CSRF_HEADER, "1");
            if let Some(cookie) = cookie {
                request = request.header(header::COOKIE, cookie);
            }
            let request = match body {
                Some(body) => request
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string())),
                None => request.body(Body::empty()),
            }
            .unwrap();
            let response = self.router.clone().oneshot(request).await.unwrap();
            let status = response.status();
            let cookie = response
                .headers()
                .get(header::SET_COOKIE)
                .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string());
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let body = if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap()
            };
            TestResponse {
                status,
                body,
                cookie,
            }
        }

        /// Completes first-run setup and returns the admin's session cookie.
        pub async fn setup_admin(&self) -> String {
            let response = self
                .send(
                    "POST",
                    "/api/setup",
                    Some(serde_json::json!({"username": "admin", "password": "admin-password"})),
                    None,
                )
                .await;
            assert_eq!(response.status, StatusCode::OK, "{:?}", response.body);
            response.cookie.unwrap()
        }

        /// Creates a user as the admin and returns their session cookie.
        pub async fn user_session(&self, admin: &str, username: &str, role: &str) -> String {
            let response = self
                .send(
                    "POST",
                    "/api/users",
                    Some(serde_json::json!({
                        "username": username, "password": "user-password", "role": role
                    })),
                    Some(admin),
                )
                .await;
            assert_eq!(response.status, StatusCode::CREATED, "{:?}", response.body);
            self.login(username, "user-password").await.cookie.unwrap()
        }

        pub async fn login(&self, username: &str, password: &str) -> TestResponse {
            self.send(
                "POST",
                "/api/auth/login",
                Some(serde_json::json!({"username": username, "password": password})),
                None,
            )
            .await
        }
    }
}
