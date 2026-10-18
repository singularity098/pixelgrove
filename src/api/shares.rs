//! Managing share links (editors and admins). Editors see their own links; admins see
//! everyone's.

use std::path::Path;

use axum::Json;
use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Deserializer, Serialize};

use super::browse::{
    Cover, cover, not_in_hidden_folders, relative, subtree, summarize, visible_folder,
};
use super::{ApiError, ApiResult, CurrentUser};
use crate::auth::{Role, User};
use crate::server::AppState;
use crate::shares::{self, Changes, NewShare, Share};

const MAX_EXPIRY_DAYS: i64 = 3650;

#[derive(Serialize)]
pub struct ShareView {
    id: i64,
    token: String,
    /// Absolute when a public URL is configured, otherwise a path for the browser to
    /// complete with its own origin.
    url: String,
    kind: &'static str,
    /// Item or folder name, and its path relative to the media root.
    name: String,
    path: String,
    cover: Option<Cover>,
    /// The folder's id, for linking to it in the library.
    folder_id: Option<i64>,
    media_id: Option<i64>,
    created_at: i64,
    created_by: String,
    expires_at: Option<i64>,
    has_password: bool,
    allow_download: bool,
    views: i64,
    last_viewed_at: Option<i64>,
}

pub fn share_url(state: &AppState, token: &str) -> String {
    let base = state
        .config
        .public_url
        .as_deref()
        .map(|u| u.trim_end_matches('/'))
        .unwrap_or("");
    format!("{base}/s/{token}")
}

