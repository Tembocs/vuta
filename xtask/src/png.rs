//! The logo images an MSIX package needs, drawn and encoded here so the
//! repository carries no binary assets.
//!
//! The logo is a white "V", which doubles as a downward chevron (vuta means
//! "pull"), on a rounded teal square. The PNG encoder writes uncompressed
//! deflate blocks: the images are tiny, and this keeps xtask free of
//! dependencies.

/// Teal, the background of the tile.
const BACKGROUND: [u8; 3] = [0x0f, 0x76, 0x6e];
/// Samples per pixel along each axis, for anti-aliasing.
const SUPERSAMPLE: u32 = 4;

/// The logo as an RGBA image of `size` by `size` pixels.
pub fn logo(size: u32) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    let n = SUPERSAMPLE * SUPERSAMPLE;
    for y in 0..size {
        for x in 0..size {
            let (mut tile, mut mark) = (0, 0);
            for sy in 0..SUPERSAMPLE {
                for sx in 0..SUPERSAMPLE {
                    let u = (x as f64 + (sx as f64 + 0.5) / SUPERSAMPLE as f64) / size as f64;
                    let v = (y as f64 + (sy as f64 + 0.5) / SUPERSAMPLE as f64) / size as f64;
                    if in_rounded_square(u, v, 0.2) {
                        tile += 1;
                        if in_chevron(u, v) {
                            mark += 1;
                        }
                    }
                }
            }
            // Blend white over teal by the mark's coverage of the tile.
            let blend = |c: u8| {
                if tile == 0 {
                    0
                } else {
                    let t = mark as f64 / tile as f64;
                    (c as f64 * (1.0 - t) + 255.0 * t).round() as u8
                }
            };
            rgba.extend([
                blend(BACKGROUND[0]),
                blend(BACKGROUND[1]),
                blend(BACKGROUND[2]),
                (tile * 255 / n) as u8,
            ]);
        }
    }
    rgba
}

/// Inside the unit square with corners rounded by `radius`.
fn in_rounded_square(u: f64, v: f64, radius: f64) -> bool {
    let cx = u.clamp(radius, 1.0 - radius);
    let cy = v.clamp(radius, 1.0 - radius);
    (u - cx).powi(2) + (v - cy).powi(2) <= radius * radius
}

/// Inside the "V": two thick strokes with round ends meeting at the bottom.
fn in_chevron(u: f64, v: f64) -> bool {
    const HALF_WIDTH: f64 = 0.075;
    let bottom = (0.5, 0.72);
    [(0.27, 0.30), (0.73, 0.30)]
        .iter()
        .any(|&top| distance_to_segment((u, v), top, bottom) <= HALF_WIDTH)
}

fn distance_to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    let (x, y) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - x).powi(2) + (p.1 - y).powi(2)).sqrt()
}

/// Encode 8-bit RGBA pixels as a PNG file.
pub fn encode(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    assert_eq!(
        rgba.len(),
        (width * height * 4) as usize,
        "RGBA buffer size"
    );
    let mut raw = Vec::with_capacity(rgba.len() + height as usize);
    for row in rgba.chunks((width * 4) as usize) {
        raw.push(0); // filter type: none
        raw.extend_from_slice(row);
    }

    let mut ihdr = Vec::new();
    ihdr.extend(width.to_be_bytes());
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]); // 8-bit, RGBA, deflate, adaptive, no interlace

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &zlib_stored(&raw));
    chunk(&mut png, b"IEND", &[]);
    png
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(kind);
    out.extend(data);
    let crc = crc32(&out[start..]);
    out.extend(crc.to_be_bytes());
}

/// A zlib stream of uncompressed deflate blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = if data.is_empty() {
        vec![&[]]
    } else {
        data.chunks(65_535).collect()
    };
    let last = blocks.len() - 1;
    for (i, block) in blocks.into_iter().enumerate() {
        out.push(u8::from(i == last));
        let len = block.len() as u16;
        out.extend(len.to_le_bytes());
        out.extend((!len).to_le_bytes());
        out.extend(block);
    }
    out.extend(adler32(data).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_match_known_values() {
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn encodes_a_valid_structure() {
        let png = encode(2, 1, &[255, 0, 0, 255, 0, 0, 255, 128]);
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[16..24], &[0, 0, 0, 2, 0, 0, 0, 1]);
        assert!(png.ends_with(&[b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82]));
    }

    #[test]
    fn large_images_use_several_blocks() {
        let size = 150;
        let png = encode(size, size, &logo(size));
        // 150 * (150 * 4 + 1) bytes of raw data need two stored blocks.
        assert!(png.len() > 90_000);
    }

    #[test]
    fn logo_has_transparent_corners_and_a_white_mark() {
        let size = 44;
        let img = logo(size);
        let px = |x: u32, y: u32| {
            let i = ((y * size + x) * 4) as usize;
            [img[i], img[i + 1], img[i + 2], img[i + 3]]
        };
        assert_eq!(px(0, 0)[3], 0, "corner is transparent");
        assert_eq!(
            px(22, 2),
            [BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255]
        );
        // The point of the V, at (0.5, 0.72) of the tile.
        assert_eq!(px(22, 31), [255, 255, 255, 255]);
    }
}
