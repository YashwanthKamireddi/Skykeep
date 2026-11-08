//! Turning what Apple stores into what the interface draws.
//!
//! Apple keeps a contact's picture at full size and records, separately, the square its owner
//! chose to show. Contacts on a phone draws that square; anything that ignores it shows the
//! middle of the original instead — which for a portrait photo is a chest, not a face. So the
//! square is cut here, in Rust, and the interface is handed a small image that is already
//! right.

/// Pointing libheif at the HEVC decoder an AppImage carries (Linux only).
#[cfg(target_os = "linux")]
pub mod appimage;
/// A video's picture when Apple's is blank, read with GStreamer (Linux only).
#[cfg(target_os = "linux")]
pub mod poster;

/// The square of a picture its owner chose to show, in the stored image's own pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crop {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

impl Crop {
    /// Reads the compact form kept in the library: `x,y,width,height`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split(',').map(|p| p.trim().parse::<i64>().ok());
        let (x, y, width, height) = (
            parts.next()??,
            parts.next()??,
            parts.next()??,
            parts.next()??,
        );
        if parts.next().is_some() || width <= 0 || height <= 0 {
            return None;
        }
        Some(Self {
            x,
            y,
            width,
            height,
        })
    }

    #[must_use]
    pub fn to_text(self) -> String {
        format!("{},{},{},{}", self.x, self.y, self.width, self.height)
    }
}

/// A picture cut to its chosen square and sized for display.
///
/// `size` is the edge of the result. Serving 256 rather than the stored 1696×2528 is most of
/// why a list of faces scrolls: the webview decodes a few kilobytes per row instead of a few
/// hundred.
///
/// # Errors
/// When the bytes are not an image this can decode.
pub fn avatar(bytes: &[u8], crop: Option<Crop>, size: u32) -> Result<Vec<u8>, image::ImageError> {
    let image = image::load_from_memory(bytes)?;
    let square = cut(image, crop);
    // Lanczos: these are faces, and a cheaper filter shows on skin.
    let scaled = square.resize_to_fill(size, size, image::imageops::FilterType::Lanczos3);
    encode(&scaled)
}

/// Cuts out the chosen square, ignoring a rectangle that does not fit the picture it came with.
///
/// A crop is only trusted where it actually lands inside the image. Apple's own API has been
/// seen to send fields that do not match what it serves, and a bad rectangle should cost a
/// centred picture, never a panic or a blank one.
fn cut(image: image::DynamicImage, crop: Option<Crop>) -> image::DynamicImage {
    use image::GenericImageView;
    let (w, h) = image.dimensions();
    let Some(c) = crop else { return image };
    let (Ok(x), Ok(y), Ok(cw), Ok(ch)) = (
        u32::try_from(c.x),
        u32::try_from(c.y),
        u32::try_from(c.width),
        u32::try_from(c.height),
    ) else {
        return image;
    };
    if cw == 0 || ch == 0 || x.saturating_add(cw) > w || y.saturating_add(ch) > h {
        return image;
    }
    // Apple's crop is in the Mac's coordinates: measured from the bottom-left corner, `y`
    // rising up the picture. Read from the top, it drew the wrong band — measured on a real
    // card, 1696×2528 with a 1279 square at y = 1249: from the top that is the bottom half,
    // the body; from the bottom it is the top square, the face, as the phone shows it.
    let top = h - y - ch;
    image.crop_imm(x, top, cw, ch)
}

/// PNG where the picture needs transparency, JPEG where it does not.
///
/// A photograph re-encoded as PNG is several times the size for no visible gain, and these are
/// read back on every scroll.
/// A HEIC (or HEIF) original as a high-quality JPEG, turned, cropped and mirrored as the file
/// says. Apple keeps no full-size JPEG of any HEIC photo (none of 495 in his library) and, for
/// screenshots and the like, often nothing larger than a thumbnail: without this, the sharpest a
/// HEIC photo could be shown was 2048 pixels, and some only a thumbnail stretched — or a broken
/// picture where none could be made. Decoded here from the original, as Photos on a Mac does.
///
/// # Errors
/// When the bytes are not a HEIF image libheif can decode.
pub fn heic_to_jpeg(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let picture = heic_rgb(bytes)?;
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92)
        .encode_image(&picture)
        .map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

#[cfg(all(windows, target_env = "gnu"))]
fn heic_rgb(_bytes: &[u8]) -> Result<image::RgbImage, String> {
    Err("HEIC pictures are not decoded in this build.".into())
}

/// A HEIC picture's pixels, upright (see the variant above for the one build where it is not).
#[cfg(not(all(windows, target_env = "gnu")))]
fn heic_rgb(bytes: &[u8]) -> Result<image::RgbImage, String> {
    use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma};
    let lib = LibHeif::new();
    let ctx = HeifContext::read_from_bytes(bytes).map_err(|e| e.to_string())?;
    let handle = ctx.primary_image_handle().map_err(|e| e.to_string())?;
    let image = lib
        .decode(&handle, ColorSpace::Rgb(RgbChroma::Rgb), None)
        .map_err(|e| e.to_string())?;
    let plane = image
        .planes()
        .interleaved
        .ok_or("the picture had no colour plane")?;
    let (w, h, stride) = (plane.width, plane.height, plane.stride);
    let row = w as usize * 3;
    let mut rgb = Vec::with_capacity(row * h as usize);
    for y in 0..h as usize {
        rgb.extend_from_slice(&plane.data[y * stride..y * stride + row]);
    }
    image::RgbImage::from_raw(w, h, rgb).ok_or_else(|| "the picture had the wrong size".to_string())
}

