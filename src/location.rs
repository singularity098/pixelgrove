//! Removing location (GPS) data from files served through share links.
//!
//! Files on disk are never changed. Instead the byte ranges holding a location are
//! worked out from the file's metadata and blanked as the bytes are sent. Every patch
//! keeps its length, so sizes, offsets and range requests (video seeking) still work.
//!
//! Covered: EXIF GPS in JPEG (including secondary images such as HDR gain maps), PNG,
//! WebP and AVIF; XMP packets that mention GPS; and the location atoms phones write in
//! MP4/MOV videos (`©xyz`, `loci`, and QuickTime `com.apple.quicktime.location.*`).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use bytes::{Bytes, BytesMut};
use lru::LruCache;

/// Replace `bytes.len()` bytes at `offset` with `bytes`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    pub offset: u64,
    pub bytes: Vec<u8>,
}

/// Image files are read whole to find their metadata; anything bigger isn't a photo.
const MAX_IMAGE_BYTES: u64 = 512 * 1024 * 1024;
/// A video's `moov` atom (its index and metadata) is read whole; even hours of video
/// stay far below this.
const MAX_MOOV_BYTES: u64 = 256 * 1024 * 1024;
/// Sanity limit for directory entries, so corrupt files can't make us loop for long.
const MAX_IFD_ENTRIES: usize = 1000;

/// The patches that remove location data from a file, sorted by offset.
pub fn find(path: &Path, kind: &str) -> Result<Vec<Patch>> {
    let mut file = File::open(path)?;
    let mut patches = if kind == "video" {
        video_patches(&mut file)?
    } else {
        let len = file.metadata()?.len();
        if len > MAX_IMAGE_BYTES {
            anyhow::bail!("image too large to check for location data");
        }
        let mut data = Vec::with_capacity(len as usize);
        file.read_to_end(&mut data)?;
        image_patches(&data)?
    };
    patches.sort_by_key(|p| p.offset);
    Ok(patches)
}

/// Caches patches per file version, so each range request of a video doesn't re-read
/// its metadata.
pub struct Cache {
    entries: Mutex<LruCache<VersionKey, Arc<Vec<Patch>>>>,
}

/// Media id, mtime and size.
type VersionKey = (i64, i64, i64);

impl Default for Cache {
    fn default() -> Self {
        Self {
            entries: Mutex::new(LruCache::new(NonZeroUsize::new(2000).unwrap())),
        }
    }
}

impl Cache {
    /// Patches for media `id` at `path`; `mtime` and `size` identify the version.
    pub async fn get(
        &self,
        id: i64,
        mtime: i64,
        size: i64,
        path: &Path,
        kind: &str,
    ) -> Result<Arc<Vec<Patch>>> {
        let key = (id, mtime, size);
        if let Some(patches) = self.entries.lock().unwrap().get(&key) {
            return Ok(patches.clone());
        }
        let (path, kind) = (path.to_path_buf(), kind.to_string());
        let patches = Arc::new(tokio::task::spawn_blocking(move || find(&path, &kind)).await??);
        self.entries.lock().unwrap().put(key, patches.clone());
        Ok(patches)
    }
}

/// Lays `patches` over `chunk`, which starts at byte `offset` of the file.
pub fn apply(patches: &[Patch], offset: u64, chunk: Bytes) -> Bytes {
    let end = offset + chunk.len() as u64;
    let mut out: Option<BytesMut> = None;
    for patch in patches {
        let patch_end = patch.offset + patch.bytes.len() as u64;
        if patch_end <= offset {
            continue;
        }
        if patch.offset >= end {
            break;
        }
        let buf = out.get_or_insert_with(|| BytesMut::from(&chunk[..]));
        let from = patch.offset.max(offset);
        let to = patch_end.min(end);
        buf[(from - offset) as usize..(to - offset) as usize].copy_from_slice(
            &patch.bytes[(from - patch.offset) as usize..(to - patch.offset) as usize],
        );
    }
    out.map_or(chunk, BytesMut::freeze)
}

// ---- Images ----

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

