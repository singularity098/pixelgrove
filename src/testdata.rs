//! Tiny synthetic media files for tests.

/// A 3×2 PNG header (enough for dimension sniffing).
pub const PNG_3X2: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, // signature
    0, 0, 0, 13, b'I', b'H', b'D', b'R', // IHDR chunk
    0, 0, 0, 3, 0, 0, 0, 2, 8, 2, 0, 0, 0, // 3×2, 8-bit RGB
    0x12, 0x16, 0xF1, 0x4D, // CRC
    0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82,
];

/// A JPEG whose EXIF says Make "PixelCam", Model "Model 1", taken 2023-06-15 14:30:00.
pub fn jpeg_with_exif() -> Vec<u8> {
    jpeg_with(&exif_tiff("2023:06:15 14:30:00", None))
}

/// A JPEG with the given capture time (`YYYY:MM:DD HH:MM:SS`).
pub fn jpeg_taken_at(datetime: &str) -> Vec<u8> {
    jpeg_with(&exif_tiff(datetime, None))
}

/// A JPEG with a capture time and GPS position (decimal degrees).
pub fn jpeg_at_location(datetime: &str, lat: f64, lon: f64) -> Vec<u8> {
    jpeg_with(&exif_tiff(datetime, Some((lat, lon))))
}

fn jpeg_with(tiff: &[u8]) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8]; // SOI
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend_from_slice(tiff);
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&app1);
    // SOF0: 8-bit, 4×3 pixels, 1 component.
    out.extend_from_slice(&[0xFF, 0xC0, 0, 11, 8, 0, 3, 0, 4, 1, 1, 0x11, 0]);
    out.extend_from_slice(&[0xFF, 0xD9]); // EOI
    out
}

const ASCII: u16 = 2;
const LONG: u16 = 4;
const RATIONAL: u16 = 5;

/// One IFD entry: tag, type, value count and the raw little-endian value bytes.
struct Entry {
    tag: u16,
    typ: u16,
    count: u32,
    value: Vec<u8>,
}

fn ascii(tag: u16, s: &str) -> Entry {
    let mut value = s.as_bytes().to_vec();
    value.push(0);
    Entry {
        tag,
        typ: ASCII,
        count: value.len() as u32,
        value,
    }
}

fn long(tag: u16, v: u32) -> Entry {
    Entry {
        tag,
        typ: LONG,
        count: 1,
        value: v.to_le_bytes().to_vec(),
    }
}

/// Degrees as the EXIF degrees/minutes/seconds rationals.
fn dms(tag: u16, degrees: f64) -> Entry {
    let d = degrees.abs();
    let deg = d.trunc();
    let min = ((d - deg) * 60.0).trunc();
    let sec = ((d - deg) * 60.0 - min) * 60.0;
    let mut value = Vec::new();
    for (num, den) in [
        (deg as u32, 1u32),
        (min as u32, 1),
        ((sec * 1000.0).round() as u32, 1000),
    ] {
        value.extend_from_slice(&num.to_le_bytes());
        value.extend_from_slice(&den.to_le_bytes());
    }
    Entry {
        tag,
        typ: RATIONAL,
        count: 3,
        value,
    }
}

/// Lays out IFDs one after another, each followed by its out-of-line values.
/// The IFD0 pointer entries (0x8769 Exif, 0x8825 GPS) are patched to the later IFDs.
pub fn exif_tiff(datetime: &str, gps: Option<(f64, f64)>) -> Vec<u8> {
    let mut ifds: Vec<Vec<Entry>> = vec![
        vec![
            ascii(0x010F, "PixelCam"),
            ascii(0x0110, "Model 1"),
            long(0x8769, 0), // Exif IFD pointer, patched below
        ],
        vec![ascii(0x9003, datetime)],
    ];
    if let Some((lat, lon)) = gps {
        ifds[0].push(long(0x8825, 0)); // GPS IFD pointer, patched below
        ifds.push(vec![
            ascii(0x0001, if lat >= 0.0 { "N" } else { "S" }),
            dms(0x0002, lat),
            ascii(0x0003, if lon >= 0.0 { "E" } else { "W" }),
            dms(0x0004, lon),
        ]);
    }
    // Sizes: 2 + 12n + 4 for the IFD itself, plus out-of-line values (> 4 bytes).
    let sizes: Vec<usize> = ifds
        .iter()
        .map(|ifd| {
            2 + ifd.len() * 12
                + 4
                + ifd
                    .iter()
                    .filter(|e| e.value.len() > 4)
                    .map(|e| e.value.len())
                    .sum::<usize>()
        })
        .collect();
    let mut offsets = vec![8usize];
    for size in &sizes[..sizes.len() - 1] {
        offsets.push(offsets.last().unwrap() + size);
    }
    for entry in &mut ifds[0] {
        match entry.tag {
            0x8769 => entry.value = (offsets[1] as u32).to_le_bytes().to_vec(),
            0x8825 => entry.value = (offsets[2] as u32).to_le_bytes().to_vec(),
            _ => {}
        }
    }

    let mut out = b"II*\0".to_vec();
    out.extend_from_slice(&8u32.to_le_bytes());
    for (ifd, &start) in ifds.iter().zip(&offsets) {
        let mut data_at = start + 2 + ifd.len() * 12 + 4;
        let mut data = Vec::new();
        out.extend_from_slice(&(ifd.len() as u16).to_le_bytes());
        for e in ifd {
            out.extend_from_slice(&e.tag.to_le_bytes());
            out.extend_from_slice(&e.typ.to_le_bytes());
            out.extend_from_slice(&e.count.to_le_bytes());
            if e.value.len() <= 4 {
                let mut inline = e.value.clone();
                inline.resize(4, 0);
                out.extend_from_slice(&inline);
            } else {
                out.extend_from_slice(&(data_at as u32).to_le_bytes());
                data.extend_from_slice(&e.value);
                data_at += e.value.len();
            }
        }
        out.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
        out.extend_from_slice(&data);
    }
    out
}
