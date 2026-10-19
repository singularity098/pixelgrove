//! JPEG XL: metadata from the file's boxes and header, and decoding for previews.
//!
//! A JPEG XL file is either a bare codestream or an ISO-BMFF style container whose
//! boxes hold the codestream (`jxlc`, or `jxlp` parts), EXIF (`Exif`), XMP (`xml `)
//! and, for files transcoded losslessly from JPEG, the data to rebuild that JPEG
//! (`jbrd`). Metadata boxes may also be Brotli-compressed inside `brob` boxes.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{Context, Result, bail};
use jxl_oxide::{
    EnumColourEncoding, InitializeResult, JpegReconstructionStatus, JxlImage, RenderingIntent,
};

const CONTAINER_SIGNATURE: &[u8] = b"\0\0\0\x0cJXL \r\n\x87\n";
/// EXIF and XMP boxes are small; anything bigger is not worth reading.
const MAX_METADATA_BYTES: u64 = 4 * 1024 * 1024;
/// The image header sits near the start of the codestream; give up after this much.
const MAX_HEADER_BYTES: usize = 4 * 1024 * 1024;

pub fn is_jxl(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("jxl"))
}

/// One box of a container: its type, where its body starts and how long it is
/// (`None`: to the end of the file).
struct BoxHeader {
    kind: [u8; 4],
    body: u64,
    len: Option<u64>,
}

/// Reads the box header at `pos`, or `None` at the end of the file.
fn box_at(reader: &mut (impl Read + Seek), pos: u64) -> Result<Option<BoxHeader>> {
    reader.seek(SeekFrom::Start(pos))?;
    let mut header = [0u8; 16];
    let mut got = 0;
    while got < 8 {
        let n = reader.read(&mut header[got..8])?;
        if n == 0 {
            return if got == 0 {
                Ok(None)
            } else {
                bail!("truncated box")
            };
        }
        got += n;
    }
    let size = u32::from_be_bytes(header[0..4].try_into().unwrap());
    let kind: [u8; 4] = header[4..8].try_into().unwrap();
    let (body, len) = match size {
        0 => (pos + 8, None),
        1 => {
            reader.read_exact(&mut header[8..16])?;
            let size = u64::from_be_bytes(header[8..16].try_into().unwrap());
            if size < 16 {
                bail!("bad box size");
            }
            (pos + 16, Some(size - 16))
        }
        s if s < 8 => bail!("bad box size"),
        s => (pos + 8, Some(u64::from(s) - 8)),
    };
    Ok(Some(BoxHeader { kind, body, len }))
}

/// The TIFF structure of the file's EXIF box, if it has one. Only box headers and the
/// EXIF box itself are read.
pub fn exif(path: &Path) -> Option<Vec<u8>> {
    let mut file = BufReader::new(File::open(path).ok()?);
    let mut signature = [0u8; 12];
    file.read_exact(&mut signature).ok()?;
    if signature != CONTAINER_SIGNATURE {
        return None;
    }
    let mut pos = 12;
    while let Some(b) = box_at(&mut file, pos).ok()? {
        let len = b.len?;
        let body = match &b.kind {
            b"Exif" | b"brob" if len <= MAX_METADATA_BYTES => {
                let mut body = vec![0; len as usize];
                file.read_exact(&mut body).ok()?;
                body
            }
            _ => Vec::new(),
        };
        match &b.kind {
            b"Exif" => return tiff(&body).map(<[u8]>::to_vec),
            b"brob" if body.starts_with(b"Exif") => {
                return tiff(&unbrotli(&body[4..]).ok()?).map(<[u8]>::to_vec);
            }
            _ => {}
        }
        pos = b.body + len;
    }
    None
}

/// The TIFF structure in an `Exif` box body, which starts with its offset.
fn tiff(body: &[u8]) -> Option<&[u8]> {
    let offset = u32::from_be_bytes(body.get(0..4)?.try_into().unwrap()) as usize;
    body.get(4 + offset..)
}

/// Decompresses a `brob` box's content (after its 4-byte inner type), up to
/// `MAX_METADATA_BYTES`.
fn unbrotli(compressed: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    brotli_decompressor::Decompressor::new(compressed, 4096)
        .take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut out)?;
    if out.len() as u64 > MAX_METADATA_BYTES {
        bail!("metadata box too large");
    }
    Ok(out)
}