fn image_patches(data: &[u8]) -> Result<Vec<Patch>> {
    let mut patches = Vec::new();
    if data.starts_with(PNG_SIGNATURE) {
        png_patches(data, &mut patches);
        return Ok(patches);
    }
    // JPEG XL keeps EXIF in `Exif` boxes, without the "Exif" prefix, and may compress
    // EXIF and XMP. A compressed box can't be edited in place, so one that holds a
    // location is turned into a `free` box (same length), which readers skip.
    use crate::jxl::MetadataBox;
    for found in crate::jxl::metadata_boxes(data)? {
        let has_location = |check: &dyn Fn(&mut Vec<Patch>)| {
            let mut inner = Vec::new();
            check(&mut inner);
            !inner.is_empty()
        };
        let type_at = match found {
            MetadataBox::Exif { tiff } => {
                tiff_patches(data, tiff, &mut patches);
                continue;
            }
            MetadataBox::CompressedExif { type_at, tiff }
                if has_location(&|p| tiff_patches(&tiff, 0, p)) =>
            {
                type_at
            }
            MetadataBox::CompressedXmp { type_at, xml }
                if has_location(&|p| xmp_patches(&xml, 0, p)) =>
            {
                type_at
            }
            _ => continue,
        };
        patches.push(Patch {
            offset: type_at as u64,
            bytes: b"free".to_vec(),
        });
    }
    // EXIF blocks wherever they are: JPEG APP1 segments (also inside embedded secondary
    // images), AVIF/HEIF Exif items, WebP EXIF chunks with the usual prefix.
    for pos in memchr::memmem::find_iter(data, b"Exif\0\0") {
        tiff_patches(data, pos + 6, &mut patches);
    }
    // WebP EXIF chunks may also hold the TIFF data without the prefix.
    if data.len() > 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        let mut pos = 12;
        while pos + 8 <= data.len() {
            let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
            if &data[pos..pos + 4] == b"EXIF" && !data[pos + 8..].starts_with(b"Exif\0\0") {
                tiff_patches(data, pos + 8, &mut patches);
            }
            pos += 8 + size + (size & 1);
        }
    }
    xmp_patches(data, 0, &mut patches);
    Ok(patches)
}

/// Blanks the GPS directory of the TIFF/EXIF structure starting at `start`.
fn tiff_patches(data: &[u8], start: usize, patches: &mut Vec<Patch>) {
    let Some(tiff) = data.get(start..) else {
        return;
    };
    let little = match tiff.get(0..4) {
        Some(b"II*\0") => true,
        Some(b"MM\0*") => false,
        _ => return,
    };
    let u16_at = |o: usize| -> Option<usize> {
        let b: [u8; 2] = tiff.get(o..o + 2)?.try_into().ok()?;
        Some(if little {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        } as usize)
    };
    let u32_at = |o: usize| -> Option<usize> {
        let b: [u8; 4] = tiff.get(o..o + 4)?.try_into().ok()?;
        Some(if little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        } as usize)
    };
    let Some(ifd0) = u32_at(4) else {
        return;
    };
    let Some(count) = u16_at(ifd0).filter(|&n| n <= MAX_IFD_ENTRIES) else {
        return;
    };
    let gps = (0..count).find_map(|i| {
        let entry = ifd0 + 2 + 12 * i;
        (u16_at(entry)? == 0x8825)
            .then(|| u32_at(entry + 8))
            .flatten()
    });
    let Some(gps) = gps else {
        return;
    };
    let Some(entries) = u16_at(gps).filter(|&n| n > 0 && n <= MAX_IFD_ENTRIES) else {
        return;
    };
    let table = 2 + 12 * entries;
    if gps + table > tiff.len() {
        return;
    }
    // Values stored outside the directory (coordinates are rationals, so most are).
    for i in 0..entries {
        let entry = gps + 2 + 12 * i;
        let (Some(typ), Some(n)) = (u16_at(entry + 2), u32_at(entry + 4)) else {
            continue;
        };
        let unit = match typ {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 | 13 => 4,
            5 | 10 | 12 => 8,
            _ => continue,
        };
        let len = unit * n;
        if len > 4
            && let Some(at) = u32_at(entry + 8)
            && at + len <= tiff.len()
        {
            patches.push(Patch {
                offset: (start + at) as u64,
                bytes: vec![0; len],
            });
        }
    }
    // The directory itself: zero entries, then a zero "next directory" pointer.
    patches.push(Patch {
        offset: (start + gps) as u64,
        bytes: vec![0; table],
    });
}

