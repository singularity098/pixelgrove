//! Video thumbnails: one frame per video, extracted by `ffmpeg` (when it's installed)
//! and kept in memory with the photo previews. Nothing is written to disk.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tokio::process::Command;

use crate::db::Db;
use crate::indexer::Indexer;
use crate::preview::{Key, Previews, Variant};

/// A stuck ffmpeg (an unreadable network file, say) is killed after this long.
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct Ffmpeg {
    path: PathBuf,
}

impl Ffmpeg {
    /// Checks that `path` (a path or a name on `PATH`) runs; `None` if it doesn't, in
    /// which case browsers grab video frames themselves.
    pub async fn detect(path: &str) -> Option<Self> {
        if path.is_empty() || path == "off" {
            return None;
        }
        let run = Command::new(path)
            .arg("-version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output();
        match tokio::time::timeout(Duration::from_secs(10), run).await {
            Ok(Ok(out)) if out.status.success() => {
                let version = String::from_utf8_lossy(&out.stdout);
                let version = version.lines().next().unwrap_or_default();
                tracing::info!(%version, "video thumbnails use ffmpeg");
                Some(Self {
                    path: PathBuf::from(path),
                })
            }
            _ => {
                tracing::info!(
                    ffmpeg = path,
                    "ffmpeg not found; browsers will make video thumbnails themselves"
                );
                None
            }
        }
    }

    /// A JPEG of one frame about a second in (or halfway through a shorter video),
    /// rotated upright and scaled for `variant`.
    pub async fn frame(
        &self,
        file: &Path,
        variant: Variant,
        duration_ms: Option<i64>,
    ) -> Result<Vec<u8>> {
        let at = match duration_ms {
            Some(ms) if ms > 0 => (ms as f64 / 2000.0).min(1.0),
            _ => 1.0,
        };
        // Fast: jump to the keyframe at or before `at` and decode just that frame.
        match self.run(file, variant, at, true).await {
            Ok(jpeg) if !jpeg.is_empty() => Ok(jpeg),
            // Some files have no usable keyframe there: decode from the start instead.
            _ => {
                let jpeg = self.run(file, variant, 0.0, false).await?;
                if jpeg.is_empty() {
                    bail!("ffmpeg produced no frame");
                }
                Ok(jpeg)
            }
        }
    }

    async fn run(&self, file: &Path, variant: Variant, at: f64, keyframe: bool) -> Result<Vec<u8>> {
        // ffmpeg applies the video's rotation first, so iw/ih are the upright size.
        let scale = match variant {
            Variant::Thumb => {
                "scale=w='if(gt(iw,ih),-2,min(360,iw))':h='if(gt(iw,ih),min(360,ih),-2)'"
            }
            Variant::Large => {
                "scale=w='if(gt(iw,ih),min(1280,iw),-2)':h='if(gt(iw,ih),-2,min(1280,ih))'"
            }
        };
        let mut cmd = Command::new(&self.path);
        cmd.args(["-nostdin", "-v", "error", "-threads", "1"]);
        if keyframe {
            cmd.args(["-skip_frame", "nokey", "-noaccurate_seek"]);
        }
        cmd.args(["-ss", &format!("{at:.3}"), "-i"])
            .arg(file)
            .args(["-frames:v", "1", "-an", "-sn", "-dn", "-vf"])
            .arg(format!("{scale}:flags=bilinear,format=yuvj420p"))
            .args(["-q:v", "5", "-f", "mjpeg", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let out = tokio::time::timeout(TIMEOUT, cmd.output())
            .await
            .context("ffmpeg timed out")?
            .context("running ffmpeg")?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            bail!("ffmpeg failed: {}", err.lines().next().unwrap_or_default());
        }
        Ok(out.stdout)
    }
}

/// After each full scan, makes thumbnails for the newest `count` videos (one at a
/// time, in the background), so grids of recent videos show at once. They're kept in
/// memory only, like every preview.
pub fn spawn_preload(db: Db, indexer: Indexer, previews: Arc<Previews>, count: usize) {
    tokio::spawn(async move {
        let mut done_for = None;
        loop {
            let scanned = indexer.status().last_full_scan_at;
            if scanned.is_some() && scanned != done_for {
                done_for = scanned;
                if let Err(err) = preload(&db, &previews, count).await {
                    tracing::warn!("preloading video thumbnails failed: {err:#}");
                }
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}

async fn preload(db: &Db, previews: &Arc<Previews>, count: usize) -> Result<()> {
    let videos: Vec<(i64, String, String, i64, i64, Option<i64>)> = db
        .run(move |conn| {
            Ok(conn
                .prepare(
                    "SELECT m.id, f.path, m.name, m.mtime, m.size, m.duration_ms
                     FROM media m JOIN folders f ON f.id = m.folder_id
                     WHERE m.kind = 'video' ORDER BY m.taken_at DESC LIMIT ?1",
                )?
                .query_map([count as i64], |r| {
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
    let started = Instant::now();
    let mut made = 0;
    for (id, folder, name, mtime, size, duration_ms) in videos {
        let key = Key {
            media_id: id,
            variant: Variant::Thumb,
            mtime,
            size,
        };
        if previews.is_cached(&key) {
            continue;
        }
        let path = Path::new(&folder).join(&name);
        match previews.video(key, &path, duration_ms).await {
            Ok(_) => made += 1,
            Err(err) => tracing::debug!(path = %path.display(), "video thumbnail failed: {err:#}"),
        }
    }
    if made > 0 {
        tracing::info!(
            videos = made,
            duration_ms = started.elapsed().as_millis() as u64,
            "video thumbnails ready"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs ffmpeg with an encoder for the test clip; skipped where it isn't installed.
    #[tokio::test]
    async fn extracts_an_upright_frame() {
        let Some(ffmpeg) = Ffmpeg::detect("ffmpeg").await else {
            eprintln!("ffmpeg not installed; skipping");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("clip.mp4");
        // A 640×360 clip flagged to display rotated by 90°, like a phone video.
        let made = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=3:size=640x360:rate=10",
            ])
            .args(["-c:v", "mpeg4", "-display_rotation"])
            .arg("90")
            .arg(&clip)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            // Older ffmpeg without -display_rotation: an unrotated clip still checks the rest.
            let ok = std::process::Command::new("ffmpeg")
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc=duration=3:size=640x360:rate=10",
                ])
                .args(["-c:v", "mpeg4"])
                .arg(&clip)
                .status()
                .unwrap()
                .success();
            assert!(ok);
        }
        let jpeg = ffmpeg
            .frame(&clip, Variant::Thumb, Some(3000))
            .await
            .unwrap();
        let (w, h) = image_size(&jpeg);
        assert!(w.min(h) == 360, "short edge is 360: {w}×{h}");

        let missing = ffmpeg
            .frame(&dir.path().join("nope.mp4"), Variant::Thumb, None)
            .await;
        assert!(missing.is_err());
    }

    fn image_size(jpeg: &[u8]) -> (usize, usize) {
        let size = imagesize::blob_size(jpeg).unwrap();
        (size.width, size.height)
    }
}
