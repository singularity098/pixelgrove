//! Video thumbnails: one frame per video, extracted by `ffmpeg` (when it's installed)
//! and kept in memory with the photo previews. Nothing is written to disk.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use tokio::io::AsyncReadExt;
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

    /// The first real video and audio streams in `file` (cover art doesn't count), read
    /// from ffmpeg's description of its input: it only reads the file's header.
    pub async fn probe(&self, file: &Path) -> Result<Streams> {
        let mut cmd = Command::new(&self.path);
        cmd.args(["-nostdin", "-hide_banner", "-i"])
            .arg(file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // No output is named, so ffmpeg always "fails" after describing the input.
        let out = tokio::time::timeout(TIMEOUT, cmd.output())
            .await
            .context("ffmpeg timed out")?
            .context("running ffmpeg")?;
        Ok(Streams::parse(&String::from_utf8_lossy(&out.stderr)))
    }

    /// Repackages a video browsers can decode but not open (Matroska) as fragmented MP4,
    /// streamed from `start` seconds in. The picture is copied as is; audio that isn't
    /// AAC or MP3 (AC-3, DTS, FLAC, Opus...) becomes stereo AAC. `None` when the video
    /// codec isn't one browsers play from MP4, so the original is the better bet.
    /// A copied picture can only start at a keyframe, so a stream from `start` really
    /// begins at the keyframe before it, and the sound starts there too.
    /// The process stops when the returned stream is dropped (the viewer seeks or closes).
    pub async fn remux(
        &self,
        file: &Path,
        start: f64,
    ) -> Result<Option<impl futures_util::Stream<Item = std::io::Result<Bytes>> + use<>>> {
        let streams = self.probe(file).await?;
        let Some(video) = streams.video else {
            return Ok(None);
        };
        let tag = match video.codec.as_str() {
            "h264" => "avc1",
            // Safari plays HEVC from MP4 only when it's tagged hvc1.
            "hevc" => "hvc1",
            "vp9" => "vp09",
            "av1" => "av01",
            _ => return Ok(None),
        };
        let mut cmd = Command::new(&self.path);
        cmd.args(["-nostdin", "-v", "error"]);
        if start > 0.0 {
            // Without -noaccurate_seek the sound would start at `start` exactly while the
            // copied picture starts at the keyframe before it, up to a GOP (often several
            // seconds) earlier; both would then play from 0 and the sound would run early.
            cmd.args(["-ss", &format!("{start:.3}"), "-noaccurate_seek"]);
        }
        cmd.arg("-i")
            .arg(file)
            .args(["-map", &format!("0:{}", video.index)])
            .args(["-c:v", "copy", "-tag:v", tag]);
        if let Some(audio) = &streams.audio {
            cmd.args(["-map", &format!("0:{}", audio.index)]);
            if matches!(audio.codec.as_str(), "aac" | "mp3") {
                cmd.args(["-c:a", "copy"]);
            } else {
                cmd.args(["-c:a", "aac", "-b:a", "192k", "-ac", "2"]);
            }
        }
        // Tags (location among them) and chapters stay behind: this is for watching.
        // Streams in a Matroska file needn't start together (sound delayed by a second or
        // two is common). An empty moov can't say so, and ffmpeg then starts every track
        // at 0; delay_moov writes the moov after the first fragment, with edit lists that
        // keep each track's own start.
        cmd.args(["-map_metadata", "-1", "-map_chapters", "-1"])
            .args(["-avoid_negative_ts", "make_zero"])
            .args([
                "-movflags",
                "frag_keyframe+empty_moov+default_base_moof+delay_moov",
            ])
            .args(["-f", "mp4", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd.spawn().context("running ffmpeg")?;
        let stdout = child.stdout.take().context("ffmpeg has no stdout")?;
        Ok(Some(futures_util::stream::unfold(
            (child, stdout),
            |(child, mut stdout)| async move {
                let mut buf = vec![0; 64 * 1024];
                match stdout.read(&mut buf).await {
                    Ok(0) => None,
                    Ok(n) => {
                        buf.truncate(n);
                        Some((Ok(Bytes::from(buf)), (child, stdout)))
                    }
                    Err(err) => Some((Err(err), (child, stdout))),
                }
            },
        )))
    }
}

/// What `Ffmpeg::probe` found.
#[derive(Debug, Default, PartialEq)]
pub struct Streams {
    pub video: Option<Stream>,
    pub audio: Option<Stream>,
}

#[derive(Debug, PartialEq)]
pub struct Stream {
    /// The stream's index in the file, for `-map 0:N`.
    pub index: u32,
    pub codec: String,
}

impl Streams {
    /// Reads lines like `  Stream #0:1(eng): Audio: ac3, 48000 Hz, 5.1(side), ...`.
    fn parse(description: &str) -> Self {
        let mut streams = Self::default();
        for line in description.lines() {
            let Some(rest) = line.trim_start().strip_prefix("Stream #0:") else {
                continue;
            };
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            let Ok(index) = rest[..digits].parse() else {
                continue;
            };
            let codec_of = |kind: &str| {
                let after = &rest[rest.find(kind)? + kind.len()..];
                let codec = after.split([' ', ',']).next()?;
                (!codec.is_empty()).then(|| codec.to_string())
            };
            if streams.video.is_none()
                && !line.contains("(attached pic)")
                && let Some(codec) = codec_of(": Video: ")
            {
                streams.video = Some(Stream { index, codec });
            } else if streams.audio.is_none()
                && let Some(codec) = codec_of(": Audio: ")
            {
                streams.audio = Some(Stream { index, codec });
            }
        }
        streams
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

    #[test]
    fn reads_streams_from_the_description() {
        let description = "Input #0, matroska,webm, from 'film.mkv':
  Duration: 01:42:10.05, start: 0.000000, bitrate: 9000 kb/s
  Stream #0:0: Video: mjpeg (Baseline), yuvj420p, 600x900 (attached pic)
  Stream #0:1(eng): Video: hevc (Main 10), yuv420p10le(tv), 3840x2160, 23.98 fps (default)
  Stream #0:2(eng): Audio: eac3, 48000 Hz, 5.1(side), fltp, 640 kb/s (default)
  Stream #0:3(jpn): Audio: aac (LC), 48000 Hz, stereo, fltp
  Stream #0:4(eng): Subtitle: subrip
At least one output file must be specified";
        let streams = Streams::parse(description);
        let stream = |index, codec: &str| Stream {
            index,
            codec: codec.into(),
        };
        assert_eq!(streams.video, Some(stream(1, "hevc")));
        assert_eq!(streams.audio, Some(stream(2, "eac3")));
        assert_eq!(Streams::parse("garbage"), Streams::default());
    }

    /// The test clip is VP9 with AC-3 audio; skipped where ffmpeg isn't installed.
    #[tokio::test]
    async fn remuxes_matroska_to_fragmented_mp4() {
        let Some(ffmpeg) = Ffmpeg::detect("ffmpeg").await else {
            eprintln!("ffmpeg not installed; skipping");
            return;
        };
        let clip = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/clip.mkv");
        let streams = ffmpeg.probe(&clip).await.unwrap();
        assert_eq!(streams.video.unwrap().codec, "vp9");
        assert_eq!(streams.audio.unwrap().codec, "ac3");

        for start in [0.0, 0.5] {
            let stream = ffmpeg.remux(&clip, start).await.unwrap().unwrap();
            let chunks: Vec<_> = futures_util::StreamExt::collect(stream).await;
            let mp4: Vec<u8> = chunks.into_iter().flat_map(Result::unwrap).collect();
            assert_eq!(&mp4[4..8], b"ftyp");
            assert!(mp4.windows(4).any(|w| w == b"moof"), "fragmented");
            assert!(mp4.windows(4).any(|w| w == b"mp4a"), "audio became AAC");
        }
        let missing = ffmpeg
            .remux(&clip.with_extension("nope"), 0.0)
            .await
            .unwrap();
        assert!(missing.is_none());
    }

    /// Sound that starts after the picture (common in Matroska) must keep its delay in
    /// the MP4, or it plays early. Skipped where ffmpeg isn't installed.
    #[tokio::test]
    async fn remux_keeps_delayed_audio_in_sync() {
        let Some(ffmpeg) = Ffmpeg::detect("ffmpeg").await else {
            eprintln!("ffmpeg not installed; skipping");
            return;
        };
        let clip = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/clip.mkv");
        let dir = tempfile::tempdir().unwrap();
        let delayed = dir.path().join("delayed.mkv");
        let ok = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&clip)
            .args(["-itsoffset", "0.5", "-i"])
            .arg(&clip)
            .args(["-map", "0:v", "-map", "1:a", "-c", "copy"])
            .arg(&delayed)
            .status()
            .unwrap()
            .success();
        assert!(ok);

        let stream = ffmpeg.remux(&delayed, 0.0).await.unwrap().unwrap();
        let chunks: Vec<_> = futures_util::StreamExt::collect(stream).await;
        let mp4: Vec<u8> = chunks.into_iter().flat_map(Result::unwrap).collect();
        let out = dir.path().join("out.mp4");
        std::fs::write(&out, mp4).unwrap();
        let lag = first_pts(&out, "a") - first_pts(&out, "v");
        assert!(
            (lag - 0.5).abs() < 0.05,
            "sound starts {lag:.3} s after the picture"
        );
    }

    /// When the first packet of `stream` ("v" or "a") in `file` plays, in seconds.
    fn first_pts(file: &Path, stream: &str) -> f64 {
        let out = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-copyts", "-i"])
            .arg(file)
            .args(["-map", &format!("0:{stream}"), "-c", "copy", "-frames", "1"])
            .args(["-f", "framecrc", "-"])
            .output()
            .unwrap();
        let text = String::from_utf8(out.stdout).unwrap();
        // "#tb 0: 1/48000", then packets as "0, dts, pts, duration, size, crc".
        let tb = text
            .lines()
            .find_map(|l| l.strip_prefix("#tb 0: "))
            .unwrap();
        let (num, den) = tb.split_once('/').unwrap();
        let packet = text.lines().find(|l| !l.starts_with('#')).unwrap();
        let pts: f64 = packet.split(',').nth(2).unwrap().trim().parse().unwrap();
        pts * num.parse::<f64>().unwrap() / den.parse::<f64>().unwrap()
    }

    fn image_size(jpeg: &[u8]) -> (usize, usize) {
        let size = imagesize::blob_size(jpeg).unwrap();
        (size.width, size.height)
    }
}
