//! CCS/1 M:clipboard images (WN-36) | IO:CF_DIB/CF_DIBV5 or the "PNG" format <-> PNG + thumbnail | SEC:bounded sizes, no payload logs.
use wind_core::clipboard::{OutgoingImage, MAX_SIDE, MAX_THUMBNAIL};

/// Long side of a thumbnail (previews only).
pub const THUMBNAIL_SIDE: u32 = 512;

#[derive(Debug, PartialEq, Eq)]
pub enum ImageError {
    /// Not an image we read (bad header, unsupported depth or compression).
    Unsupported,
    /// Wider or taller than `MAX_SIDE`.
    TooBig,
}

/// 8-bit RGBA pixels, top row first.
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// A PNG as it came (the clipboard's "PNG" format, or the network): its size
/// and pixels.
pub fn decode_png(png: &[u8]) -> Result<Rgba, ImageError> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|_| ImageError::Unsupported)?;
    let (width, height) = {
        let info = reader.info();
        (info.width, info.height)
    };
    if width == 0 || height == 0 {
        return Err(ImageError::Unsupported);
    }
    if u64::from(width) > MAX_SIDE || u64::from(height) > MAX_SIDE {
        return Err(ImageError::TooBig);
    }
    let mut buffer = vec![0; reader.output_buffer_size().ok_or(ImageError::Unsupported)?];
    let frame = reader
        .next_frame(&mut buffer)
        .map_err(|_| ImageError::Unsupported)?;
    let data = &buffer[..frame.buffer_size()];
    let pixels = match frame.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::Indexed => return Err(ImageError::Unsupported),
    };
    Ok(Rgba {
        width,
        height,
        pixels,
    })
}

/// Encodes RGBA as PNG (RGB when fully opaque: smaller). Fast compression:
/// a screenshot has to be on the phone in well under half a second.
pub fn encode_png(image: &Rgba) -> Vec<u8> {
    let opaque = image.pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255);
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        if opaque {
            encoder.set_color(png::ColorType::Rgb);
            let rgb: Vec<u8> = image
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect();
            let mut writer = encoder.write_header().expect("in-memory PNG header");
            writer.write_image_data(&rgb).expect("in-memory PNG data");
        } else {
            encoder.set_color(png::ColorType::Rgba);
            let mut writer = encoder.write_header().expect("in-memory PNG header");
            writer
                .write_image_data(&image.pixels)
                .expect("in-memory PNG data");
        }
    }
    out
}

/// Box-filter downscale so the long side is at most `side`.
pub fn downscale(image: &Rgba, side: u32) -> Rgba {
    let long = image.width.max(image.height);
    if long <= side {
        return Rgba {
            width: image.width,
            height: image.height,
            pixels: image.pixels.clone(),
        };
    }
    let w = ((u64::from(image.width) * u64::from(side)) / u64::from(long)).max(1) as u32;
    let h = ((u64::from(image.height) * u64::from(side)) / u64::from(long)).max(1) as u32;
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let y0 = (u64::from(y) * u64::from(image.height) / u64::from(h)) as u32;
        let y1 = ((u64::from(y + 1) * u64::from(image.height) / u64::from(h)) as u32).max(y0 + 1);
        for x in 0..w {
            let x0 = (u64::from(x) * u64::from(image.width) / u64::from(w)) as u32;
            let x1 =
                ((u64::from(x + 1) * u64::from(image.width) / u64::from(w)) as u32).max(x0 + 1);
            let mut sum = [0u64; 4];
            for sy in y0..y1 {
                let row = (sy * image.width) as usize * 4;
                for sx in x0..x1 {
                    let p = row + sx as usize * 4;
                    for (total, value) in sum.iter_mut().zip(&image.pixels[p..p + 4]) {
                        *total += u64::from(*value);
                    }
                }
            }
            let n = u64::from((y1 - y0) * (x1 - x0));
            pixels.extend(sum.iter().map(|s| (s / n) as u8));
        }
    }
    Rgba {
        width: w,
        height: h,
        pixels,
    }
}

/// A thumbnail PNG within `MAX_THUMBNAIL` (smaller sides until it fits).
pub fn thumbnail(image: &Rgba) -> Vec<u8> {
    let mut side = THUMBNAIL_SIDE;
    loop {
        let png = encode_png(&downscale(image, side));
        if png.len() <= MAX_THUMBNAIL || side <= 32 {
            return png;
        }
        side /= 2;
    }
}

/// What goes out for a PNG from the clipboard: the bytes as they are.
pub fn outgoing_from_png(png: Vec<u8>) -> Result<OutgoingImage, ImageError> {
    let image = decode_png(&png)?;
    Ok(OutgoingImage {
        thumbnail: thumbnail(&image),
        width: u64::from(image.width),
        height: u64::from(image.height),
        png,
    })
}