/// The image's size, upright (after its orientation is applied), from its header.
pub fn dimensions(path: &Path) -> Option<(u32, u32)> {
    let mut file = File::open(path).ok()?;
    let mut uninit = JxlImage::builder().build_uninit();
    let mut buf = vec![0u8; 64 * 1024];
    let mut read = 0;
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        read += n;
        uninit.feed_bytes(&buf[..n]).ok()?;
        match uninit.try_init().ok()? {
            InitializeResult::Initialized(image) => return Some((image.width(), image.height())),
            InitializeResult::NeedMoreData(more) if read < MAX_HEADER_BYTES => uninit = more,
            InitializeResult::NeedMoreData(_) => return None,
        }
    }
}

pub enum Decoded {
    /// A file transcoded losslessly from JPEG: that JPEG, rebuilt (cheap to make, and
    /// the JPEG decoder can scale it down while decoding), and the EXIF-style
    /// orientation (1–8) its pixels need.
    Jpeg { jpeg: Vec<u8>, orientation: u16 },
    /// Upright RGB pixels, transparency flattened onto white.
    Rgb {
        width: u32,
        height: u32,
        data: Vec<u8>,
    },
}

/// Decodes the first frame for a preview. Memory-hungry: see `Previews`, which runs
/// one at a time.
pub fn decode(path: &Path) -> Result<Decoded> {
    let mut image = JxlImage::builder()
        .open(path)
        .map_err(|e| anyhow::anyhow!("decoding {}: {e}", path.display()))?;
    if image.jpeg_reconstruction_status() == JpegReconstructionStatus::Available {
        let mut jpeg = Vec::new();
        if image.reconstruct_jpeg(&mut jpeg).is_ok() {
            let orientation = image.image_header().metadata.orientation as u16;
            return Ok(Decoded::Jpeg { jpeg, orientation });
        }
    }
    image.request_color_encoding(EnumColourEncoding::srgb(RenderingIntent::Relative));
    let render = image
        .render_frame(0)
        .map_err(|e| anyhow::anyhow!("decoding {}: {e}", path.display()))?;
    let mut stream = render.stream();
    let (width, height, channels) = (stream.width(), stream.height(), stream.channels() as usize);
    let mut samples = vec![0u8; width as usize * height as usize * channels];
    stream.write_to_buffer(&mut samples);
    let data = match channels {
        1 => samples.iter().flat_map(|&v| [v; 3]).collect(),
        2 => samples
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|&[v, a]| [over_white(v, a); 3])
            .collect(),
        3 => samples,
        4 => samples
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|&[r, g, b, a]| [r, g, b].map(|c| over_white(c, a)))
            .collect(),
        n => bail!("unsupported JPEG XL channel layout ({n} channels)"),
    };
    Ok(Decoded::Rgb {
        width,
        height,
        data,
    })
}

fn over_white(c: u8, a: u8) -> u8 {
    ((u16::from(c) * u16::from(a) + 255 * (255 - u16::from(a))) / 255) as u8
}

/// A metadata box of a JPEG XL file held in memory, for finding location data.
pub enum MetadataBox {
    /// An uncompressed `Exif` box: where its TIFF structure starts in the file.
    Exif {
        tiff: usize,
    },
    /// A Brotli-compressed EXIF (TIFF structure) or XMP box: `type_at` is where the
    /// box's `brob` type is in the file.
    CompressedExif {
        type_at: usize,
        tiff: Vec<u8>,
    },
    CompressedXmp {
        type_at: usize,
        xml: Vec<u8>,
    },
}

