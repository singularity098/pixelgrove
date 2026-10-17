//! Preview images, generated on demand and cached in memory only (nothing is written
//! to disk). JPEGs are decoded at reduced scale (1/2, 1/4 or 1/8) straight from the
//! DCT data, which makes thumbnails of large photos cheap.

use std::fs::File;
use std::io::BufReader;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use fast_image_resize::images::Image;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use lru::LruCache;
use tokio::sync::Semaphore;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Variant {
    /// Grid tiles: the shorter edge is at least this many pixels, so square crops
    /// stay sharp on high-DPI screens.
    Thumb,
    /// The viewer: the longer edge is at most this many pixels.
    Large,
}

impl Variant {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "thumb" => Some(Self::Thumb),
            "large" => Some(Self::Large),
            _ => None,
        }
    }

    fn target(self, width: u32, height: u32) -> (u32, u32) {
        let scale = match self {
            Variant::Thumb => THUMB_SHORT_EDGE as f64 / width.min(height) as f64,
            Variant::Large => LARGE_LONG_EDGE as f64 / width.max(height) as f64,
        };
        if scale >= 1.0 {
            return (width, height);
        }
        let w = ((width as f64 * scale).round() as u32).max(1);
        let h = ((height as f64 * scale).round() as u32).max(1);
        (w, h)
    }

    fn quality(self) -> u8 {
        match self {
            Variant::Thumb => 78,
            Variant::Large => 85,
        }
    }
}

const THUMB_SHORT_EDGE: u32 = 360;
const LARGE_LONG_EDGE: u32 = 2048;
/// Files that need no downscaling are served as-is below this size.
const SERVE_ORIGINAL_BELOW: u64 = 1_500_000;

/// What to send for a preview request.
pub enum Preview {
    Jpeg(Bytes),
    /// The original is small enough, or in a format we don't decode (AVIF, video).
    Original,
}

/// Generates previews with bounded concurrency and caches them in memory.
pub struct Previews {
    cache: Mutex<SizedLru>,
    permits: Semaphore,
}

/// Identifies a preview; the mtime and size make edits invalidate old entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub media_id: i64,
    pub variant: Variant,
    pub mtime: i64,
    pub size: i64,
}

impl Previews {
    pub fn new(cache_bytes: usize) -> Self {
        let workers = std::thread::available_parallelism().map_or(2, NonZeroUsize::get);
        Self {
            cache: Mutex::new(SizedLru::new(cache_bytes)),
            permits: Semaphore::new(workers),
        }
    }

    pub async fn get(
        self: &Arc<Self>,
        key: Key,
        path: &Path,
        orientation: Option<u16>,
    ) -> Result<Preview> {
        if let Some(bytes) = self.cache.lock().unwrap().get(&key) {
            return Ok(Preview::Jpeg(bytes));
        }
        let _permit = self.permits.acquire().await?;
        // Another request may have produced it while we waited.
        if let Some(bytes) = self.cache.lock().unwrap().get(&key) {
            return Ok(Preview::Jpeg(bytes));
        }
        let path = path.to_path_buf();
        let rendered = tokio::task::spawn_blocking(move || {
            render(&path, key.variant, orientation.unwrap_or(1))
        })
        .await??;
        Ok(match rendered {
            Some(jpeg) => {
                let bytes = Bytes::from(jpeg);
                self.cache.lock().unwrap().put(key, bytes.clone());
                Preview::Jpeg(bytes)
            }
            None => Preview::Original,
        })
    }
}

/// Renders a preview as JPEG, or `None` if the original should be served instead.
pub fn render(path: &Path, variant: Variant, orientation: u16) -> Result<Option<Vec<u8>>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let file_size = std::fs::metadata(path)?.len();
    let pixels = match ext.as_str() {
        "jpg" | "jpeg" => decode_jpeg(path, variant)?,
        "png" | "webp" | "gif" => {
            if file_size < SERVE_ORIGINAL_BELOW
                && let Ok(size) = imagesize::size(path)
                && variant.target(size.width as u32, size.height as u32)
                    == (size.width as u32, size.height as u32)
            {
                return Ok(None);
            }
            decode_other(path)?
        }
        _ => return Ok(None),
    };
    let (width, height) = variant.target(pixels.width, pixels.height);
    if (width, height) == (pixels.width, pixels.height)
        && file_size < SERVE_ORIGINAL_BELOW
        && orientation == 1
    {
        return Ok(None);
    }
    let resized = resize(pixels, width, height)?;
    let oriented = orient(resized, orientation);
    encode_jpeg(&oriented, variant.quality()).map(Some)
}

