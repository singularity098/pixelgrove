//! Explore (browse by place, camera, year and type) and the map. No machine learning:
//! everything comes from file metadata.

use axum::Json;
use axum::extract::{Query, State};
use rusqlite::{Connection, params_from_iter};
use serde::{Deserialize, Serialize};

use super::browse::{Cover, MEDIA_ITEM_COLUMNS, MediaItem, Paged, cover, scope};
use super::{ApiResult, CurrentUser};
use crate::auth::User;
use crate::server::AppState;

/// How many entries each Explore section shows.
const SECTION_SIZE: i64 = 24;

#[derive(Serialize)]
pub struct Group {
    /// Filters selecting this group's items (passed back to `/explore/items`).
    filter: Filter,
    label: String,
    detail: Option<String>,
    count: i64,
    cover: Option<Cover>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Filter {
    #[serde(skip_serializing_if = "Option::is_none")]
    country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    place: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    make: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    year: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
}

#[derive(Serialize)]
pub struct Explore {
    places: Vec<Group>,
    countries: Vec<Group>,
    cameras: Vec<Group>,
    years: Vec<Group>,
    kinds: Vec<Group>,
}

/// Runs a grouped query. `select` names the grouping columns (as `g0, g1`); SQLite
/// returns the cover columns from the row holding `max(m.taken_at)`.
fn groups(
    conn: &Connection,
    user: &User,
    select: &str,
    condition: &str,
    group_by: &str,
    order: &str,
    to_group: impl Fn(&rusqlite::Row) -> rusqlite::Result<(Filter, String, Option<String>)>,
) -> anyhow::Result<Vec<Group>> {
    let (where_scope, mut params) = scope(user, "f.path");
    params.push(SECTION_SIZE.to_string());
    let sql = format!(
        "SELECT {select}, count(*) AS n, max(m.taken_at), m.id, m.kind, m.mtime
         FROM media m JOIN folders f ON f.id = m.folder_id
         WHERE {where_scope} AND {condition}
         GROUP BY {group_by} ORDER BY {order} LIMIT CAST(? AS INTEGER)"
    );
    let mut stmt = conn.prepare(&sql)?;
    let columns = stmt.column_count();
    let rows = stmt
        .query_map(params_from_iter(&params), |r| {
            let (filter, label, detail) = to_group(r)?;
            Ok(Group {
                filter,
                label,
                detail,
                count: r.get(columns - 5)?,
                cover: cover(r, columns - 3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

pub async fn explore(
    State(state): State<AppState>,
    current: CurrentUser,
) -> ApiResult<Json<Explore>> {
    let user = current.user;
    let explore = state
        .db
        .run(move |conn| {
            let places = groups(
                conn,
                &user,
                "m.place AS g0, m.country AS g1",
                "m.place IS NOT NULL",
                "g0, g1",
                "n DESC",
                |r| {
                    let (place, country): (String, String) = (r.get(0)?, r.get(1)?);
                    Ok((
                        Filter {
                            place: Some(place.clone()),
                            country: Some(country.clone()),
                            ..Default::default()
                        },
                        place,
                        Some(country),
                    ))
                },
            )?;
            let countries = groups(
                conn,
                &user,
                "m.country AS g0",
                "m.country IS NOT NULL",
                "g0",
                "n DESC",
                |r| {
                    let country: String = r.get(0)?;
                    Ok((
                        Filter {
                            country: Some(country.clone()),
                            ..Default::default()
                        },
                        country,
                        None,
                    ))
                },
            )?;
            let cameras = groups(
                conn,
                &user,
                "coalesce(m.camera_make, '') AS g0, coalesce(m.camera_model, '') AS g1",
                "(m.camera_make IS NOT NULL OR m.camera_model IS NOT NULL)",
                "g0, g1",
                "n DESC",
                |r| {
                    let (make, model): (String, String) = (r.get(0)?, r.get(1)?);
                    let label = camera_label(&make, &model);
                    Ok((
                        Filter {
                            make: Some(make),
                            model: Some(model),
                            ..Default::default()
                        },
                        label,
                        None,
                    ))
                },
            )?;
            let years = groups(
                conn,
                &user,
                "CAST(strftime('%Y', m.taken_at, 'unixepoch') AS INTEGER) AS g0",
                "1",
                "g0",
                "g0 DESC",
                |r| {
                    let year: i32 = r.get(0)?;
                    Ok((
                        Filter {
                            year: Some(year),
                            ..Default::default()
                        },
                        year.to_string(),
                        None,
                    ))
                },
            )?;
            let kinds = groups(conn, &user, "m.kind AS g0", "1", "g0", "g0", |r| {
                let kind: String = r.get(0)?;
                let label = if kind == "video" { "Videos" } else { "Photos" };
                Ok((
                    Filter {
                        kind: Some(kind),
                        ..Default::default()
                    },
                    label.to_string(),
                    None,
                ))
            })?;
            Ok(Explore {
                places,
                countries,
                cameras,
                years,
                kinds,
            })
        })
        .await?;
    Ok(Json(explore))
}

/// "Apple iPhone 15" rather than "Apple Apple iPhone 15" (many models repeat the make).
fn camera_label(make: &str, model: &str) -> String {
    if model.to_lowercase().starts_with(&make.to_lowercase()) || make.is_empty() {
        model.to_string()
    } else if model.is_empty() {
        make.to_string()
    } else {
        format!("{make} {model}")
    }
}

/// The [`Filter`] fields, listed out: query-string parsing can't handle numbers inside
/// a `#[serde(flatten)]`ed struct.
#[derive(Deserialize)]
pub struct ItemsQuery {
    country: Option<String>,
    place: Option<String>,
    make: Option<String>,
    model: Option<String>,
    year: Option<i32>,
    kind: Option<String>,
    #[serde(default)]
    offset: i64,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    200
}

/// Items matching every given filter, newest first.
pub async fn items(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<ItemsQuery>,
) -> ApiResult<Json<Paged<MediaItem>>> {
    let user = current.user;
    let offset = query.offset.max(0);
    let limit = query.limit.clamp(1, 500);
    let page = state
        .db
        .run(move |conn| {
            let (where_scope, mut params) = scope(&user, "f.path");
            let mut conditions = vec![where_scope];
            let f = &query;
            let mut add = |sql: &str, value: Option<String>| {
                if let Some(value) = value {
                    conditions.push(sql.to_string());
                    params.push(value);
                }
            };
            add("m.country = ?", f.country.clone());
            add("m.place = ?", f.place.clone());
            add("coalesce(m.camera_make, '') = ?", f.make.clone());
            add("coalesce(m.camera_model, '') = ?", f.model.clone());
            add(
                "strftime('%Y', m.taken_at, 'unixepoch') = ?",
                f.year.map(|y| format!("{y:04}")),
            );
            add("m.kind = ?", f.kind.clone());
            let from = format!(
                "FROM media m JOIN folders f ON f.id = m.folder_id WHERE {}",
                conditions.join(" AND ")
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
                    "SELECT {MEDIA_ITEM_COLUMNS} {from} ORDER BY m.taken_at DESC, m.id DESC
                     LIMIT CAST(? AS INTEGER) OFFSET CAST(? AS INTEGER)"
                ))?
                .query_map(params_from_iter(&params), MediaItem::from_row)?
                .collect::<rusqlite::Result<_>>()?;
            Ok(Paged { items, total })
        })
        .await?;
    Ok(Json(page))
}

/// Every geotagged item as `[id, lat, lon, taken_at, v, is_video]`, compact because a
/// large library can have tens of thousands of points.
pub async fn map_points(
    State(state): State<AppState>,
    current: CurrentUser,
) -> ApiResult<Json<Vec<(i64, f64, f64, i64, i64, u8)>>> {
    let user = current.user;
    let points = state
        .db
        .run(move |conn| {
            let (where_scope, params) = scope(&user, "f.path");
            Ok(conn
                .prepare(&format!(
                    "SELECT m.id, round(m.latitude, 5), round(m.longitude, 5), m.taken_at, m.mtime,
                            m.kind = 'video'
                     FROM media m JOIN folders f ON f.id = m.folder_id
                     WHERE m.latitude IS NOT NULL AND m.longitude IS NOT NULL AND {where_scope}"
                ))?
                .query_map(params_from_iter(&params), |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                })?
                .collect::<rusqlite::Result<_>>()?)
        })
        .await?;
    Ok(Json(points))
}

#[derive(Deserialize)]
pub struct Ids {
    /// Comma-separated media ids.
    ids: String,
}

/// Media items by id (e.g. the photos in a map cluster), newest first. Ids the user may
/// not see are skipped.
pub async fn batch(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<Ids>,
) -> ApiResult<Json<Vec<MediaItem>>> {
    let user = current.user;
    let ids: Vec<i64> = query
        .ids
        .split(',')
        .filter_map(|id| id.trim().parse().ok())
        .take(500)
        .collect();
    if ids.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let items = state
        .db
        .run(move |conn| {
            let (where_scope, scope_params) = scope(&user, "f.path");
            let placeholders = vec!["CAST(? AS INTEGER)"; ids.len()].join(", ");
            let mut params: Vec<String> = ids.iter().map(i64::to_string).collect();
            params.extend(scope_params);
            Ok(conn
                .prepare(&format!(
                    "SELECT {MEDIA_ITEM_COLUMNS} FROM media m JOIN folders f ON f.id = m.folder_id
                     WHERE m.id IN ({placeholders}) AND {where_scope}
                     ORDER BY m.taken_at DESC, m.id DESC"
                ))?
                .query_map(params_from_iter(&params), MediaItem::from_row)?
                .collect::<rusqlite::Result<_>>()?)
        })
        .await?;
    Ok(Json(items))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::Value;

    use super::camera_label;
    use crate::api::test_support::TestApp;
    use crate::testdata;

    fn library(app: &TestApp) {
        app.write_media(
            "japan/kyoto1.jpg",
            &testdata::jpeg_at_location("2022:04:05 10:00:00", 34.9949, 135.785),
        );
        app.write_media(
            "japan/kyoto2.jpg",
            &testdata::jpeg_at_location("2022:04:06 10:00:00", 35.0116, 135.7681),
        );
        app.write_media(
            "france/paris.jpg",
            &testdata::jpeg_at_location("2023:07:01 10:00:00", 48.8584, 2.2945),
        );
        app.write_media(
            "home/nogps.jpg",
            &testdata::jpeg_taken_at("2024:01:01 10:00:00"),
        );
        app.scan();
    }

    fn labels(groups: &Value) -> Vec<&str> {
        groups
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["label"].as_str().unwrap())
            .collect()
    }

    #[tokio::test]
    async fn explore_groups_and_items() {
        let app = TestApp::new();
        library(&app);
        let admin = app.setup_admin().await;
        let explore = app.send("GET", "/api/explore", None, Some(&admin)).await;
        assert_eq!(explore.status, StatusCode::OK);
        assert_eq!(labels(&explore.body["places"]), ["Kyoto", "Paris"]);
        assert_eq!(explore.body["places"][0]["count"], 2);
        assert_eq!(explore.body["places"][0]["detail"], "Japan");
        assert_eq!(labels(&explore.body["countries"]), ["Japan", "France"]);
        assert_eq!(labels(&explore.body["years"]), ["2024", "2023", "2022"]);
        assert_eq!(labels(&explore.body["cameras"]), ["PixelCam Model 1"]);
        assert_eq!(explore.body["cameras"][0]["count"], 4);
        assert!(explore.body["places"][0]["cover"]["id"].is_i64());

        let kyoto = app
            .send(
                "GET",
                "/api/explore/items?place=Kyoto&country=Japan",
                None,
                Some(&admin),
            )
            .await;
        assert_eq!(kyoto.body["total"], 2);
        assert_eq!(kyoto.body["items"][0]["name"], "kyoto2.jpg");
        let year = app
            .send("GET", "/api/explore/items?year=2023", None, Some(&admin))
            .await;
        assert_eq!(year.body["items"][0]["name"], "paris.jpg");

        // Places are searchable too.
        let search = app
            .send("GET", "/api/search?q=kyoto", None, Some(&admin))
            .await;
        assert_eq!(search.body["media"]["total"], 2);
        let search = app
            .send("GET", "/api/search?q=france", None, Some(&admin))
            .await;
        assert_eq!(search.body["media"]["items"][0]["name"], "paris.jpg");
    }

    #[tokio::test]
    async fn map_points_and_batch() {
        let app = TestApp::new();
        library(&app);
        let admin = app.setup_admin().await;
        let points = app.send("GET", "/api/map/points", None, Some(&admin)).await;
        let points = points.body.as_array().unwrap();
        assert_eq!(points.len(), 3, "photos without GPS aren't on the map");
        assert!(
            (points
                .iter()
                .find(|p| p[1].as_f64().unwrap() > 40.0)
                .unwrap()[2]
                .as_f64()
                .unwrap()
                - 2.2945)
                .abs()
                < 0.001
        );

        let ids: Vec<String> = points.iter().map(|p| p[0].to_string()).collect();
        let batch = app
            .send(
                "GET",
                &format!("/api/media/batch?ids={},99999", ids.join(",")),
                None,
                Some(&admin),
            )
            .await;
        assert_eq!(batch.body.as_array().unwrap().len(), 3);
        assert_eq!(batch.body[0]["name"], "paris.jpg", "newest first");
    }

    #[tokio::test]
    async fn explore_respects_folder_limits() {
        let app = TestApp::new();
        library(&app);
        let admin = app.setup_admin().await;
        let japan = format!("{}/japan", app.media_root().display());
        app.send(
            "POST",
            "/api/users",
            Some(serde_json::json!({"username": "j", "password": "japan-pass", "role": "viewer", "folders": [japan]})),
            Some(&admin),
        )
        .await;
        let j = app.login("j", "japan-pass").await.cookie.unwrap();
        let explore = app.send("GET", "/api/explore", None, Some(&j)).await;
        assert_eq!(labels(&explore.body["countries"]), ["Japan"]);
        let points = app.send("GET", "/api/map/points", None, Some(&j)).await;
        assert_eq!(points.body.as_array().unwrap().len(), 2);
        let paris = app.media_id("paris.jpg");
        let batch = app
            .send(
                "GET",
                &format!("/api/media/batch?ids={paris}"),
                None,
                Some(&j),
            )
            .await;
        assert_eq!(batch.body.as_array().unwrap().len(), 0);
    }

    #[test]
    fn camera_labels() {
        assert_eq!(camera_label("Apple", "Apple iPhone 15"), "Apple iPhone 15");
        assert_eq!(camera_label("SONY", "ILCE-7M3"), "SONY ILCE-7M3");
        assert_eq!(camera_label("", "X100V"), "X100V");
        assert_eq!(camera_label("Canon", ""), "Canon");
    }
}