/// The EXIF and XMP boxes of a JPEG XL file (none for a bare codestream or other files).
/// Uncompressed XMP needs no special handling: it's plain text in the file.
pub fn metadata_boxes(data: &[u8]) -> Result<Vec<MetadataBox>> {
    if !data.starts_with(CONTAINER_SIGNATURE) {
        return Ok(Vec::new());
    }
    let mut reader = std::io::Cursor::new(data);
    let mut found = Vec::new();
    let mut pos = 12;
    while let Some(b) = box_at(&mut reader, pos)? {
        let end = b.len.map_or(data.len() as u64, |len| b.body + len);
        if end > data.len() as u64 {
            bail!("truncated JPEG XL box");
        }
        let body = &data[b.body as usize..end as usize];
        let type_at = pos as usize + 4;
        match &b.kind {
            b"Exif" if body.len() >= 4 => {
                let offset = u32::from_be_bytes(body[0..4].try_into().unwrap()) as usize;
                found.push(MetadataBox::Exif {
                    tiff: b.body as usize + 4 + offset,
                });
            }
            b"brob" if body.starts_with(b"Exif") => {
                let exif = unbrotli(&body[4..])?;
                let tiff = tiff(&exif).context("bad Exif box")?.to_vec();
                found.push(MetadataBox::CompressedExif { type_at, tiff });
            }
            b"brob" if body.starts_with(b"xml ") => {
                let xml = unbrotli(&body[4..])?;
                found.push(MetadataBox::CompressedXmp { type_at, xml });
            }
            _ => {}
        }
        if b.len.is_none() {
            break;
        }
        pos = end;
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{self, MediaKind};

    const TRANSCODED: &[u8] = include_bytes!("../tests/fixtures/transcoded.jxl");
    const ALPHA: &[u8] = include_bytes!("../tests/fixtures/alpha.jxl");

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// A 64×48 JPEG with EXIF (camera, date, GPS, orientation 6) transcoded losslessly.
    #[test]
    fn metadata_from_exif_box_and_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "a.jxl", TRANSCODED);
        let info = metadata::read(&mut nom_exif::MediaParser::new(), &path, MediaKind::Image);
        assert_eq!(info.camera_make.as_deref(), Some("TestCam"));
        assert_eq!(info.camera_model.as_deref(), Some("JXL-1"));
        assert!(info.taken_at.is_some());
        assert!((info.latitude.unwrap() - 48.8583).abs() < 1e-3);
        assert!((info.longitude.unwrap() - 2.2944).abs() < 1e-3);
        // Upright size; the header's orientation is applied by the decoder.
        assert_eq!((info.width, info.height), (Some(48), Some(64)));
        assert_eq!(info.orientation, None);
    }

    #[test]
    fn transcoded_jpeg_preview_is_upright() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "a.jxl", TRANSCODED);
        let jpeg = crate::preview::render(&path, crate::preview::Variant::Thumb, 1)
            .unwrap()
            .expect("always converted");
        let size = imagesize::blob_size(&jpeg).unwrap();
        assert_eq!((size.width, size.height), (48, 64));
    }

    #[test]
    fn transparency_is_flattened() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "b.jxl", ALPHA);
        let Decoded::Rgb {
            width,
            height,
            data,
        } = decode(&path).unwrap()
        else {
            panic!("not a transcoded JPEG");
        };
        assert_eq!((width, height), (40, 30));
        // Left half opaque blue, right half transparent: white.
        let px = |x: usize| &data[(15 * 40 + x) * 3..][..3];
        assert!(px(5)[2] > 200 && px(5)[0] < 60, "{:?}", px(5));
        assert!(px(35).iter().all(|&c| c > 230), "{:?}", px(35));
        assert_eq!(
            metadata::read(&mut nom_exif::MediaParser::new(), &path, MediaKind::Image).width,
            Some(40)
        );
    }

    #[test]
    fn metadata_boxes_are_found() {
        // cjxl compresses the EXIF box.
        let boxes = metadata_boxes(TRANSCODED).unwrap();
        assert!(
            matches!(&boxes[..], [MetadataBox::CompressedExif { tiff, .. }] if tiff.starts_with(b"II*\0") || tiff.starts_with(b"MM\0*"))
        );
        assert!(metadata_boxes(ALPHA).unwrap().is_empty());
        // Compressed data that isn't Brotli: an error, so nothing is shared unchecked.
        let mut bad = TRANSCODED[..12].to_vec();
        bad.extend_from_slice(&[0, 0, 0, 16]);
        bad.extend_from_slice(b"brobExif\xff\xff\xff\xff");
        assert!(metadata_boxes(&bad).is_err());
    }
}