struct Pixels {
    width: u32,
    height: u32,
    /// 1 (grey) or 3 (RGB).
    channels: usize,
    data: Vec<u8>,
}

fn decode_jpeg(path: &Path, variant: Variant) -> Result<Pixels> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut decoder = jpeg_decoder::Decoder::new(BufReader::new(file));
    decoder.read_info()?;
    let info = decoder.info().context("JPEG has no image info")?;
    let (w, h) = variant.target(u32::from(info.width), u32::from(info.height));
    // Ask for at least the target size; the decoder picks the smallest DCT scale that
    // still covers it.
    decoder.scale(w as u16, h as u16)?;
    let data = decoder.decode()?;
    let info = decoder.info().context("JPEG has no image info")?;
    let (width, height) = (u32::from(info.width), u32::from(info.height));
    let (channels, data) = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => (3, data),
        jpeg_decoder::PixelFormat::L8 => (1, data),
        jpeg_decoder::PixelFormat::L16 => (1, data.chunks_exact(2).map(|p| p[0]).collect()),
        jpeg_decoder::PixelFormat::CMYK32 => (3, cmyk_to_rgb(&data)),
    };
    Ok(Pixels {
        width,
        height,
        channels,
        data,
    })
}

fn cmyk_to_rgb(cmyk: &[u8]) -> Vec<u8> {
    // jpeg-decoder returns Adobe CMYK already inverted, so each channel is (1 - ink).
    cmyk.chunks_exact(4)
        .flat_map(|p| {
            let k = u16::from(p[3]);
            [p[0], p[1], p[2]].map(|c| (u16::from(c) * k / 255) as u8)
        })
        .collect()
}

fn decode_other(path: &Path) -> Result<Pixels> {
    let image = image::ImageReader::open(path)?
        .with_guessed_format()?
        .decode()?;
    let (width, height) = (image.width(), image.height());
    // Flatten transparency onto white, the usual backdrop for screenshots and logos.
    let rgba = image.to_rgba8();
    let mut data = Vec::with_capacity(width as usize * height as usize * 3);
    for p in rgba.pixels() {
        let a = u16::from(p[3]);
        for c in &p.0[..3] {
            data.push(((u16::from(*c) * a + 255 * (255 - a)) / 255) as u8);
        }
    }
    Ok(Pixels {
        width,
        height,
        channels: 3,
        data,
    })
}

fn resize(src: Pixels, width: u32, height: u32) -> Result<Pixels> {
    if (width, height) == (src.width, src.height) {
        return Ok(src);
    }
    let pixel_type = if src.channels == 3 {
        PixelType::U8x3
    } else {
        PixelType::U8
    };
    let src_image = Image::from_vec_u8(src.width, src.height, src.data, pixel_type)?;
    let mut dst_image = Image::new(width, height, pixel_type);
    Resizer::new().resize(
        &src_image,
        &mut dst_image,
        &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::CatmullRom)),
    )?;
    Ok(Pixels {
        width,
        height,
        channels: src.channels,
        data: dst_image.into_vec(),
    })
}

/// Applies an EXIF orientation (1–8) so previews display upright without metadata.
fn orient(src: Pixels, orientation: u16) -> Pixels {
    if !(2..=8).contains(&orientation) {
        return src;
    }
    let (w, h, c) = (src.width as usize, src.height as usize, src.channels);
    let transposed = orientation >= 5;
    let (out_w, out_h) = if transposed { (h, w) } else { (w, h) };
    let mut data = vec![0u8; src.data.len()];
    for y in 0..h {
        for x in 0..w {
            // Destination coordinates for source pixel (x, y), per the EXIF definitions.
            let (dx, dy) = match orientation {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (h - 1 - y, x),
                7 => (h - 1 - y, w - 1 - x),
                _ => (y, w - 1 - x), // 8
            };
            let s = (y * w + x) * c;
            let d = (dy * out_w + dx) * c;
            data[d..d + c].copy_from_slice(&src.data[s..s + c]);
        }
    }
    Pixels {
        width: out_w as u32,
        height: out_h as u32,
        channels: c,
        data,
    }
}

fn encode_jpeg(pixels: &Pixels, quality: u8) -> Result<Vec<u8>> {
    let (Ok(width), Ok(height)) = (u16::try_from(pixels.width), u16::try_from(pixels.height))
    else {
        bail!("preview too large to encode");
    };
    let color = if pixels.channels == 3 {
        jpeg_encoder::ColorType::Rgb
    } else {
        jpeg_encoder::ColorType::Luma
    };
    let mut out = Vec::with_capacity(pixels.data.len() / 8);
    let mut encoder = jpeg_encoder::Encoder::new(&mut out, quality);
    encoder.set_progressive(true);
    encoder.encode(&pixels.data, width, height, color)?;
    Ok(out)
}