/// Replaces XMP packets that mention a location with spaces (XMP's own padding, so the
/// file stays valid). `base` is where `data` starts in the file.
fn xmp_patches(data: &[u8], base: usize, patches: &mut Vec<Patch>) {
    const OPEN: &[u8] = b"<x:xmpmeta";
    const CLOSE: &[u8] = b"</x:xmpmeta>";
    let mut from = 0;
    while let Some(start) = memchr::memmem::find(&data[from..], OPEN).map(|i| from + i) {
        let Some(end) =
            memchr::memmem::find(&data[start..], CLOSE).map(|i| start + i + CLOSE.len())
        else {
            break;
        };
        let packet = &data[start..end];
        if memchr::memmem::find(packet, b"GPS").is_some()
            || memchr::memmem::find(packet, b"Location").is_some()
        {
            patches.push(Patch {
                offset: (base + start) as u64,
                bytes: vec![b' '; end - start],
            });
        }
        from = end;
    }
}

fn png_patches(data: &[u8], patches: &mut Vec<Patch>) {
    let mut pos = PNG_SIGNATURE.len();
    while pos + 12 <= data.len() {
        let len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let typ = &data[pos + 4..pos + 8];
        let body = pos + 8;
        if body + len + 4 > data.len() {
            break;
        }
        let chunk = &data[body..body + len];
        let mut local = Vec::new();
        match typ {
            b"eXIf" => tiff_patches(data, body, &mut local),
            b"iTXt" | b"tEXt" | b"zTXt" => {
                let keyword = chunk.split(|&b| b == 0).next().unwrap_or_default();
                let compressed = match typ {
                    b"zTXt" => true,
                    b"iTXt" => chunk.get(keyword.len() + 1) == Some(&1),
                    _ => false,
                };
                if keyword == b"XML:com.adobe.xmp" && !compressed {
                    xmp_patches(chunk, body, &mut local);
                } else if keyword == b"XML:com.adobe.xmp"
                    || keyword.starts_with(b"Raw profile type")
                {
                    // Compressed XMP, or EXIF stored as hex text (ImageMagick): drop the
                    // whole chunk by turning it into a private chunk viewers ignore.
                    local.push(Patch {
                        offset: (pos + 4) as u64,
                        bytes: b"pgRm".to_vec(),
                    });
                    local.push(Patch {
                        offset: body as u64,
                        bytes: vec![0; len],
                    });
                }
            }
            b"IEND" => break,
            _ => {}
        }
        // A corrupt chunk could point outside itself; leave such files alone.
        local.retain(|p| {
            p.offset as usize >= pos + 4 && p.offset as usize + p.bytes.len() <= body + len
        });
        if !local.is_empty() {
            // Recompute the chunk's CRC over its patched type and data.
            let mut patched = data[pos + 4..body + len].to_vec();
            for patch in &local {
                let at = patch.offset as usize - (pos + 4);
                patched[at..at + patch.bytes.len()].copy_from_slice(&patch.bytes);
            }
            patches.append(&mut local);
            patches.push(Patch {
                offset: (body + len) as u64,
                bytes: crc32fast::hash(&patched).to_be_bytes().to_vec(),
            });
        }
        pos = body + len + 4;
    }
}

// ---- Videos (MP4 / QuickTime) ----

fn video_patches(file: &mut File) -> Result<Vec<Patch>> {
    let file_len = file.metadata()?.len();
    let mut pos = 0u64;
    let mut patches = Vec::new();
    // Find `moov` among the top-level atoms (it may come after the media data).
    while pos + 8 <= file_len {
        let mut header = [0u8; 16];
        file.seek(SeekFrom::Start(pos))?;
        let n = file.read(&mut header)?;
        if n < 8 {
            break;
        }
        let (size, header_len) = match u32::from_be_bytes(header[0..4].try_into().unwrap()) {
            0 => (file_len - pos, 8),
            1 if n >= 16 => (u64::from_be_bytes(header[8..16].try_into().unwrap()), 16),
            s => (u64::from(s), 8),
        };
        if size < header_len || pos + size > file_len {
            break;
        }
        if &header[4..8] == b"moov" {
            if size > MAX_MOOV_BYTES {
                anyhow::bail!("video index too large to check for location data");
            }
            let mut moov = vec![0u8; size as usize];
            file.seek(SeekFrom::Start(pos))?;
            file.read_exact(&mut moov)?;
            atoms(&moov, header_len as usize, moov.len(), pos, &mut patches);
        }
        pos += size;
    }
    Ok(patches)
}

