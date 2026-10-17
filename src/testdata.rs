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
    jpeg_with(&exif_tiff("PixelCam", "Model 1", "2023:06:15 14:30:00"))
}

/// A JPEG with the given capture time (`YYYY:MM:DD HH:MM:SS`).
pub fn jpeg_taken_at(datetime: &str) -> Vec<u8> {
    jpeg_with(&exif_tiff("PixelCam", "Model 1", datetime))
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

/// Builds a little-endian TIFF block with IFD0 (Make, Model, Exif pointer)
/// and an Exif IFD (DateTimeOriginal).
fn exif_tiff(make: &str, model: &str, datetime: &str) -> Vec<u8> {
    const ASCII: u16 = 2;
    const LONG: u16 = 4;
    let ascii = |s: &str| {
        let mut v = s.as_bytes().to_vec();
        v.push(0);
        v
    };
    let (make, model, datetime) = (ascii(make), ascii(model), ascii(datetime));

    // Layout: header(8) | IFD0 (3 entries) | Exif IFD (1 entry) | data
    let ifd_len = |n: usize| 2 + n * 12 + 4;
    let ifd0_at = 8;
    let exif_at = ifd0_at + ifd_len(3);
    let mut data_at = exif_at + ifd_len(1);

    let mut out = b"II*\0".to_vec();
    out.extend_from_slice(&(ifd0_at as u32).to_le_bytes());
    let mut data = Vec::new();
    let mut entry = |out: &mut Vec<u8>, tag: u16, typ: u16, value: &[u8]| {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&typ.to_le_bytes());
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data_at as u32).to_le_bytes());
        data.extend_from_slice(value);
        data_at += value.len();
    };

    out.extend_from_slice(&3u16.to_le_bytes());
    entry(&mut out, 0x010F, ASCII, &make);
    entry(&mut out, 0x0110, ASCII, &model);
    out.extend_from_slice(&0x8769u16.to_le_bytes());
    out.extend_from_slice(&LONG.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(exif_at as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // no next IFD

    out.extend_from_slice(&1u16.to_le_bytes());
    entry(&mut out, 0x9003, ASCII, &datetime);
    out.extend_from_slice(&0u32.to_le_bytes());

    out.extend_from_slice(&data);
    out
}
