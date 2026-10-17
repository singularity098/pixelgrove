//! Media type detection and metadata extraction (EXIF for images, container
//! metadata for videos). Only file headers are read.

use std::fs::Metadata as FsMetadata;
use std::path::Path;
use std::time::UNIX_EPOCH;

use nom_exif::{EntryValue, ExifTag, MediaKind as SourceKind, MediaParser, MediaSource};
use nom_exif::{ExifDateTime, TrackInfoTag};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
}

impl MediaKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MediaKind::Image => "image",
            MediaKind::Video => "video",
        }
    }
}

/// Media types supported in v1, by lower-case file extension.
pub fn kind_for_path(path: &Path) -> Option<MediaKind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "avif" => Some(MediaKind::Image),
        "mp4" | "m4v" | "webm" | "mov" => Some(MediaKind::Video),
        _ => None,
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MediaInfo {
    /// Wall-clock capture time, seconds since the epoch.
    pub taken_at: Option<i64>,
    /// Offset from UTC in seconds, when the file records one.
    pub tz_offset: Option<i32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub orientation: Option<u16>,
    pub duration_ms: Option<u64>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub description: Option<String>,
}

/// File modification time in whole seconds since the epoch.
pub fn mtime_secs(meta: &FsMetadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

/// Reads whatever metadata the file offers. Never fails: unreadable or
/// metadata-less files simply yield fewer fields.
pub fn read(parser: &mut MediaParser, path: &Path, kind: MediaKind) -> MediaInfo {
    let mut info = MediaInfo::default();
    match kind {
        MediaKind::Image => read_image(parser, path, &mut info),
        MediaKind::Video => read_video(parser, path, &mut info),
    }
    if (info.width.is_none() || info.height.is_none())
        && kind == MediaKind::Image
        && let Ok(size) = imagesize::size(path)
    {
        info.width = Some(size.width as u32);
        info.height = Some(size.height as u32);
    }
    info
}

fn read_image(parser: &mut MediaParser, path: &Path, info: &mut MediaInfo) {
    let Ok(source) = MediaSource::open(path) else {
        return;
    };
    if source.kind() != SourceKind::Image {
        return;
    }
    let Ok(iter) = parser.parse_exif(source) else {
        return;
    };
    let exif: nom_exif::Exif = iter.into();
    let text = |tag| exif.get(tag).and_then(EntryValue::as_str).and_then(clean);
    let uint = |tag| exif.get(tag).and_then(as_u32);

    if let Some(dt) = [ExifTag::DateTimeOriginal, ExifTag::CreateDate]
        .into_iter()
        .find_map(|tag| exif.get(tag).and_then(EntryValue::as_datetime))
    {
        set_time(info, dt);
    }
    info.width = uint(ExifTag::ExifImageWidth).or_else(|| uint(ExifTag::ImageWidth));
    info.height = uint(ExifTag::ExifImageHeight).or_else(|| uint(ExifTag::ImageHeight));
    info.orientation = uint(ExifTag::Orientation).map(|o| o as u16);
    info.camera_make = text(ExifTag::Make);
    info.camera_model = text(ExifTag::Model);
    info.lens = text(ExifTag::LensModel);
    info.description = text(ExifTag::ImageDescription);
    if let Some(gps) = exif.gps_info() {
        set_position(info, gps.latitude_decimal(), gps.longitude_decimal());
    }
}

fn read_video(parser: &mut MediaParser, path: &Path, info: &mut MediaInfo) {
    let Ok(source) = MediaSource::open(path) else {
        return;
    };
    if source.kind() != SourceKind::Track {
        return;
    }
    let Ok(track) = parser.parse_track(source) else {
        return;
    };
    let text = |tag| track.get(tag).and_then(EntryValue::as_str).and_then(clean);

    if let Some(dt) = track
        .get(TrackInfoTag::CreateDate)
        .and_then(EntryValue::as_datetime)
    {
        set_time(info, dt);
    }
    info.duration_ms = track
        .get(TrackInfoTag::DurationMs)
        .and_then(EntryValue::as_u64);
    info.width = track.get(TrackInfoTag::Width).and_then(as_u32);
    info.height = track.get(TrackInfoTag::Height).and_then(as_u32);
    info.camera_make = text(TrackInfoTag::Make);
    info.camera_model = text(TrackInfoTag::Model);
    if let Some(gps) = track.gps_info() {
        set_position(info, gps.latitude_decimal(), gps.longitude_decimal());
    }
}

fn set_time(info: &mut MediaInfo, dt: ExifDateTime) {
    let offset = dt.aware().map(|a| a.offset().local_minus_utc());
    let naive = dt.into_naive();
    // Cameras write all-zero dates when the clock was never set.
    if naive.and_utc().timestamp() > 0 {
        info.taken_at = Some(naive.and_utc().timestamp());
        info.tz_offset = offset;
    }
}

fn set_position(info: &mut MediaInfo, lat: Option<f64>, lon: Option<f64>) {
    if let (Some(lat), Some(lon)) = (lat, lon)
        && lat.is_finite()
        && lon.is_finite()
        && (lat, lon) != (0.0, 0.0)
        && lat.abs() <= 90.0
        && lon.abs() <= 180.0
    {
        info.latitude = Some(lat);
        info.longitude = Some(lon);
    }
}

fn as_u32(value: &EntryValue) -> Option<u32> {
    value
        .as_u32()
        .or_else(|| value.as_u16().map(u32::from))
        .or_else(|| value.as_u64().and_then(|v| u32::try_from(v).ok()))
}

fn clean(s: &str) -> Option<String> {
    let s = s.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    (!s.is_empty()).then(|| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_extension() {
        assert_eq!(kind_for_path(Path::new("a/B.JPG")), Some(MediaKind::Image));
        assert_eq!(kind_for_path(Path::new("clip.mov")), Some(MediaKind::Video));
        assert_eq!(kind_for_path(Path::new("notes.txt")), None);
        assert_eq!(kind_for_path(Path::new("noext")), None);
    }

    #[test]
    fn png_dimensions_without_exif() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tiny.png");
        std::fs::write(&path, crate::testdata::PNG_3X2).unwrap();
        let info = read(&mut MediaParser::new(), &path, MediaKind::Image);
        assert_eq!((info.width, info.height), (Some(3), Some(2)));
        assert_eq!(info.taken_at, None);
    }

    #[test]
    fn jpeg_exif_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        std::fs::write(&path, crate::testdata::jpeg_with_exif()).unwrap();
        let info = read(&mut MediaParser::new(), &path, MediaKind::Image);
        assert_eq!(info.camera_make.as_deref(), Some("PixelCam"));
        assert_eq!(info.camera_model.as_deref(), Some("Model 1"));
        // 2023-06-15 14:30:00, interpreted as wall-clock time.
        assert_eq!(info.taken_at, Some(1_686_839_400));
    }

    #[test]
    fn jpeg_gps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kyoto.jpg");
        std::fs::write(
            &path,
            crate::testdata::jpeg_at_location("2022:04:05 10:00:00", 34.9949, 135.785),
        )
        .unwrap();
        let info = read(&mut MediaParser::new(), &path, MediaKind::Image);
        assert!((info.latitude.unwrap() - 34.9949).abs() < 0.001, "{info:?}");
        assert!((info.longitude.unwrap() - 135.785).abs() < 0.001);
        let south_west = dir.path().join("rio.jpg");
        std::fs::write(
            &south_west,
            crate::testdata::jpeg_at_location("2022:04:05 10:00:00", -22.9, -43.2),
        )
        .unwrap();
        let info = read(&mut MediaParser::new(), &south_west, MediaKind::Image);
        assert!(info.latitude.unwrap() < 0.0 && info.longitude.unwrap() < 0.0);
    }

    #[test]
    fn garbage_file_yields_empty_info() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.jpg");
        std::fs::write(&path, b"not really a jpeg").unwrap();
        let info = read(&mut MediaParser::new(), &path, MediaKind::Image);
        assert_eq!(info, MediaInfo::default());
    }
}