/// The atoms in `buf[start..end]` as `(offset, header length, size, type)`.
fn children(buf: &[u8], start: usize, end: usize) -> Vec<(usize, usize, usize, [u8; 4])> {
    let mut out = Vec::new();
    let mut pos = start;
    while pos + 8 <= end {
        let (size, header) = match u32::from_be_bytes(buf[pos..pos + 4].try_into().unwrap()) {
            0 => (end - pos, 8),
            1 if pos + 16 <= end => (
                u64::from_be_bytes(buf[pos + 8..pos + 16].try_into().unwrap()) as usize,
                16,
            ),
            s => (s as usize, 8),
        };
        if size < header || pos + size > end {
            break;
        }
        out.push((pos, header, size, buf[pos + 4..pos + 8].try_into().unwrap()));
        pos += size;
    }
    out
}

const XYZ: [u8; 4] = [0xA9, b'x', b'y', b'z'];

fn atoms(buf: &[u8], start: usize, end: usize, base: u64, patches: &mut Vec<Patch>) {
    for (pos, header, size, typ) in children(buf, start, end) {
        match &typ {
            b"moov" | b"trak" | b"udta" | b"mdia" | b"minf" => {
                atoms(buf, pos + header, pos + size, base, patches)
            }
            b"meta" => meta(buf, pos + header, pos + size, base, patches),
            t if *t == XYZ || t == b"loci" => {
                // Turn the atom into padding that every reader skips.
                patches.push(Patch {
                    offset: base + pos as u64 + 4,
                    bytes: b"free".to_vec(),
                });
                patches.push(Patch {
                    offset: base + (pos + header) as u64,
                    bytes: vec![0; size - header],
                });
            }
            _ => {}
        }
    }
}