/// A photo as a PNG, upright: what Copy puts on the clipboard. PNG because it is lossless and
/// is the picture type every desktop's apps take from the clipboard; HEIC never goes there —
/// most apps cannot read it. A HEIC is decoded here (libheif turns it as the file says);
/// anything else is turned the way its camera wrote it, since a PNG carries no orientation of
/// its own. Fast compression: a 12-megapixel photo is ready in well under a second, and the
/// bytes are only held while they are on the clipboard.
///
/// # Errors
/// When the bytes are not a picture this can decode.
pub fn to_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    png_of(&upright(bytes)?)
}

/// A photo for Windows' clipboard: the PNG [`to_png`] makes, which browsers, chat apps and
/// Office take, and the same picture as a device-independent bitmap (`CF_DIB`), the picture
/// type every Windows app takes (Paint, WordPad, older apps). Decoded once for both.
///
/// # Errors
/// When the bytes are not a picture this can decode.
pub fn to_png_and_dib(bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
    let picture = upright(bytes)?;
    Ok((png_of(&picture)?, dib_of(&picture.to_rgb8())))
}

/// A photo as File Explorer's thumbnail takes it: upright, at most `long` pixels on its longer
/// side, as 32-bit pixels, blue first and the top row first (a top-down bitmap's own order),
/// opaque. Made by Skykeep and handed to its thumbnail provider ready to draw, so the code that
/// runs inside Explorer never decodes a picture.
///
/// # Errors
/// When the bytes are not a picture this can decode.
pub fn thumbnail_bgra(bytes: &[u8], long: u32) -> Result<(u32, u32, Vec<u8>), String> {
    let picture = upright(bytes)?;
    let long = long.max(1);
    let picture = if picture.width().max(picture.height()) > long {
        picture.thumbnail(long, long)
    } else {
        picture
    };
    let rgb = picture.to_rgb8();
    let (w, h) = rgb.dimensions();
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for p in rgb.pixels() {
        let [r, g, b] = p.0;
        out.extend_from_slice(&[b, g, r, 255]);
    }
    Ok((w, h, out))
}

/// A photo decoded and turned upright: a HEIC by libheif (which turns it as the file says),
/// anything else the way its camera wrote it.
fn upright(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    use image::ImageDecoder;
    if is_heif(bytes) {
        return Ok(image::DynamicImage::ImageRgb8(heic_rgb(bytes)?));
    }
    let mut decoder = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .into_decoder()
        .map_err(|e| e.to_string())?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut picture = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    picture.apply_orientation(orientation);
    Ok(picture)
}

fn png_of(picture: &image::DynamicImage) -> Result<Vec<u8>, String> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let mut out = Vec::new();
    picture
        .write_with_encoder(PngEncoder::new_with_quality(
            &mut out,
            CompressionType::Fast,
            FilterType::Adaptive,
        ))
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// A picture as Windows' `CF_DIB`: a 40-byte `BITMAPINFOHEADER`, then 24-bit rows, blue first,
/// bottom row first, each padded to a multiple of four bytes. 24-bit because a photo has no
/// transparency, and every app reads it alike (the PNG alongside keeps any there is).
fn dib_of(rgb: &image::RgbImage) -> Vec<u8> {
    let (w, h) = rgb.dimensions();
    let row = (w as usize * 3).div_ceil(4) * 4;
    let image_size = row * h as usize;
    let mut out = Vec::with_capacity(40 + image_size);
    out.extend_from_slice(&40u32.to_le_bytes()); // biSize
    out.extend_from_slice(&w.to_le_bytes()); // biWidth
    out.extend_from_slice(&h.to_le_bytes()); // biHeight: positive, bottom row first
    out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    out.extend_from_slice(&24u16.to_le_bytes()); // biBitCount
    out.extend_from_slice(&0u32.to_le_bytes()); // biCompression: BI_RGB
    out.extend_from_slice(&u32::try_from(image_size).unwrap_or(0).to_le_bytes()); // biSizeImage
    out.extend_from_slice(&2835u32.to_le_bytes()); // biXPelsPerMeter: 72 dpi
    out.extend_from_slice(&2835u32.to_le_bytes()); // biYPelsPerMeter
    out.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    out.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant
    for y in (0..h).rev() {
        let start = out.len();
        for x in 0..w {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            out.extend_from_slice(&[b, g, r]);
        }
        out.resize(start + row, 0);
    }
    out
}

