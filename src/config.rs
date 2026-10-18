//! Runtime configuration.
//!
//! Values are resolved in order: built-in defaults, then `<config_dir>/pixelgrove.toml`
//! (if present), then `PIXELGROVE_*` environment variables.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

pub const CONFIG_FILE_NAME: &str = "pixelgrove.toml";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Address the HTTP server listens on.
    pub bind: SocketAddr,
    /// Directory holding the database and config file. Kept small: no thumbnails, ever.
    pub config_dir: PathBuf,
    /// Directories scanned for media.
    pub media_roots: Vec<PathBuf>,
    /// Externally reachable URL, used to build share links.
    pub public_url: Option<String>,
    /// Upper bound for the in-memory preview cache, in megabytes.
    pub preview_cache_mb: u64,
    /// The ffmpeg used for video thumbnails (a path, or a name on PATH; "off" disables).
    pub ffmpeg: String,
    /// How many of the newest videos get thumbnails made ahead of time, after each scan.
    pub video_preload: usize,
    /// Hours between full rescans of the media roots.
    pub rescan_interval_hours: u64,
    /// File/directory name globs that are never indexed (hidden entries are always skipped).
    pub exclude: Vec<String>,
    /// Admin account to create on first start, from `PIXELGROVE_ADMIN_USER` and
    /// `PIXELGROVE_ADMIN_PASSWORD` (environment only, so the password isn't kept in a file).
    pub initial_admin: Option<InitialAdmin>,
}

#[derive(Clone, PartialEq)]
pub struct InitialAdmin {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for InitialAdmin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InitialAdmin")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

impl Config {
    pub fn with_config_dir(config_dir: PathBuf) -> Self {
        Self {
            bind: SocketAddr::from(([0, 0, 0, 0], 8080)),
            config_dir,
            media_roots: vec![PathBuf::from("/media")],
            public_url: None,
            preview_cache_mb: 256,
            ffmpeg: "ffmpeg".into(),
            video_preload: 1000,
            rescan_interval_hours: 6,
            exclude: crate::scanner::DEFAULT_EXCLUDES
                .iter()
                .map(|s| s.to_string())
                .collect(),
            initial_admin: None,
        }
    }

    /// Loads configuration from the process environment.
    pub fn load() -> Result<Self> {
        Self::load_from(|key| std::env::var(key).ok())
    }

    /// Loads configuration using `env` to look up environment variables.
    pub fn load_from(env: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let config_dir = env("PIXELGROVE_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/config"));
        let mut config = Self::with_config_dir(config_dir);

        let file_path = config.config_dir.join(CONFIG_FILE_NAME);
        if file_path.is_file() {
            let file = FileConfig::read(&file_path)?;
            config.apply_file(file);
        }

        config.apply_env(env)?;
        Ok(config)
    }

    fn apply_file(&mut self, file: FileConfig) {
        if let Some(bind) = file.bind {
            self.bind = bind;
        }
        if let Some(roots) = file.media_roots {
            self.media_roots = roots;
        }
        if file.public_url.is_some() {
            self.public_url = file.public_url;
        }
        if let Some(mb) = file.preview_cache_mb {
            self.preview_cache_mb = mb;
        }
        if let Some(hours) = file.rescan_interval_hours {
            self.rescan_interval_hours = hours;
        }
        if let Some(exclude) = file.exclude {
            self.exclude = exclude;
        }
    }

    fn apply_env(&mut self, env: impl Fn(&str) -> Option<String>) -> Result<()> {
        if let Some(bind) = env("PIXELGROVE_BIND") {
            self.bind = bind
                .parse()
                .with_context(|| format!("invalid PIXELGROVE_BIND: {bind}"))?;
        }
        if let Some(roots) = env("PIXELGROVE_MEDIA_ROOTS") {
            self.media_roots = std::env::split_paths(&roots).collect();
        }
        if let Some(url) = env("PIXELGROVE_PUBLIC_URL") {
            self.public_url = Some(url).filter(|u| !u.is_empty());
        }
        if let Some(mb) = env("PIXELGROVE_PREVIEW_CACHE_MB") {
            self.preview_cache_mb = mb
                .parse()
                .with_context(|| format!("invalid PIXELGROVE_PREVIEW_CACHE_MB: {mb}"))?;
        }
        if let Some(ffmpeg) = env("PIXELGROVE_FFMPEG") {
            self.ffmpeg = ffmpeg;
        }
        if let Some(count) = env("PIXELGROVE_VIDEO_PRELOAD") {
            self.video_preload = count
                .parse()
                .with_context(|| format!("invalid PIXELGROVE_VIDEO_PRELOAD: {count}"))?;
        }
        if let Some(hours) = env("PIXELGROVE_RESCAN_INTERVAL_HOURS") {
            self.rescan_interval_hours = hours
                .parse()
                .with_context(|| format!("invalid PIXELGROVE_RESCAN_INTERVAL_HOURS: {hours}"))?;
        }
        if let Some(exclude) = env("PIXELGROVE_EXCLUDE") {
            self.exclude = exclude
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
        }
        if let (Some(username), Some(password)) = (
            env("PIXELGROVE_ADMIN_USER").filter(|u| !u.trim().is_empty()),
            env("PIXELGROVE_ADMIN_PASSWORD").filter(|p| !p.is_empty()),
        ) {
            self.initial_admin = Some(InitialAdmin { username, password });
        }
        Ok(())
    }
}

