//! Every item in the library in one stream, newest first, plus per-month counts for
//! the date scrubber.

use axum::Json;
use axum::extract::{Query, State};
use rusqlite::params_from_iter;
use serde::{Deserialize, Serialize};

use super::browse::{MEDIA_ITEM_COLUMNS, MediaItem, scope};
use super::{ApiResult, CurrentUser};
use crate::server::AppState;

#[derive(Deserialize)]
pub struct TimelineQuery {
    /// Keyset cursor: return items strictly older than (before, before_id). Without
    /// `before_id`, everything taken before `before` (used to jump to a month).
    before: Option<i64>,
    before_id: Option<i64>,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    200
}

#[derive(Serialize)]
pub struct TimelinePage {
    items: Vec<MediaItem>,
    /// Whether there are older items.
    more: bool,
}

pub async fn timeline(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<TimelineQuery>,
) -> ApiResult<Json<TimelinePage>> {
    let user = current.user;
    let limit = query.limit.clamp(1, 500);
    let page = state
        .db
        .run(move |conn| {
            let (where_scope, mut params) = scope(&user, "f.path");
            let mut conditions = vec![where_scope];
            match (query.before, query.before_id) {
                (Some(before), Some(id)) => {
                    conditions.push("(m.taken_at < ? OR (m.taken_at = ? AND m.id < ?))".into());
                    params.extend([before.to_string(), before.to_string(), id.to_string()]);
                }
                (Some(before), None) => {
                    conditions.push("m.taken_at < ?".into());
                    params.push(before.to_string());
                }
                _ => {}
            }
            // Fetch one extra row to learn whether more remain.
            params.push((limit + 1).to_string());
            let mut items: Vec<MediaItem> = conn
                .prepare(&format!(
                    "SELECT {MEDIA_ITEM_COLUMNS} FROM media m
                     JOIN folders f ON f.id = m.folder_id
                     WHERE {}
                     ORDER BY m.taken_at DESC, m.id DESC
                     LIMIT CAST(? AS INTEGER)",
                    conditions.join(" AND ")
                ))?
                .query_map(params_from_iter(&params), MediaItem::from_row)?
                .collect::<rusqlite::Result<_>>()?;
            let more = items.len() as i64 > limit;
            items.truncate(limit as usize);
            Ok(TimelinePage { items, more })
        })
        .await?;
    Ok(Json(page))
}

#[derive(Serialize)]
pub struct Month {
    /// `YYYY-MM`.
    month: String,
    count: i64,
}

pub async fn months(
    State(state): State<AppState>,
    current: CurrentUser,
) -> ApiResult<Json<Vec<Month>>> {
    let user = current.user;
    let months = state
        .db
        .run(move |conn| {
            let (where_scope, params) = scope(&user, "f.path");
            Ok(conn
                .prepare(&format!(
                    "SELECT strftime('%Y-%m', m.taken_at, 'unixepoch') AS month, count(*)
                     FROM media m JOIN folders f ON f.id = m.folder_id
                     WHERE {where_scope}
                     GROUP BY month ORDER BY month DESC"
                ))?
                .query_map(params_from_iter(&params), |r| {
                    Ok(Month {
                        month: r.get(0)?,
                        count: r.get(1)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?)
        })
        .await?;
    Ok(Json(months))
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use crate::api::test_support::TestApp;
    use crate::testdata;

    fn names(items: &Value) -> Vec<&str> {
        items
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["name"].as_str().unwrap())
            .collect()
    }

    #[tokio::test]
    async fn pages_newest_first_across_folders() {
        let app = TestApp::new();
        for (path, date) in [
            ("a/1.jpg", "2024:03:01 10:00:00"),
            ("b/2.jpg", "2024:03:02 10:00:00"),
            ("a/3.jpg", "2024:01:15 10:00:00"),
            ("c/4.jpg", "2023:12:31 10:00:00"),
        ] {
            app.write_media(path, &testdata::jpeg_taken_at(date));
        }
        app.scan();
        let admin = app.setup_admin().await;

        let first = app
            .send("GET", "/api/timeline?limit=2", None, Some(&admin))
            .await;
        assert_eq!(names(&first.body["items"]), ["2.jpg", "1.jpg"]);
        assert_eq!(first.body["more"], true);
        let last = &first.body["items"][1];
        let next = app
            .send(
                "GET",
                &format!(
                    "/api/timeline?limit=2&before={}&before_id={}",
                    last["taken_at"], last["id"]
                ),
                None,
                Some(&admin),
            )
            .await;
        assert_eq!(names(&next.body["items"]), ["3.jpg", "4.jpg"]);
        assert_eq!(next.body["more"], false);

        // Jump to January 2024 (everything before Feb 1st).
        let jump = app
            .send("GET", "/api/timeline?before=1706745600", None, Some(&admin))
            .await;
        assert_eq!(names(&jump.body["items"]), ["3.jpg", "4.jpg"]);

        let months = app
            .send("GET", "/api/timeline/months", None, Some(&admin))
            .await;
        assert_eq!(
            months.body,
            serde_json::json!([
                {"month": "2024-03", "count": 2},
                {"month": "2024-01", "count": 1},
                {"month": "2023-12", "count": 1},
            ])
        );
    }
}