/// RGB rows (`stride` bytes apart) made a JPEG, as heic_to_jpeg makes one.
///
/// # Errors
/// When the rows are shorter than the size says.
pub fn rgb_to_jpeg(data: &[u8], w: u32, h: u32, stride: usize) -> Result<Vec<u8>, String> {
    let row = w as usize * 3;
    let mut rgb = Vec::with_capacity(row * h as usize);
    for y in 0..h as usize {
        rgb.extend_from_slice(
            data.get(y * stride..y * stride + row)
                .ok_or("the frame was cut short")?,
        );
    }
    let picture = image::RgbImage::from_raw(w, h, rgb).ok_or("the picture had the wrong size")?;
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
        .encode_image(&picture)
        .map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// How light RGB rows are, 0–255 on average: a frame of nothing (a recording's black first
/// frame) is near 0.
#[must_use]
pub fn rgb_lightness(data: &[u8], w: u32, h: u32, stride: usize) -> f64 {
    let (mut sum, mut n) = (0u64, 0u64);
    // Every fourth pixel of every fourth row is plenty to tell a picture from black.
    for y in (0..h as usize).step_by(4) {
        let Some(line) = data.get(y * stride..y * stride + w as usize * 3) else {
            break;
        };
        for px in line.as_chunks::<3>().0.iter().step_by(4) {
            sum += u64::from(px[0]) * 3 + u64::from(px[1]) * 6 + u64::from(px[2]);
            n += 10;
        }
    }
    if n == 0 {
        0.0
    } else {
        sum as f64 / n as f64
    }
}

/// Whether a picture shows nothing: black all but a speck, as Apple's poster for a video whose
/// first frame is black (a screen recording's, a film that fades in) is. A few bright pixels —
/// a recording's status-bar dot, a JPEG's ringing at an edge — still read as black: at most
/// one pixel in 200 may be lit.
#[must_use]
pub fn is_blank(bytes: &[u8]) -> bool {
    let Ok(picture) = image::load_from_memory(bytes) else {
        return false;
    };
    let gray = picture.to_luma8();
    let total = u64::from(gray.width()) * u64::from(gray.height());
    if total == 0 {
        return false;
    }
    let lit = gray.pixels().filter(|p| p.0[0] >= 24).count() as u64;
    lit * 200 <= total
}

#[cfg(test)]
mod blank_tests {
    fn jpeg(w: u32, h: u32, lit: u32) -> Vec<u8> {
        let mut img = image::GrayImage::new(w, h);
        for i in 0..lit {
            img.put_pixel(i % w, i / w, image::Luma([255]));
        }
        let mut out = Vec::new();
        image::DynamicImage::ImageLuma8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn black_with_a_speck_is_blank_and_a_picture_is_not() {
        assert!(super::is_blank(&jpeg(100, 100, 0)));
        assert!(super::is_blank(&jpeg(100, 100, 30)), "a status-bar dot");
        assert!(
            !super::is_blank(&jpeg(100, 100, 2000)),
            "a dark picture with something in it"
        );
    }
}

/// The `moov` box of the film at `path`, wherever among the top-level boxes it is, and where
/// it starts. `None` for a file with none (or one too large to be a film's tables).
fn read_moov(path: &std::path::Path) -> std::io::Result<Option<(u64, Vec<u8>)>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    // The `moov` box, wherever among the top-level boxes it is (after the film, often).
    let mut at = 0u64;
    let found = loop {
        if at + 8 > len {
            return Ok(None);
        }
        file.seek(SeekFrom::Start(at))?;
        let mut head = [0u8; 16];
        let got = file.read(&mut head)?;
        if got < 8 {
            return Ok(None);
        }
        let mut size = u64::from(u32::from_be_bytes([head[0], head[1], head[2], head[3]]));
        let kind = &head[4..8];
        if size == 1 && got >= 16 {
            size = u64::from_be_bytes(head[8..16].try_into().unwrap_or_default());
        } else if size == 0 {
            size = len - at;
        }
        if size < 8 {
            return Ok(None);
        }
        if kind == b"moov" {
            // Its tables only: a few megabytes at most for the longest film.
            if size > 64 * 1024 * 1024 {
                return Ok(None);
            }
            let mut moov = vec![0u8; usize::try_from(size).unwrap_or(0)];
            file.seek(SeekFrom::Start(at))?;
            file.read_exact(&mut moov)?;
            break (at, moov);
        }
        at += size;
    };
    Ok(Some(found))
}

/// How much larger a film's picture is than the part of it meant to be seen: a Live Photo's
/// moving part is recorded with a margin round it (its `clap`, the clean aperture — 872 × 654 of
/// a 960 × 720 picture in his library), and the still is that inner part. Drawn whole, the film
/// showed a wider view than the still, smaller within it. 1.0 when the film says nothing of it.
///
/// # Errors
/// When the file cannot be read.
pub fn video_zoom(path: &std::path::Path) -> std::io::Result<f64> {
    let Some((_, moov)) = read_moov(path)? else {
        return Ok(1.0);
    };
    let u32_at = |i: usize| -> Option<u32> {
        Some(u32::from_be_bytes(moov.get(i..i + 4)?.try_into().ok()?))
    };
    let Some(clap) = moov.windows(4).position(|w| w == b"clap") else {
        return Ok(1.0);
    };
    // The picture's own width, from the sample entry the aperture belongs to.
    let entry = moov[..clap]
        .windows(4)
        .rposition(|w| matches!(w, b"avc1" | b"avc3" | b"hvc1" | b"hev1"));
    let (Some(entry), Some(wn), Some(wd)) = (entry, u32_at(clap + 4), u32_at(clap + 8)) else {
        return Ok(1.0);
    };
    // Sample entry: size, type, 6 reserved, data reference (2), 16 pre-defined: then width.
    let Some(width) = moov
        .get(entry + 28..entry + 30)
        .map(|b| f64::from(u16::from_be_bytes([b[0], b[1]])))
    else {
        return Ok(1.0);
    };
    if wd == 0 || wn == 0 {
        return Ok(1.0);
    }
    let seen = f64::from(wn) / f64::from(wd);
    let zoom = width / seen;
    Ok(if (1.0..=1.5).contains(&zoom) {
        zoom
    } else {
        1.0
    })
}

/// A video's turn as its file records it: the picture track's matrix (QuickTime/MP4 `tkhd`) as
/// `[a, b, c, d]`, each -1, 0 or 1, and where in the file the matrix's 36 bytes are. A phone
/// records upright video sideways and says how to turn it here; the front camera's is also a
/// mirror (`[0, 1, 1, 0]`). `None` for a file with no picture track or no such header.
///
/// # Errors
/// When the file cannot be read.
pub fn video_matrix(path: &std::path::Path) -> std::io::Result<Option<(u64, [i32; 4])>> {
    let Some((moov_at, moov)) = read_moov(path)? else {
        return Ok(None);
    };
    let boxes = |from: usize, to: usize| {
        let mut out = Vec::new();
        let mut at = from;
        while at + 8 <= to {
            let size =
                u32::from_be_bytes([moov[at], moov[at + 1], moov[at + 2], moov[at + 3]]) as usize;
            if size < 8 || at + size > to {
                break;
            }
            out.push((&moov[at + 4..at + 8], at, size));
            at += size;
        }
        out
    };
    for (kind, trak, size) in boxes(8, moov.len()) {
        if kind != b"trak" {
            continue;
        }
        for (kind, tkhd, tsize) in boxes(trak + 8, trak + size) {
            if kind != b"tkhd" || tsize < 92 {
                continue;
            }
            let body = tkhd + 8;
            let matrix_at = body + if moov[body] == 1 { 52 } else { 40 };
            if matrix_at + 44 > tkhd + tsize {
                continue;
            }
            let word = |i: usize| {
                i32::from_be_bytes(
                    moov[matrix_at + i * 4..matrix_at + i * 4 + 4]
                        .try_into()
                        .unwrap_or_default(),
                )
            };
            // A picture track has a size; a sound track's is 0 by 0.
            if word(9) == 0 {
                continue;
            }
            let unit = |v: i32| (f64::from(v) / 65536.0).round().clamp(-1.0, 1.0) as i32;
            let m = [unit(word(0)), unit(word(1)), unit(word(3)), unit(word(4))];
            return Ok(Some((moov_at + matrix_at as u64, m)));
        }
    }
    Ok(None)
}

/// The 36 bytes of an upright matrix: what a film is served with once its turn is drawn by the
/// window instead (see `video_matrix`).
pub const UPRIGHT_MATRIX: [u8; 36] = [
    0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //
    0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, //
    0, 0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0, 0,
];

/// The `file://` URI of a path, escaped exactly as GLib escapes it (g_filename_to_uri, its
/// UNSAFE_PATH table in gconvert.c): the freedesktop thumbnail cache names each thumbnail by
/// the MD5 of this string, so one character escaped differently and no file manager finds it.
#[must_use]
pub fn file_uri(path: &std::path::Path) -> String {
    // GLib's table, ASCII 32 to 127; a character is kept when its byte has 0x08 set.
    const ACCEPTABLE: [u8; 96] = [
        0x00, 0x3F, 0x20, 0x20, 0x28, 0x00, 0x2C, 0x3F, 0x3F, 0x3F, 0x3F, 0x2A, 0x28, 0x3F, 0x3F,
        0x1C, //
        0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x38, 0x20, 0x20, 0x2C, 0x20,
        0x20, //
        0x38, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F,
        0x3F, //
        0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x20, 0x20, 0x20, 0x20,
        0x3F, //
        0x20, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F,
        0x3F, //
        0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x3F, 0x20, 0x20, 0x20, 0x3F,
        0x20,
    ];
    let mut out = String::from("file://");
    // On Linux these are the path's own bytes, as GLib reads them.
    for &b in path.as_os_str().as_encoded_bytes() {
        let kept = (32..128).contains(&b) && ACCEPTABLE[usize::from(b - 32)] & 0x08 != 0;
        if kept {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A thumbnail as the freedesktop thumbnail cache keeps one: a PNG no larger than `px` on its
/// longer side (never enlarged), carrying the file's URI and modification time, which a file
/// manager checks before trusting it.
///
/// # Errors
/// When the picture cannot be read or the PNG cannot be made.
pub fn freedesktop_thumbnail(
    picture: &[u8],
    px: u32,
    uri: &str,
    mtime: i64,
) -> Result<Vec<u8>, String> {
    let image = image::load_from_memory(picture).map_err(|e| e.to_string())?;
    let image = if image.width().max(image.height()) > px {
        image.resize(px, px, image::imageops::FilterType::Lanczos3)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, rgba.width(), rgba.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .add_text_chunk("Thumb::URI".into(), uri.into())
            .map_err(|e| e.to_string())?;
        encoder
            .add_text_chunk("Thumb::MTime".into(), mtime.to_string())
            .map_err(|e| e.to_string())?;
        encoder
            .add_text_chunk("Software".into(), "Skykeep".into())
            .map_err(|e| e.to_string())?;
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// The `Thumb::MTime` a freedesktop thumbnail carries, if it is one.
#[must_use]
pub fn thumbnail_mtime(png_bytes: &[u8]) -> Option<i64> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let reader = decoder.read_info().ok()?;
    reader
        .info()
        .uncompressed_latin1_text
        .iter()
        .find(|t| t.keyword == "Thumb::MTime")
        .and_then(|t| t.text.trim().parse().ok())
}

/// A drawing's ink, for a page of either look: Apple renders a drawing as ink on white
/// (its fallback image), and a note is not white. The white is taken back out exactly
/// ("un-multiplied" over white: each pixel's ink and how much of it there is), leaving the
/// ink alone on a transparent page. For `dark`, each colour's lightness is turned and its hue
/// and saturation kept, as a phone shows a drawing in dark mode: black ink white, a yellow
/// still the yellow it was.
///
/// # Errors
/// When the image cannot be read or written.
pub fn drawing_ink(png: &[u8], dark: bool) -> Result<Vec<u8>, String> {
    let mut img = image::load_from_memory(png)
        .map_err(|e| e.to_string())?
        .to_rgba8();
    for px in img.pixels_mut() {
        let [r, g, b, a0] = px.0.map(|c| f32::from(c) / 255.0);
        // How much ink: none on white, all of it wherever a channel is fully dark.
        let ink = 1.0 - r.min(g).min(b);
        if ink <= 1.0 / 255.0 {
            px.0 = [0, 0, 0, 0];
            continue;
        }
        let un = |c: f32| ((c - (1.0 - ink)) / ink).clamp(0.0, 1.0);
        let (mut r, mut g, mut b) = (un(r), un(g), un(b));
        if dark {
            let (h, s, l) = to_hsl(r, g, b);
            (r, g, b) = from_hsl(h, s, 1.0 - l);
        }
        let byte = |c: f32| (c * 255.0).round().clamp(0.0, 255.0) as u8;
        px.0 = [byte(r), byte(g), byte(b), byte(ink * a0)];
    }
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

fn to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d < f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if (max - r).abs() < f32::EPSILON {
        ((g - b) / d).rem_euclid(6.0)
    } else if (max - g).abs() < f32::EPSILON {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s.clamp(0.0, 1.0), l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (r + m, g + m, b + m)
}

/// Whether a freedesktop thumbnail was written by Quince (its `Software` chunk): the ones to
/// remove with an account, and only those — every other app's stay.
#[must_use]
pub fn thumbnail_is_ours(png_bytes: &[u8]) -> bool {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let Ok(reader) = decoder.read_info() else {
        return false;
    };
    reader
        .info()
        .uncompressed_latin1_text
        .iter()
        // Ours under either name: written as Quince before the app was renamed.
        .any(|t| t.keyword == "Software" && (t.text == "Skykeep" || t.text == "Quince"))
}

/// Whether bytes are a HEIF file (HEIC and its kin): an `ftyp` box naming a HEIF brand.
#[must_use]
pub fn is_heif(bytes: &[u8]) -> bool {
    bytes.get(4..8) == Some(b"ftyp")
        && bytes.get(8..12).is_some_and(|b| {
            matches!(
                b,
                b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"mif1" | b"msf1"
            )
        })
}

/// A picture's longer side, read from its header alone (nothing decoded).
#[must_use]
pub fn long_side(bytes: &[u8]) -> Option<u32> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
        .map(|(w, h)| w.max(h))
}

/// A photo made the size it is shown at: its longer side `long` pixels (never larger than it
/// is), turned the way its camera said, resampled with Lanczos.
///
/// WebKitGTK shrinks a large picture by sampling it, with no smoothing first: a 4032-pixel
/// photo drawn 1000 wide came out with broken letters and moiré (its own output, compared with
/// an ideal resample). Drawn at its own size, a picture is exactly as sharp as it can be. This
/// is what the viewer and the grid show; the whole picture only when zoomed in past it.
///
/// `None` when the picture is already no larger: then the original is the one to show.
///
/// # Errors
/// When the bytes are not an image this can decode.
pub fn fit(bytes: &[u8], long: u32) -> Result<Option<Vec<u8>>, image::ImageError> {
    use fast_image_resize as fr;
    use image::ImageDecoder;
    let mut decoder = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()?
        .into_decoder()?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let (w, h) = decoder.dimensions();
    if w.max(h) <= long {
        return Ok(None);
    }
    let mut picture = image::DynamicImage::from_decoder(decoder)?;
    picture.apply_orientation(orientation);
    let (w, h) = (picture.width(), picture.height());
    let k = f64::from(long) / f64::from(w.max(h));
    let (tw, th) = (
        ((f64::from(w) * k).round() as u32).max(1),
        ((f64::from(h) * k).round() as u32).max(1),
    );
    let alpha = transparent(&picture);
    let (pixels, kind) = if alpha {
        (picture.to_rgba8().into_raw(), fr::PixelType::U8x4)
    } else {
        (picture.to_rgb8().into_raw(), fr::PixelType::U8x3)
    };
    let to_err = |e: String| {
        image::ImageError::Parameter(image::error::ParameterError::from_kind(
            image::error::ParameterErrorKind::Generic(e),
        ))
    };
    let src =
        fr::images::Image::from_vec_u8(w, h, pixels, kind).map_err(|e| to_err(e.to_string()))?;
    let mut dst = fr::images::Image::new(tw, th, kind);
    fr::Resizer::new()
        .resize(
            &src,
            &mut dst,
            &fr::ResizeOptions::new()
                .resize_alg(fr::ResizeAlg::Convolution(fr::FilterType::Lanczos3)),
        )
        .map_err(|e| to_err(e.to_string()))?;
    let mut out = std::io::Cursor::new(Vec::new());
    if alpha {
        image::RgbaImage::from_raw(tw, th, dst.into_vec())
            .ok_or_else(|| to_err("resized picture had the wrong size".into()))?
            .write_to(&mut out, image::ImageFormat::Png)?;
    } else {
        let rgb = image::RgbImage::from_raw(tw, th, dst.into_vec())
            .ok_or_else(|| to_err("resized picture had the wrong size".into()))?;
        // High quality: this is the picture people look at, not a row's avatar.
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92).encode_image(&rgb)?;
    }
    Ok(Some(out.into_inner()))
}

fn encode(image: &image::DynamicImage) -> Result<Vec<u8>, image::ImageError> {
    let mut out = std::io::Cursor::new(Vec::new());
    if transparent(image) {
        image.write_to(&mut out, image::ImageFormat::Png)?;
    } else {
        let rgb = image.to_rgb8();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88);
        encoder.encode_image(&rgb)?;
    }
    Ok(out.into_inner())
}

fn transparent(image: &image::DynamicImage) -> bool {
    image.color().has_alpha() && image.to_rgba8().pixels().any(|p| p.0[3] != 255)
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_thumbnail_for_explorer_keeps_its_shape_blue_first() {
        let red = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            400,
            200,
            image::Rgb([255, 0, 0]),
        ));
        let png = super::png_of(&red).unwrap();
        let (w, h, px) = super::thumbnail_bgra(&png, 100).unwrap();
        assert_eq!((w, h, px.len()), (100, 50, 100 * 50 * 4));
        assert_eq!(&px[..4], &[0, 0, 255, 255]);
        // Smaller than asked: as it is, never enlarged.
        assert_eq!(super::thumbnail_bgra(&png, 1000).unwrap().0, 400);
    }
    #[test]
    fn a_photo_is_made_its_shown_size_turned_the_camera_s_way() {
        // 400×300 with a red left half; JPEG.
        let img = image::RgbImage::from_fn(400, 300, |x, _| {
            if x < 200 {
                image::Rgb([255, 0, 0])
            } else {
                image::Rgb([0, 0, 255])
            }
        });
        let mut jpg = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut jpg, image::ImageFormat::Jpeg)
            .unwrap();
        let out = super::fit(jpg.get_ref(), 200).unwrap().unwrap();
        let back = image::load_from_memory(&out).unwrap();
        assert_eq!((back.width(), back.height()), (200, 150));
        let left = back.to_rgb8().get_pixel(20, 75).0;
        assert!(left[0] > 200 && left[2] < 60, "{left:?}");
        // Never made larger than it is.
        assert!(super::fit(jpg.get_ref(), 800).unwrap().is_none());
    }

    /// Copy's picture from a HEIC: a PNG, whole and in its colours (red left, blue right — a
    /// 64×32 HEIC made with libheif's own heif-enc).
    #[cfg(not(all(windows, target_env = "gnu")))]
    #[test]
    fn a_heic_is_copied_as_a_png() {
        let heic = include_bytes!("../fixtures/two.heic");
        assert!(super::is_heif(heic));
        let png = super::to_png(heic).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let back = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .unwrap()
            .to_rgb8();
        assert_eq!(back.dimensions(), (64, 32));
        let (left, right) = (back.get_pixel(8, 16).0, back.get_pixel(56, 16).0);
        assert!(left[0] > 200 && left[2] < 60, "{left:?}");
        assert!(right[2] > 200 && right[0] < 60, "{right:?}");
    }

    /// A JPEG the camera wrote on its side comes out upright: a PNG has no orientation to say
    /// it with, so the turn is made in the pixels.
    #[test]
    fn a_jpeg_is_copied_upright() {
        let img = image::RgbImage::from_fn(40, 20, |x, _| {
            if x < 20 {
                image::Rgb([255, 0, 0])
            } else {
                image::Rgb([0, 0, 255])
            }
        });
        let mut jpg = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut jpg),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        // An EXIF block saying "turn a quarter clockwise" (orientation 6), put after SOI.
        let exif: &[u8] =
            b"Exif\0\0MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0";
        let len = u16::try_from(exif.len() + 2).unwrap().to_be_bytes();
        let mut turned = jpg[..2].to_vec();
        turned.extend_from_slice(&[0xFF, 0xE1, len[0], len[1]]);
        turned.extend_from_slice(exif);
        turned.extend_from_slice(&jpg[2..]);
        let back = image::load_from_memory(&super::to_png(&turned).unwrap())
            .unwrap()
            .to_rgb8();
        assert_eq!(back.dimensions(), (20, 40));
        // Turned clockwise, the red left half is now the top.
        let top = back.get_pixel(10, 5).0;
        assert!(top[0] > 200 && top[2] < 60, "{top:?}");
        assert!(super::to_png(b"not a picture").is_err());
    }

    /// Windows' copy of a photo: the PNG, and the same picture as a `CF_DIB` laid out as
    /// Windows reads one — header, bottom row first, blue first, rows padded to four bytes.
    #[test]
    fn a_photo_is_copied_for_windows_as_a_png_and_a_bitmap() {
        // 3×2: the top row red, the bottom row blue (3 pixels = 9 bytes, padded to 12).
        let img = image::RgbImage::from_fn(3, 2, |_, y| {
            if y == 0 {
                image::Rgb([255, 0, 0])
            } else {
                image::Rgb([0, 0, 255])
            }
        });
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let (out_png, dib) = super::to_png_and_dib(&png).unwrap();
        assert_eq!(&out_png[..8], b"\x89PNG\r\n\x1a\n");
        let u32_at = |i: usize| u32::from_le_bytes(dib[i..i + 4].try_into().unwrap());
        let u16_at = |i: usize| u16::from_le_bytes(dib[i..i + 2].try_into().unwrap());
        assert_eq!(u32_at(0), 40, "BITMAPINFOHEADER");
        assert_eq!((u32_at(4), u32_at(8)), (3, 2), "width, height (bottom-up)");
        assert_eq!((u16_at(12), u16_at(14)), (1, 24), "planes, bits per pixel");
        assert_eq!(u32_at(16), 0, "BI_RGB");
        assert_eq!(u32_at(20), 24, "two rows of 12 bytes");
        assert_eq!(dib.len(), 40 + 24);
        // The bottom (blue) row first, each pixel blue-green-red, then the padding.
        assert_eq!(&dib[40..52], &[255, 0, 0, 255, 0, 0, 255, 0, 0, 0, 0, 0]);
        assert_eq!(&dib[52..64], &[0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 0]);
        assert!(super::to_png_and_dib(b"not a picture").is_err());
    }

    use super::*;

    /// A drawing's white goes, its ink stays; in dark, black ink turns white and a mid-tone
    /// colour keeps its hue and lightness (yellow stays yellow), as on a phone.
    #[test]
    fn a_drawing_is_its_ink_on_either_page() {
        let mut page = image::RgbImage::from_pixel(3, 1, image::Rgb([255, 255, 255]));
        page.put_pixel(1, 0, image::Rgb([0, 0, 0]));
        page.put_pixel(2, 0, image::Rgb([255, 204, 0]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(page)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let read = |b: Vec<u8>| image::load_from_memory(&b).unwrap().to_rgba8();
        let light = read(drawing_ink(&png, false).unwrap());
        assert_eq!(light.get_pixel(0, 0).0[3], 0, "the white is gone");
        assert_eq!(
            light.get_pixel(1, 0).0,
            [0, 0, 0, 255],
            "black ink stays black"
        );
        assert_eq!(
            light.get_pixel(2, 0).0,
            [255, 204, 0, 255],
            "a colour stays itself"
        );
        let dark = read(drawing_ink(&png, true).unwrap());
        assert_eq!(
            dark.get_pixel(1, 0).0,
            [255, 255, 255, 255],
            "black ink turns white"
        );
        let y = dark.get_pixel(2, 0).0;
        assert!(
            y[0] > y[2] && y[1] > y[2] && y[3] == 255,
            "yellow stays yellow: {y:?}"
        );
    }

    /// Only Quince's own thumbnails are told apart as ours: another app's are never touched.
    #[test]
    fn our_thumbnails_are_known_by_their_software_chunk() {
        let mut plain = Vec::new();
        image::DynamicImage::new_rgb8(4, 4)
            .write_to(
                &mut std::io::Cursor::new(&mut plain),
                image::ImageFormat::Png,
            )
            .unwrap();
        let ours = freedesktop_thumbnail(&plain, 128, "file:///home/test/a.png", 1).unwrap();
        assert!(thumbnail_is_ours(&ours));
        assert!(!thumbnail_is_ours(&plain));
        assert!(!thumbnail_is_ours(b"not a png"));
    }
    use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};

    /// A tall picture with a red square at its top right — where a face sits in a portrait.
    fn portrait() -> Vec<u8> {
        let mut img = RgbaImage::from_pixel(100, 200, Rgba([0, 0, 255, 255]));
        for y in 0..50 {
            for x in 50..100 {
                img.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }
        let mut out = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    fn middle_pixel(bytes: &[u8]) -> [u8; 3] {
        let img = image::load_from_memory(bytes).unwrap();
        let p = img.get_pixel(img.width() / 2, img.height() / 2);
        [p.0[0], p.0[1], p.0[2]]
    }

    /// The whole point: a portrait photo cropped where its owner said, not through the middle.
    /// `y` is from the bottom, as Apple measures it: 150 up a 200-tall picture, a 50 square
    /// is the top one — the face — and read from the top it would have been the bottom.
    #[test]
    fn the_chosen_square_is_what_gets_drawn() {
        let source = portrait();
        let chosen = Crop {
            x: 50,
            y: 150,
            width: 50,
            height: 50,
        };

        let cropped = avatar(&source, Some(chosen), 64).unwrap();
        assert!(
            middle_pixel(&cropped)[0] > 200,
            "the chosen square is the red one"
        );

        let centred = avatar(&source, None, 64).unwrap();
        assert!(
            middle_pixel(&centred)[2] > 200,
            "with no crop it centres, which is why the crop matters"
        );
    }

    #[test]
    fn the_result_is_the_size_that_was_asked_for() {
        let out = avatar(&portrait(), None, 48).unwrap();
        let img = image::load_from_memory(&out).unwrap();
        assert_eq!(img.dimensions(), (48, 48));
    }

    /// Apple's own API sends fields that do not match what it serves. A rectangle falling off
    /// the picture must cost a centred face, not a crash or an empty frame.
    #[test]
    fn a_rectangle_that_does_not_fit_is_ignored_rather_than_obeyed() {
        for bad in [
            Crop {
                x: 0,
                y: 0,
                width: 500,
                height: 500,
            },
            Crop {
                x: 90,
                y: 190,
                width: 50,
                height: 50,
            },
            Crop {
                x: -10,
                y: 0,
                width: 20,
                height: 20,
            },
        ] {
            let out = avatar(&portrait(), Some(bad), 32).unwrap();
            assert_eq!(
                image::load_from_memory(&out).unwrap().dimensions(),
                (32, 32)
            );
        }
    }

    #[test]
    fn a_crop_survives_the_round_trip_through_the_library() {
        let c = Crop {
            x: 251,
            y: 1249,
            width: 1279,
            height: 1279,
        };
        assert_eq!(Crop::parse(&c.to_text()), Some(c));
        assert_eq!(Crop::parse("1,2,3"), None);
        assert_eq!(Crop::parse("1,2,3,4,5"), None);
        assert_eq!(Crop::parse("1,2,0,4"), None, "an empty square is no square");
        assert_eq!(Crop::parse("a,b,c,d"), None);
    }

    /// A photograph should not be shipped as a PNG; a picture with holes in it must be.
    #[test]
    fn the_format_follows_what_the_picture_needs() {
        let opaque = avatar(&portrait(), None, 64).unwrap();
        assert_eq!(&opaque[..3], b"\xff\xd8\xff", "opaque becomes a JPEG");

        let mut img = RgbaImage::from_pixel(20, 20, Rgba([0, 0, 0, 0]));
        img.put_pixel(10, 10, Rgba([255, 255, 255, 255]));
        let mut src = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(img)
            .write_to(&mut src, image::ImageFormat::Png)
            .unwrap();
        let holes = avatar(&src.into_inner(), None, 64).unwrap();
        assert_eq!(&holes[..4], b"\x89PNG", "transparency stays a PNG");
    }

    #[test]
    fn bytes_that_are_not_a_picture_are_an_error_not_a_panic() {
        assert!(avatar(b"404 Not Found", None, 64).is_err());
        assert!(avatar(b"", None, 64).is_err());
    }
}

#[cfg(test)]
mod zoom_tests {
    fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = u32::try_from(body.len() + 8)
            .unwrap()
            .to_be_bytes()
            .to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    /// A Live Photo's film as Apple writes it: a 960 × 720 picture whose clean aperture is
    /// 872 × 654. Drawn 960 / 872 larger, only what the still shows is seen.
    #[test]
    fn a_films_margin_is_read_from_its_clean_aperture() {
        let mut clap = Vec::new();
        for v in [872u32, 1, 654, 1, 0, 1, 0, 1] {
            clap.extend_from_slice(&v.to_be_bytes());
        }
        let mut entry = vec![0u8; 24];
        entry.extend_from_slice(&960u16.to_be_bytes());
        entry.extend_from_slice(&720u16.to_be_bytes());
        entry.extend_from_slice(&[0u8; 50]);
        entry.extend(boxed(b"clap", &clap));
        let moov = boxed(b"moov", &boxed(b"stsd", &boxed(b"avc1", &entry)));
        let mut file = boxed(b"ftyp", b"qt  \0\0\0\0");
        file.extend(moov);
        let dir = std::env::temp_dir().join(format!("quince-zoom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("live.mov");
        std::fs::write(&path, &file).unwrap();
        let z = super::video_zoom(&path).unwrap();
        assert!((z - 960.0 / 872.0).abs() < 1e-9, "{z}");
        // No aperture said: drawn as it is.
        let plain = dir.join("plain.mov");
        let mut f = boxed(b"ftyp", b"qt  \0\0\0\0");
        f.extend(boxed(b"moov", &boxed(b"stsd", &[0u8; 16])));
        std::fs::write(&plain, &f).unwrap();
        assert!((super::video_zoom(&plain).unwrap() - 1.0).abs() < 1e-9);
        std::fs::remove_dir_all(&dir).ok();
    }
}