/// A `meta` atom: ISO (with version/flags) or QuickTime (without); either may hold
/// `keys` + `ilst` (QuickTime metadata) or an iTunes-style `ilst`.
fn meta(buf: &[u8], start: usize, end: usize, base: u64, patches: &mut Vec<Patch>) {
    let plausible = |at: usize| {
        children(buf, at, end)
            .first()
            .is_some_and(|(_, _, _, t)| t.iter().all(|b| b.is_ascii_alphanumeric() || *b == 0xA9))
    };
    let start = if !plausible(start) && plausible(start + 4) {
        start + 4
    } else {
        start
    };
    let kids = children(buf, start, end);
    // QuickTime keys are numbered from 1; ilst items are named by that number.
    let mut location_keys = Vec::new();
    for &(pos, header, size, typ) in &kids {
        if &typ != b"keys" || size < header + 8 {
            continue;
        }
        let mut entry = pos + header + 8; // version/flags, entry count
        let mut index = 1u32;
        while entry + 8 <= pos + size {
            let len = u32::from_be_bytes(buf[entry..entry + 4].try_into().unwrap()) as usize;
            if len < 8 || entry + len > pos + size {
                break;
            }
            let name = &buf[entry + 8..entry + len];
            if name
                .to_ascii_lowercase()
                .windows(8)
                .any(|w| w == b"location")
            {
                location_keys.push(index.to_be_bytes());
            }
            entry += len;
            index += 1;
        }
    }
    for &(pos, header, size, typ) in &kids {
        if &typ != b"ilst" {
            continue;
        }
        for (item, item_header, item_size, item_type) in children(buf, pos + header, pos + size) {
            if item_type != XYZ && !location_keys.contains(&item_type) {
                continue;
            }
            // Blank each `data` value (after its 4-byte type and 4-byte locale).
            for (data, data_header, data_size, data_type) in
                children(buf, item + item_header, item + item_size)
            {
                let value = data + data_header + 8;
                if &data_type == b"data" && value < data + data_size {
                    patches.push(Patch {
                        offset: base + value as u64,
                        bytes: vec![0; data + data_size - value],
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{self, MediaKind};
    use crate::testdata;

    fn patched(data: &[u8], patches: &[Patch]) -> Vec<u8> {
        apply(patches, 0, Bytes::copy_from_slice(data)).to_vec()
    }

    #[test]
    fn jpeg_xl_location_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        // Compressed EXIF (as cjxl writes it): the box is disabled.
        let original = include_bytes!("../tests/fixtures/transcoded.jxl");
        assert!(
            info(dir.path(), "a.jxl", original, MediaKind::Image)
                .latitude
                .is_some()
        );
        let stripped = patched(original, &image_patches(original).unwrap());
        let after = info(dir.path(), "b.jxl", &stripped, MediaKind::Image);
        assert_eq!((after.latitude, after.camera_make), (None, None));
        assert_eq!(after.width, Some(48), "still a valid image");
        let path = dir.path().join("b.jxl");
        assert!(
            crate::preview::render(&path, crate::preview::Variant::Thumb, 1)
                .unwrap()
                .is_some()
        );

        // An uncompressed Exif box: just the GPS directory goes.
        let mut container = b"\0\0\0\x0cJXL \r\n\x87\n".to_vec();
        let tiff = testdata::exif_tiff("2023:06:15 14:30:00", Some((35.0, 139.0)));
        container.extend_from_slice(&((tiff.len() + 12) as u32).to_be_bytes());
        container.extend_from_slice(b"Exif\0\0\0\0");
        container.extend_from_slice(&tiff);
        let codestream = include_bytes!("../tests/fixtures/alpha.jxl");
        container.extend_from_slice(&((codestream.len() + 8) as u32).to_be_bytes());
        container.extend_from_slice(b"jxlc");
        container.extend_from_slice(codestream);
        let before = info(dir.path(), "c.jxl", &container, MediaKind::Image);
        assert!(before.latitude.is_some() && before.taken_at.is_some());
        let stripped = patched(&container, &image_patches(&container).unwrap());
        let after = info(dir.path(), "d.jxl", &stripped, MediaKind::Image);
        assert_eq!(after.latitude, None);
        assert_eq!(after.taken_at, before.taken_at, "the rest is kept");
        assert_eq!(after.width, Some(40));
    }

    fn info(dir: &Path, name: &str, bytes: &[u8], kind: MediaKind) -> metadata::MediaInfo {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        metadata::read(&mut nom_exif::MediaParser::new(), &path, kind)
    }

    #[test]
    fn jpeg_gps_is_removed_and_the_rest_kept() {
        let dir = tempfile::tempdir().unwrap();
        let original = testdata::jpeg_at_location("2022:04:05 10:00:00", 34.9949, 135.785);
        assert!(
            info(dir.path(), "a.jpg", &original, MediaKind::Image)
                .latitude
                .is_some()
        );

        let patches = image_patches(&original).unwrap();
        assert!(!patches.is_empty());
        let stripped = patched(&original, &patches);
        assert_eq!(stripped.len(), original.len());
        let after = info(dir.path(), "b.jpg", &stripped, MediaKind::Image);
        assert_eq!((after.latitude, after.longitude), (None, None));
        assert_eq!(
            after.camera_make.as_deref(),
            Some("PixelCam"),
            "other EXIF stays"
        );
        assert!(after.taken_at.is_some());
    }

    #[test]
    fn files_without_location_are_untouched() {
        assert!(
            image_patches(&testdata::jpeg_with_exif())
                .unwrap()
                .is_empty()
        );
        assert!(image_patches(testdata::PNG_3X2).unwrap().is_empty());
        assert!(image_patches(b"not an image").unwrap().is_empty());
    }

    #[test]
    fn xmp_with_location_is_blanked() {
        let mut data = b"\xff\xd8\xff\xe1\x00\x00http://ns.adobe.com/xap/1.0/\0".to_vec();
        data.extend_from_slice(
            br#"<?xpacket begin=""?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:Description exif:GPSLatitude="35,0.6N"/></x:xmpmeta><?xpacket end="w"?>"#,
        );
        let stripped = patched(&data, &image_patches(&data).unwrap());
        let text = String::from_utf8_lossy(&stripped);
        assert!(!text.contains("GPS") && !text.contains("35,0.6N"), "{text}");
        assert!(text.contains(r#"<?xpacket end="w"?>"#));
        assert_eq!(stripped.len(), data.len());
    }

    fn png_chunk(typ: &[u8], body: &[u8]) -> Vec<u8> {
        let mut out = (body.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(typ);
        out.extend_from_slice(body);
        let mut crc_input = typ.to_vec();
        crc_input.extend_from_slice(body);
        out.extend_from_slice(&crc32fast::hash(&crc_input).to_be_bytes());
        out
    }

    #[test]
    fn png_exif_chunk_is_stripped_with_valid_crc() {
        // Reuse the TIFF block from a test JPEG as a PNG eXIf chunk.
        let jpeg = testdata::jpeg_at_location("2022:04:05 10:00:00", -22.9, -43.2);
        let at = memchr::memmem::find(&jpeg, b"Exif\0\0").unwrap() + 6;
        let len = u16::from_be_bytes([jpeg[4], jpeg[5]]) as usize - 8;
        let tiff = &jpeg[at..at + len];
        let mut png = testdata::PNG_3X2[..33].to_vec(); // signature + IHDR
        png.extend(png_chunk(b"eXIf", tiff));
        png.extend(png_chunk(b"IEND", b""));

        let patches = image_patches(&png).unwrap();
        let stripped = patched(&png, &patches);
        let chunk = 33;
        let body = &stripped[chunk + 8..chunk + 8 + tiff.len()];
        let crc = &stripped[chunk + 8 + tiff.len()..chunk + 12 + tiff.len()];
        let mut crc_input = b"eXIf".to_vec();
        crc_input.extend_from_slice(body);
        assert_eq!(crc, crc32fast::hash(&crc_input).to_be_bytes());
        assert_ne!(body, tiff, "GPS directory blanked");
        assert!(
            image_patches(&stripped).unwrap().is_empty(),
            "nothing left to strip"
        );
    }

    fn atom(typ: &[u8], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(typ);
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn video_location_atoms_are_removed() {
        let iso = b"+35.0116+135.7681+050.000/";
        let mut xyz_body = (iso.len() as u16).to_be_bytes().to_vec();
        xyz_body.extend_from_slice(&[0x15, 0xC7]);
        xyz_body.extend_from_slice(iso);
        let udta = atom(b"udta", &atom(&XYZ, &xyz_body));

        // QuickTime metadata: key 1 is the make, key 2 the location.
        let mut keys_body = vec![0, 0, 0, 0, 0, 0, 0, 2];
        keys_body.extend(atom(b"mdta", b"com.apple.quicktime.make"));
        keys_body.extend(atom(b"mdta", b"com.apple.quicktime.location.ISO6709"));
        let value = |text: &[u8]| {
            let mut data = vec![0, 0, 0, 1, 0, 0, 0, 0];
            data.extend_from_slice(text);
            atom(b"data", &data)
        };
        let mut ilst_body = atom(&1u32.to_be_bytes(), &value(b"Apple"));
        ilst_body.extend(atom(&2u32.to_be_bytes(), &value(iso)));
        let mut meta_body = atom(b"hdlr", &[0; 25]);
        meta_body.extend(atom(b"keys", &keys_body));
        meta_body.extend(atom(b"ilst", &ilst_body));
        let mut moov_body = atom(b"mvhd", &[0; 100]);
        moov_body.extend(udta);
        moov_body.extend(atom(b"meta", &meta_body));

        let mut file = atom(b"ftyp", b"qt  \0\0\0\0qt  ");
        file.extend(atom(b"mdat", &[7; 64]));
        file.extend(atom(b"moov", &moov_body));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.mov");
        std::fs::write(&path, &file).unwrap();
        let patches = find(&path, "video").unwrap();
        let stripped = patched(&file, &patches);
        assert_eq!(stripped.len(), file.len());
        assert!(memchr::memmem::find(&stripped, b"+35.0116").is_none());
        assert!(
            memchr::memmem::find(&stripped, b"Apple").is_some(),
            "other metadata stays"
        );
        assert!(memchr::memmem::find(&stripped, &XYZ).is_none());
        assert_eq!(
            &stripped[..file.len() - moov_body.len() - 8],
            &file[..file.len() - moov_body.len() - 8],
            "media data untouched"
        );
    }

    #[test]
    fn apply_handles_chunk_boundaries() {
        let patches = vec![
            Patch {
                offset: 2,
                bytes: b"AB".to_vec(),
            },
            Patch {
                offset: 6,
                bytes: b"CDEF".to_vec(),
            },
        ];
        let data = b"0123456789";
        let mut out = Vec::new();
        for (i, chunk) in data.chunks(3).enumerate() {
            out.extend_from_slice(&apply(
                &patches,
                (i * 3) as u64,
                Bytes::copy_from_slice(chunk),
            ));
        }
        assert_eq!(out, b"01AB45CDEF");
        // A range starting mid-patch.
        assert_eq!(&apply(&patches, 7, Bytes::from_static(b"789"))[..], b"DEF");
    }
}