/// An LRU cache bounded by total bytes.
struct SizedLru {
    entries: LruCache<Key, Bytes>,
    bytes: usize,
    capacity: usize,
}

impl SizedLru {
    fn new(capacity: usize) -> Self {
        Self {
            entries: LruCache::unbounded(),
            bytes: 0,
            capacity,
        }
    }

    fn get(&mut self, key: &Key) -> Option<Bytes> {
        self.entries.get(key).cloned()
    }

    fn put(&mut self, key: Key, value: Bytes) {
        if value.len() > self.capacity {
            return;
        }
        self.bytes += value.len();
        if let Some(old) = self.entries.put(key, value) {
            self.bytes -= old.len();
        }
        while self.bytes > self.capacity {
            match self.entries.pop_lru() {
                Some((_, evicted)) => self.bytes -= evicted.len(),
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real (decodable) JPEG with a gradient, `w`×`h`.
    fn gradient_jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut data = Vec::with_capacity(w as usize * h as usize * 3);
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, 128]);
            }
        }
        let mut out = Vec::new();
        jpeg_encoder::Encoder::new(&mut out, 90)
            .encode(&data, w, h, jpeg_encoder::ColorType::Rgb)
            .unwrap();
        out
    }

    fn dims(jpeg: &[u8]) -> (usize, usize) {
        let size = imagesize::blob_size(jpeg).unwrap();
        (size.width, size.height)
    }

    #[test]
    fn jpeg_thumb_and_large() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.jpg");
        std::fs::write(&path, gradient_jpeg(4000, 3000)).unwrap();

        let thumb = render(&path, Variant::Thumb, 1).unwrap().unwrap();
        assert_eq!(dims(&thumb), (480, 360));
        let large = render(&path, Variant::Large, 1).unwrap().unwrap();
        assert_eq!(dims(&large), (2048, 1536));
    }

    #[test]
    fn orientation_rotates_preview() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rotated.jpg");
        std::fs::write(&path, gradient_jpeg(4000, 3000)).unwrap();
        let thumb = render(&path, Variant::Thumb, 6).unwrap().unwrap();
        assert_eq!(dims(&thumb), (360, 480));
    }

    #[test]
    fn small_files_are_served_as_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("small.jpg");
        std::fs::write(&path, gradient_jpeg(300, 200)).unwrap();
        assert!(render(&path, Variant::Thumb, 1).unwrap().is_none());
        let avif = dir.path().join("x.avif");
        std::fs::write(&avif, b"whatever").unwrap();
        assert!(render(&avif, Variant::Thumb, 1).unwrap().is_none());
    }

    #[test]
    fn png_is_decoded_and_scaled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.png");
        let img = image::RgbaImage::from_pixel(1600, 1200, image::Rgba([10, 200, 30, 128]));
        img.save(&path).unwrap();
        let thumb = render(&path, Variant::Thumb, 1).unwrap().unwrap();
        assert_eq!(dims(&thumb), (480, 360));
    }

    #[test]
    fn orient_matches_exif_semantics() {
        // 2×1 image: left pixel 1, right pixel 2 (grey).
        let src = || Pixels {
            width: 2,
            height: 1,
            channels: 1,
            data: vec![1, 2],
        };
        assert_eq!(orient(src(), 2).data, [2, 1]); // mirrored
        let rotated = orient(src(), 6); // 90° clockwise: becomes 1×2, top = left pixel
        assert_eq!(
            (rotated.width, rotated.height, rotated.data),
            (1, 2, vec![1, 2])
        );
        let rotated = orient(src(), 8); // 90° counter-clockwise: top = right pixel
        assert_eq!(rotated.data, [2, 1]);
    }

    #[test]
    fn cache_evicts_by_size() {
        let mut cache = SizedLru::new(10);
        let key = |id| Key {
            media_id: id,
            variant: Variant::Thumb,
            mtime: 0,
            size: 0,
        };
        cache.put(key(1), Bytes::from(vec![0; 6]));
        cache.put(key(2), Bytes::from(vec![0; 6]));
        assert!(cache.get(&key(1)).is_none());
        assert!(cache.get(&key(2)).is_some());
        assert_eq!(cache.bytes, 6);
        cache.put(key(3), Bytes::from(vec![0; 11]));
        assert!(
            cache.get(&key(3)).is_none(),
            "oversized entries are not cached"
        );
    }
}
