mod alloc;
mod assets;
mod config;
mod db;
mod indexer;
mod metadata;
mod scanner;
mod server;
#[cfg(test)]
mod testdata;

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
                .unwrap_or_else(|_| EnvFilter::new("info,tower_http=warn")),
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
    let indexer = Indexer::new(db.clone(), &config)?;
    indexer.start();

    let app = server::router(AppState {
        config: Arc::new(config),
        db,
        indexer,
    });
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
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
