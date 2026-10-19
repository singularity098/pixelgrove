mod alloc;
mod api;
mod assets;
mod auth;
mod config;
mod db;
mod indexer;
mod jxl;
mod location;
mod metadata;
mod places;
mod preview;
mod scanner;
mod server;
mod shares;
#[cfg(test)]
mod testdata;
mod trash;
mod video;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::db::Db;
use crate::indexer::Indexer;
use crate::server::AppState;

fn main() -> Result<()> {
    alloc::tune();
    tokio::runtime::Runtime::new()?.block_on(run())
}

async fn run() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        None | Some("serve") => serve().await,
        Some("healthcheck") => healthcheck().await,
        Some("--version" | "-V") => {
            println!("pixelgrove {}", server::VERSION);
            Ok(())
        }
        Some(other) => bail!("unknown command `{other}` (expected `serve` or `healthcheck`)"),
    }
}

async fn serve() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("PIXELGROVE_LOG")
                // nom_exif logs at ERROR/WARN for routine things like a photo without GPS.
                .unwrap_or_else(|_| EnvFilter::new("info,tower_http=warn,nom_exif=off")),
        )
        .init();

    let config = Config::load()?;
    std::fs::create_dir_all(&config.config_dir)
        .with_context(|| format!("creating config dir {}", config.config_dir.display()))?;

    let listener = TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("binding {}", config.bind))?;
    tracing::info!(
        version = server::VERSION,
        bind = %config.bind,
        config_dir = %config.config_dir.display(),
        media_roots = ?config.media_roots,
        "PixelGrove started"
    );

    let db = Db::open(&config.config_dir.join(db::DB_FILE_NAME))?;
    bootstrap_admin(&db, &config)?;
    match scanner::backfill_places(&db) {
        Ok(0) => {}
        Ok(n) => tracing::info!(items = n, "added place names to existing photos"),
        Err(err) => tracing::warn!("adding place names failed: {err:#}"),
    }
    let indexer = Indexer::new(db.clone(), &config)?;
    indexer.start();
    spawn_session_cleanup(db.clone());

    let ffmpeg = video::Ffmpeg::detect(&config.ffmpeg).await;
    let previews = Arc::new(
        preview::Previews::new(config.preview_cache_mb as usize * 1024 * 1024).with_ffmpeg(ffmpeg),
    );
    if previews.has_ffmpeg() && config.video_preload > 0 {
        video::spawn_preload(
            db.clone(),
            indexer.clone(),
            previews.clone(),
            config.video_preload,
        );
    }
    let app = server::router(AppState {
        previews,
        config: Arc::new(config),
        db,
        indexer,
        login_throttle: Default::default(),
        locations: Default::default(),
    });
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

/// Creates the admin from `PIXELGROVE_ADMIN_*` if there are no users yet; otherwise
/// the web UI asks for one on first visit.
fn bootstrap_admin(db: &Db, config: &Config) -> Result<()> {
    let mut conn = db.conn()?;
    if auth::user_count(&conn)? > 0 {
        return Ok(());
    }
    match &config.initial_admin {
        Some(admin) => {
            let username = auth::clean_username(&admin.username)
                .map_err(|e| anyhow::anyhow!("PIXELGROVE_ADMIN_USER: {e}"))?;
            auth::check_password(&admin.password)
                .map_err(|e| anyhow::anyhow!("PIXELGROVE_ADMIN_PASSWORD: {e}"))?;
            auth::create_first_admin(&mut conn, &username, &admin.password)?;
            tracing::info!(%username, "created admin account from environment");
        }
        None => tracing::warn!(
            "no accounts exist yet: open PixelGrove in a browser to create the admin account"
        ),
    }
    Ok(())
}

/// Hourly housekeeping: expired sessions and trash older than 30 days.
fn spawn_session_cleanup(db: Db) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        loop {
            interval.tick().await;
            match db.run(|conn| auth::purge_expired_sessions(conn)).await {
                Ok(0) => {}
                Ok(n) => tracing::debug!(sessions = n, "removed expired sessions"),
                Err(err) => tracing::warn!("removing expired sessions failed: {err:#}"),
            }
            match db.run(|conn| trash::purge_expired(conn)).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(items = n, "deleted items trashed over 30 days ago"),
                Err(err) => tracing::warn!("emptying old trash failed: {err:#}"),
            }
        }
    });
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutting down");
}

/// Exits non-zero unless the local server answers `/api/health`. Used by the Docker
/// HEALTHCHECK, since the image has no shell or curl.
async fn healthcheck() -> Result<()> {
    let config = Config::load()?;
    let mut addr = config.bind;
    if addr.ip().is_unspecified() {
        addr.set_ip(std::net::Ipv4Addr::LOCALHOST.into());
    }
    let check = async {
        let mut stream = TcpStream::connect(addr).await?;
        stream
            .write_all(b"GET /api/health HTTP/1.0\r\nHost: localhost\r\n\r\n")
            .await?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        anyhow::Ok(response)
    };
    let response = tokio::time::timeout(Duration::from_secs(5), check)
        .await
        .context("health check timed out")??;
    if response.starts_with(b"HTTP/1.0 200") || response.starts_with(b"HTTP/1.1 200") {
        Ok(())
    } else {
        bail!("health check failed")
    }
}