/// What goes out for a DIB (CF_DIB / CF_DIBV5): encoded as PNG.
pub fn outgoing_from_dib(dib: &[u8]) -> Result<OutgoingImage, ImageError> {
    let image = decode_dib(dib)?;
    Ok(OutgoingImage {
        thumbnail: thumbnail(&image),
        width: u64::from(image.width),
        height: u64::from(image.height),
        png: encode_png(&image),
    })
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// A packed DIB (BITMAPINFOHEADER or BITMAPV4/V5HEADER, then pixels): 24- or
/// 32-bit, BI_RGB or BI_BITFIELDS with byte-aligned masks. A 32-bit BI_RGB
/// DIB whose alpha bytes are all 0 is opaque (the common screenshot case).
pub fn decode_dib(dib: &[u8]) -> Result<Rgba, ImageError> {
    const BI_RGB: u32 = 0;
    const BI_BITFIELDS: u32 = 3;
    let header = u32_at(dib, 0).ok_or(ImageError::Unsupported)? as usize;
    if header < 40 || header > dib.len() {
        return Err(ImageError::Unsupported);
    }
    let width = u32_at(dib, 4).ok_or(ImageError::Unsupported)? as i32;
    let height = u32_at(dib, 8).ok_or(ImageError::Unsupported)? as i32;
    let bits = u16::from_le_bytes([dib[14], dib[15]]);
    let compression = u32_at(dib, 16).ok_or(ImageError::Unsupported)?;
    let colors_used = u32_at(dib, 32).ok_or(ImageError::Unsupported)? as usize;
    if width <= 0 || height == 0 {
        return Err(ImageError::Unsupported);
    }
    let (w, h) = (width as u32, height.unsigned_abs());
    if u64::from(w) > MAX_SIDE || u64::from(h) > MAX_SIDE {
        return Err(ImageError::TooBig);
    }
    let top_down = height < 0;
    let mut offset = header;
    // Masks: inside a V4/V5 header, or three DWORDs after a 40-byte header.
    let masks = match compression {
        BI_RGB => None,
        BI_BITFIELDS if bits == 32 => {
            let at = if header >= 52 { 40 } else { header };
            if header < 52 {
                offset += 12;
            }
            Some([
                u32_at(dib, at).ok_or(ImageError::Unsupported)?,
                u32_at(dib, at + 4).ok_or(ImageError::Unsupported)?,
                u32_at(dib, at + 8).ok_or(ImageError::Unsupported)?,
                if header >= 56 {
                    u32_at(dib, 52).ok_or(ImageError::Unsupported)?
                } else {
                    0
                },
            ])
        }
        _ => return Err(ImageError::Unsupported),
    };
    if !matches!(bits, 24 | 32) {
        return Err(ImageError::Unsupported);
    }
    offset += colors_used * 4;
    let stride = (w as usize * bits as usize).div_ceil(32) * 4;
    let need = offset + stride * h as usize;
    if dib.len() < need {
        return Err(ImageError::Unsupported);
    }
    let shift = |mask: u32| -> Option<u32> {
        match mask {
            0 => None,
            m if m.count_ones() == 8 && m.trailing_zeros() % 8 == 0 => Some(m.trailing_zeros()),
            _ => None,
        }
    };
    let (rs, gs, bs, alpha_shift) = match masks {
        None => (
            Some(16),
            Some(8),
            Some(0),
            if bits == 32 { Some(24) } else { None },
        ),
        Some([r, g, b, a]) => {
            let (r, g, b) = (shift(r), shift(g), shift(b));
            if r.is_none() || g.is_none() || b.is_none() {
                return Err(ImageError::Unsupported);
            }
            (r, g, b, shift(a))
        }
    };
    let mut pixels = Vec::with_capacity(w as usize * h as usize * 4);
    let mut any_alpha = false;
    for y in 0..h as usize {
        let row = if top_down { y } else { h as usize - 1 - y };
        let line = &dib[offset + row * stride..];
        for x in 0..w as usize {
            let value = if bits == 32 {
                u32::from_le_bytes(line[x * 4..x * 4 + 4].try_into().unwrap())
            } else {
                u32::from(line[x * 3])
                    | u32::from(line[x * 3 + 1]) << 8
                    | u32::from(line[x * 3 + 2]) << 16
            };
            let at = |s: Option<u32>| s.map(|s| (value >> s) as u8);
            let a = at(alpha_shift).unwrap_or(255);
            any_alpha |= a != 0;
            pixels.extend([at(rs).unwrap(), at(gs).unwrap(), at(bs).unwrap(), a]);
        }
    }
    if !any_alpha {
        pixels
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .for_each(|p| p[3] = 255);
    }
    Ok(Rgba {
        width: w,
        height: h,
        pixels,
    })
}

/// A packed top-down 32-bit DIB (BITMAPINFOHEADER, BI_RGB, BGRA) for CF_DIB.
pub fn encode_dib(image: &Rgba) -> Vec<u8> {
    let mut out = Vec::with_capacity(40 + image.pixels.len());
    out.extend(40u32.to_le_bytes());
    out.extend((image.width as i32).to_le_bytes());
    out.extend((-(image.height as i32)).to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(32u16.to_le_bytes());
    out.extend(0u32.to_le_bytes()); // BI_RGB
    out.extend((image.width * image.height * 4).to_le_bytes());
    out.extend([0u8; 16]); // resolution, colours used, important
    out.extend(
        image
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[2], p[1], p[0], p[3]]),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: u32, height: u32, alpha: bool) -> Rgba {
        let pixels = (0..width * height)
            .flat_map(|i| {
                let (x, y) = (i % width, i / width);
                [
                    (x % 256) as u8,
                    (y % 256) as u8,
                    ((x + y) % 256) as u8,
                    if alpha { (x % 200) as u8 + 55 } else { 255 },
                ]
            })
            .collect();
        Rgba {
            width,
            height,
            pixels,
        }
    }

    #[test]
    fn png_round_trip_keeps_pixels() {
        for alpha in [false, true] {
            let image = gradient(37, 19, alpha);
            let back = decode_png(&encode_png(&image)).unwrap();
            assert_eq!((back.width, back.height), (37, 19));
            assert!(back.pixels == image.pixels, "alpha {alpha}");
        }
    }

    #[test]
    fn dib_round_trip_and_bottom_up_24_bit() {
        let image = gradient(5, 3, true);
        let back = decode_dib(&encode_dib(&image)).unwrap();
        assert!(back.pixels == image.pixels);
        // A bottom-up 24-bit DIB (rows padded to 4 bytes), as Paint puts it.
        let (w, h) = (3u32, 2u32);
        let mut dib = Vec::new();
        dib.extend(40u32.to_le_bytes());
        dib.extend((w as i32).to_le_bytes());
        dib.extend((h as i32).to_le_bytes());
        dib.extend(1u16.to_le_bytes());
        dib.extend(24u16.to_le_bytes());
        dib.extend([0u8; 24]);
        // Bottom row first: blue pixels, then the top row: red.
        for row in [[255u8, 0, 0], [0, 0, 255]] {
            for _ in 0..w {
                dib.extend(row);
            }
            dib.extend([0u8; 3]); // 9 bytes -> stride 12
        }
        let image = decode_dib(&dib).unwrap();
        assert_eq!(&image.pixels[..4], &[255, 0, 0, 255], "top-left red");
        assert_eq!(&image.pixels[12..16], &[0, 0, 255, 255], "bottom-left blue");
    }

    #[test]
    fn a_32_bit_dib_with_zero_alpha_is_opaque_and_bitfields_work() {
        let mut image = gradient(4, 4, false);
        image
            .pixels
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .for_each(|p| p[3] = 0);
        let opaque = decode_dib(&encode_dib(&image)).unwrap();
        assert!(opaque.pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        // BI_BITFIELDS with masks after a 40-byte header (R G B in BGRA order).
        let mut dib = encode_dib(&gradient(2, 1, false));
        dib[16..20].copy_from_slice(&3u32.to_le_bytes());
        let masks: Vec<u8> = [0x00ff_0000u32, 0x0000_ff00, 0x0000_00ff]
            .iter()
            .flat_map(|m| m.to_le_bytes())
            .collect();
        dib.splice(40..40, masks);
        assert!(decode_dib(&dib).unwrap().pixels == gradient(2, 1, false).pixels);
    }

    #[test]
    fn bad_and_oversized_dibs_are_refused() {
        assert_eq!(decode_dib(&[1, 2, 3]).err(), Some(ImageError::Unsupported));
        let mut short = encode_dib(&gradient(4, 4, false));
        short.truncate(60);
        assert_eq!(decode_dib(&short).err(), Some(ImageError::Unsupported));
        let mut wide = encode_dib(&gradient(1, 1, false));
        wide[4..8].copy_from_slice(&(MAX_SIDE as i32 + 1).to_le_bytes());
        assert_eq!(decode_dib(&wide).err(), Some(ImageError::TooBig));
        let mut eight_bit = encode_dib(&gradient(1, 1, false));
        eight_bit[14..16].copy_from_slice(&8u16.to_le_bytes());
        assert_eq!(decode_dib(&eight_bit).err(), Some(ImageError::Unsupported));
    }

    #[test]
    fn screenshot_thumbnail_fits_and_keeps_the_aspect() {
        let shot = gradient(1920, 1080, false);
        let out = outgoing_from_dib(&encode_dib(&shot)).unwrap();
        assert_eq!((out.width, out.height), (1920, 1080));
        assert!(out.thumbnail.len() <= MAX_THUMBNAIL);
        let thumb = decode_png(&out.thumbnail).unwrap();
        // Long side 512 px, or smaller when that PNG would exceed the cap
        // (this noisy gradient does; real screenshots compress far better).
        assert!(thumb.width <= THUMBNAIL_SIDE && thumb.width >= 64);
        assert_eq!(thumb.width * 9, thumb.height * 16, "aspect kept");
        assert!(decode_png(&out.png).unwrap().pixels == shot.pixels);
    }
}