fn view(conn: &Connection, state: &AppState, share: Share) -> anyhow::Result<ShareView> {
    let created_by: String = conn
        .query_row(
            "SELECT username FROM users WHERE id = ?1",
            [share.created_by],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_default();
    let (kind, name, path, cover, folder_id) = if let Some(media_id) = share.media_id {
        let row = conn
            .query_row(
                "SELECT m.name, f.path, r.path, m.id, m.kind, m.mtime, f.id FROM media m
                 JOIN folders f ON f.id = m.folder_id JOIN roots r ON r.id = f.root_id
                 WHERE m.id = ?1",
                [media_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        relative(&r.get::<_, String>(1)?, &r.get::<_, String>(2)?),
                        cover(r, 3)?,
                        r.get::<_, i64>(6)?,
                    ))
                },
            )
            .optional()?;
        let (name, folder, cover, folder_id) = row.unwrap_or_default();
        let path = if folder.is_empty() {
            name.clone()
        } else {
            format!("{folder}/{name}")
        };
        ("media", name, path, cover, Some(folder_id))
    } else {
        let folder_path = share.folder_path.clone().unwrap_or_default();
        // The cover visitors see: nothing hidden for the link.
        let hidden = crate::auth::Hidden::for_share(conn, Some(share.created_by))?;
        let (below, mut params) = subtree("f.path", &folder_path);
        let (visible, visible_params) = not_in_hidden_folders(&hidden, "f.path");
        params.extend(visible_params);
        let (_, _, cover) = summarize(conn, &format!("{below} AND {visible}"), &params, &hidden)?;
        let row: Option<(i64, String)> = conn
            .query_row(
                "SELECT f.id, r.path FROM folders f JOIN roots r ON r.id = f.root_id WHERE f.path = ?1",
                [&folder_path],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let name = Path::new(&folder_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let path = row.as_ref().map_or_else(
            || folder_path.clone(),
            |(_, root)| relative(&folder_path, root),
        );
        ("folder", name, path, cover, row.map(|(id, _)| id))
    };
    Ok(ShareView {
        url: share_url(state, &share.token),
        id: share.id,
        token: share.token,
        kind,
        name,
        path,
        cover,
        folder_id,
        media_id: share.media_id,
        created_at: share.created_at,
        created_by,
        expires_at: share.expires_at,
        has_password: share.password_hash.is_some(),
        allow_download: share.allow_download,
        views: share.views,
        last_viewed_at: share.last_viewed_at,
    })
}

pub async fn list(
    State(state): State<AppState>,
    current: CurrentUser,
) -> ApiResult<Json<Vec<ShareView>>> {
    current.require(Role::Editor)?;
    let owner = (current.user.role != Role::Admin).then_some(current.user.id);
    let state2 = state.clone();
    let views = state
        .db
        .run(move |conn| {
            shares::list(conn, owner)?
                .into_iter()
                .map(|share| view(conn, &state2, share))
                .collect::<anyhow::Result<Vec<_>>>()
        })
        .await?;
    Ok(Json(views))
}

#[derive(Deserialize)]
pub struct CreateShare {
    media_id: Option<i64>,
    folder_id: Option<i64>,
    /// Days until the link stops working; omitted or null for never.
    expires_in_days: Option<i64>,
    password: Option<String>,
    #[serde(default = "yes")]
    allow_download: bool,
}

fn yes() -> bool {
    true
}

fn expiry(days: Option<i64>) -> ApiResult<Option<i64>> {
    match days {
        None => Ok(None),
        Some(days) if (1..=MAX_EXPIRY_DAYS).contains(&days) => {
            Ok(Some(crate::auth::now() + days * 86_400))
        }
        Some(_) => Err(ApiError::bad_request(
            "Choose an expiry between 1 and 3650 days.",
        )),
    }
}

fn clean_password(password: Option<String>) -> ApiResult<Option<String>> {
    match password.filter(|p| !p.is_empty()) {
        Some(p) if p.chars().count() < 4 => Err(ApiError::bad_request(
            "Use a password of at least 4 characters.",
        )),
        other => Ok(other),
    }
}

pub async fn create(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(body): Json<CreateShare>,
) -> ApiResult<(StatusCode, Json<ShareView>)> {
    current.require(Role::Editor)?;
    let expires_at = expiry(body.expires_in_days)?;
    let password = clean_password(body.password)?;
    let user = current.user;
    let state2 = state.clone();
    let view = state
        .db
        .run(move |conn| {
            Ok(create_share(
                conn,
                &state2,
                &user,
                body.media_id,
                body.folder_id,
                expires_at,
                password.as_deref(),
                body.allow_download,
            ))
        })
        .await??;
    Ok((StatusCode::CREATED, Json(view)))
}

#[allow(clippy::too_many_arguments)]
fn create_share(
    conn: &Connection,
    state: &AppState,
    user: &User,
    media_id: Option<i64>,
    folder_id: Option<i64>,
    expires_at: Option<i64>,
    password: Option<&str>,
    allow_download: bool,
) -> ApiResult<ShareView> {
    let not_found = || ApiError::status(StatusCode::NOT_FOUND, "Not found.");
    let (media_id, folder_path) = match (media_id, folder_id) {
        (Some(id), None) => {
            let media = super::media::load(conn, id)?.ok_or_else(not_found)?;
            if !user.can_access(Path::new(&media.folder_path)) {
                return Err(not_found());
            }
            (Some(id), None)
        }
        (None, Some(id)) => (None, Some(visible_folder(conn, user, id)?.path)),
        _ => {
            return Err(ApiError::bad_request(
                "Share either one item or one folder.",
            ));
        }
    };
    let share = shares::create(
        conn,
        &NewShare {
            media_id,
            folder_path: folder_path.as_deref(),
            created_by: user.id,
            expires_at,
            password,
            allow_download,
        },
    )?;
    Ok(view(conn, state, share)?)
}

/// Distinguishes a missing field (leave as is) from `null` (clear it).
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<Option<T>>, D::Error> {
    Ok(Some(Option::deserialize(d)?))
}

#[derive(Deserialize)]
pub struct UpdateShare {
    #[serde(default, deserialize_with = "present")]
    expires_in_days: Option<Option<i64>>,
    /// A new password, or null / "" to remove it.
    #[serde(default, deserialize_with = "present")]
    password: Option<Option<String>>,
    allow_download: Option<bool>,
}

async fn owned_share(state: &AppState, user: &User, id: i64) -> ApiResult<Share> {
    let share = state
        .db
        .run(move |conn| shares::get(conn, id))
        .await?
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND, "No such link."))?;
    if share.created_by != user.id && user.role != Role::Admin {
        return Err(ApiError::status(StatusCode::NOT_FOUND, "No such link."));
    }
    Ok(share)
}

pub async fn update(
    State(state): State<AppState>,
    current: CurrentUser,
    UrlPath(id): UrlPath<i64>,
    Json(body): Json<UpdateShare>,
) -> ApiResult<Json<ShareView>> {
    current.require(Role::Editor)?;
    owned_share(&state, &current.user, id).await?;
    let expires_at = body.expires_in_days.map(expiry).transpose()?;
    let password = body.password.map(clean_password).transpose()?;
    let state2 = state.clone();
    let view = state
        .db
        .run(move |conn| {
            shares::update(
                conn,
                id,
                &Changes {
                    expires_at,
                    password: password.as_ref().map(|p| p.as_deref()),
                    allow_download: body.allow_download,
                },
            )?;
            let share = shares::get(conn, id)?.expect("exists");
            view(conn, &state2, share)
        })
        .await?;
    Ok(Json(view))
}

pub async fn delete(
    State(state): State<AppState>,
    current: CurrentUser,
    UrlPath(id): UrlPath<i64>,
) -> ApiResult<StatusCode> {
    current.require(Role::Editor)?;
    owned_share(&state, &current.user, id).await?;
    state.db.run(move |conn| shares::delete(conn, id)).await?;
    Ok(StatusCode::NO_CONTENT)
}