/// The on-disk config file; every field is optional.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    bind: Option<SocketAddr>,
    media_roots: Option<Vec<PathBuf>>,
    public_url: Option<String>,
    preview_cache_mb: Option<u64>,
    rescan_interval_hours: Option<u64>,
    exclude: Option<Vec<String>>,
}

impl FileConfig {
    fn read(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn defaults_without_file_or_env() {
        let dir = tempfile::tempdir().unwrap();
        let dir_str = dir.path().to_str().unwrap();
        let config = Config::load_from(env_from(&[("PIXELGROVE_CONFIG_DIR", dir_str)])).unwrap();
        assert_eq!(config, Config::with_config_dir(dir.path().to_path_buf()));
    }

    #[test]
    fn env_overrides_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(CONFIG_FILE_NAME),
            r#"
            bind = "127.0.0.1:9000"
            media_roots = ["/photos"]
            public_url = "https://file.example"
            preview_cache_mb = 64
            "#,
        )
        .unwrap();
        let dir_str = dir.path().to_str().unwrap();
        let config = Config::load_from(env_from(&[
            ("PIXELGROVE_CONFIG_DIR", dir_str),
            ("PIXELGROVE_PUBLIC_URL", "https://env.example"),
            ("PIXELGROVE_RESCAN_INTERVAL_HOURS", "12"),
            ("PIXELGROVE_ADMIN_USER", "boss"),
            ("PIXELGROVE_ADMIN_PASSWORD", "secret-password"),
        ]))
        .unwrap();

        assert_eq!(config.bind, "127.0.0.1:9000".parse().unwrap());
        assert_eq!(config.media_roots, vec![PathBuf::from("/photos")]);
        assert_eq!(config.public_url.as_deref(), Some("https://env.example"));
        assert_eq!(config.preview_cache_mb, 64);
        assert_eq!(config.rescan_interval_hours, 12);
        assert!(config.exclude.contains(&"@eaDir".to_string()));
        let admin = config.initial_admin.unwrap();
        assert_eq!(admin.username, "boss");
        assert!(!format!("{admin:?}").contains("secret-password"));
    }

    #[test]
    fn media_roots_env_is_path_list() {
        let dir = tempfile::tempdir().unwrap();
        let dir_str = dir.path().to_str().unwrap();
        let config = Config::load_from(env_from(&[
            ("PIXELGROVE_CONFIG_DIR", dir_str),
            ("PIXELGROVE_MEDIA_ROOTS", "/media/a:/data/b"),
            ("PIXELGROVE_EXCLUDE", "*.tmp, Thumbs"),
        ]))
        .unwrap();
        assert_eq!(
            config.media_roots,
            vec![PathBuf::from("/media/a"), PathBuf::from("/data/b")]
        );
        assert_eq!(config.exclude, ["*.tmp", "Thumbs"]);
        assert_eq!(config.initial_admin, None);
    }

    #[test]
    fn invalid_values_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let dir_str = dir.path().to_str().unwrap();
        assert!(
            Config::load_from(env_from(&[
                ("PIXELGROVE_CONFIG_DIR", dir_str),
                ("PIXELGROVE_BIND", "not-an-address"),
            ]))
            .is_err()
        );

        std::fs::write(dir.path().join(CONFIG_FILE_NAME), "unknown_key = 1").unwrap();
        assert!(Config::load_from(env_from(&[("PIXELGROVE_CONFIG_DIR", dir_str)])).is_err());
    }
}
