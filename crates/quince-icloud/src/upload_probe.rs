//! A file looked at before any of it is sent to Photos: its first bytes and, for a photo, the
//! header that gives its size; for a video, the boxes that name its codecs. Apple only says
//! no after the whole file is up (`putAsset`, 415), which for a video is up to 500 MB sent
//! for nothing (D#262).
//!
//! Only headers are read: a handful of small reads, never the file, never anything held in
//! proportion to its size, and a cap on how many boxes are looked at, so a damaged or hostile
//! file costs the same as a good one. A file is refused only on something proven refused;
//! what this can't read, or isn't sure of, goes as it is and Apple decides.
//!
//! Evidence, so a new refusal adds one row (docs/research/2026-11-25/media-conversion.md):
//! - Sizes: icloud.com's own uploader refuses past 20 480 px a side or 419 430 400 px in all
//!   (photos3 `ui.main.js`, docs/icloud-web/photos.md §6).
//! - MPEG-4 Part 2 and Motion JPEG video, and camera PCM sound (`twos`, `sowt`, `raw `; only the
//!   iPhone's `lpcm` is taken): refused by the same `photosupload` service in real uploads
//!   (isynca PRs #3, #8, #9; a Windows Phone MP4, an Olympus MOV, a Pentax MOV).
//! - AVI, Matroska/WebM, MPEG program and transport streams, ASF/WMV, Flash video: containers
//!   none of Apple's apps read, and not in icloud.com's own list of what it uploads. WeChat's
//!   WXGF, saved under a `.jpg` name by some export tools, is raw HEVC no one else reads.

use crate::photos_upload::{Refusal, MAX_AREA, MAX_SIDE};
use std::io::{Read, Seek, SeekFrom};

/// The most boxes or segments looked at in one file: far more than any real file's headers.
const BUDGET: u32 = 20_000;
/// The most tracks' sample descriptions looked at, and entries in each.
const ENTRIES: usize = 8;

/// What the file's contents make it a refusal for, if anything. `len` is its length.
pub(crate) fn inspect<R: Read + Seek>(r: &mut R, len: u64) -> Option<Refusal> {
    let mut s = Io(r);
    let mut head = [0u8; 512];
    let n = s.read_up_to(&mut head);
    let head = &head[..n];
    if let Some(what) = foreign(head) {
        return Some(Refusal::Unsupported(what.into()));
    }
    let size = match head {
        [0xFF, 0xD8, 0xFF, ..] => jpeg_size(&mut s),
        [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, ..] => png_size(head),
        [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => gif_size(head),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => webp_size(head),
        [b'I', b'I', 42, 0, ..] => tiff_size(&mut s, false),
        [b'M', b'M', 0, 42, ..] => tiff_size(&mut s, true),
        _ if bmff(head) => {
            if heif(head) {
                heif_size(&mut s, len)
            } else {
                return video(&mut s, len).map(|what| Refusal::Unsupported(what.into()));
            }
        }
        _ => None,
    };
    let (w, h) = size?;
    (w > MAX_SIDE || h > MAX_SIDE || u64::from(w) * u64::from(h) > MAX_AREA)
        .then_some(Refusal::Dimensions(w, h))
}

/// A container Apple takes under no name, by its first bytes.
fn foreign(head: &[u8]) -> Option<&'static str> {
    let at = |i: usize, sig: &[u8]| head.get(i..i + sig.len()) == Some(sig);
    if at(0, b"RIFF") && at(8, b"AVI ") {
        return Some("an AVI video");
    }
    if at(0, &[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some("an MKV or WebM video");
    }
    if at(0, &[0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11]) {
        return Some("a Windows Media video");
    }
    if at(0, b"FLV\x01") {
        return Some("a Flash video");
    }
    if at(0, b"wxgf") {
        return Some("a WeChat WXGF picture, not a JPEG");
    }
    // An MP4 box sized 0x1BA would read the same: those name their type next.
    if at(0, &[0, 0, 1, 0xBA]) && !bmff(head) {
        return Some("an MPEG program stream video");
    }
    // A transport stream: a sync byte every 188 bytes, three in a row.
    if [0, 188, 376].iter().all(|&i| head.get(i) == Some(&0x47)) {
        return Some("an MPEG transport stream video");
    }
    None
}

/// An ISO media file (MP4, MOV, HEIF): `ftyp` first, or an older QuickTime movie's first box.
fn bmff(head: &[u8]) -> bool {
    matches!(
        head.get(4..8),
        Some(b"ftyp" | b"moov" | b"mdat" | b"wide" | b"free" | b"skip" | b"pnot")
    )
}

/// An ISO file that is a still image (HEIC, HEIF, AVIF), by its major brand.
fn heif(head: &[u8]) -> bool {
    head.get(4..8) == Some(b"ftyp")
        && matches!(
            head.get(8..12),
            Some(b"heic" | b"heix" | b"heim" | b"heis" | b"mif1" | b"mif2" | b"avif")
        )
}

/// Reads at an offset, `None` past the end or on any error.
struct Io<'a, R>(&'a mut R);

impl<R: Read + Seek> Io<'_, R> {
    fn read_at(&mut self, at: u64, buf: &mut [u8]) -> Option<()> {
        self.0.seek(SeekFrom::Start(at)).ok()?;
        self.0.read_exact(buf).ok()
    }

    fn read_up_to(&mut self, buf: &mut [u8]) -> usize {
        if self.0.seek(SeekFrom::Start(0)).is_err() {
            return 0;
        }
        let mut n = 0;
        while n < buf.len() {
            match self.0.read(&mut buf[n..]) {
                Ok(0) | Err(_) => break,
                Ok(k) => n += k,
            }
        }
        n
    }

    fn u8_at(&mut self, at: u64) -> Option<u8> {
        let mut b = [0u8; 1];
        self.read_at(at, &mut b)?;
        Some(b[0])
    }

    fn u16_at(&mut self, at: u64, big: bool) -> Option<u16> {
        let mut b = [0u8; 2];
        self.read_at(at, &mut b)?;
        Some(if big {
            u16::from_be_bytes(b)
        } else {
            u16::from_le_bytes(b)
        })
    }

    fn u32_at(&mut self, at: u64, big: bool) -> Option<u32> {
        let mut b = [0u8; 4];
        self.read_at(at, &mut b)?;
        Some(if big {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        })
    }

    fn kind_at(&mut self, at: u64) -> Option<[u8; 4]> {
        let mut b = [0u8; 4];
        self.read_at(at, &mut b)?;
        Some(b)
    }
}

fn be16(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from(u16::from_be_bytes(
        b.get(i..i + 2)?.try_into().ok()?,
    )))
}

fn le16(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes(
        b.get(i..i + 2)?.try_into().ok()?,
    )))
}

fn le24(b: &[u8], i: usize) -> Option<u32> {
    let b = b.get(i..i + 3)?;
    Some(u32::from(b[0]) | u32::from(b[1]) << 8 | u32::from(b[2]) << 16)
}

/// A JPEG's size from its frame header (SOFn), walking the segments before it by their
/// lengths; an EXIF thumbnail sits inside APP1 and is stepped over whole.
fn jpeg_size<R: Read + Seek>(s: &mut Io<R>) -> Option<(u32, u32)> {
    let mut at = 2u64;
    for _ in 0..BUDGET {
        let mut m = [0u8; 2];
        s.read_at(at, &mut m)?;
        if m[0] != 0xFF {
            return None;
        }
        match m[1] {
            0xFF => at += 1,
            0x01 | 0xD0..=0xD8 => at += 2,
            0xD9 | 0xDA => return None,
            marker => {
                let len = u64::from(s.u16_at(at + 2, true)?);
                if len < 2 {
                    return None;
                }
                if matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
                    let mut f = [0u8; 5];
                    s.read_at(at + 4, &mut f)?;
                    let (h, w) = (be16(&f, 1)?, be16(&f, 3)?);
                    return (w > 0 && h > 0).then_some((w, h));
                }
                at += 2 + len;
            }
        }
    }
    None
}

fn png_size(head: &[u8]) -> Option<(u32, u32)> {
    if head.get(12..16) != Some(b"IHDR") {
        return None;
    }
    let w = u32::from_be_bytes(head.get(16..20)?.try_into().ok()?);
    let h = u32::from_be_bytes(head.get(20..24)?.try_into().ok()?);
    Some((w, h))
}

fn gif_size(head: &[u8]) -> Option<(u32, u32)> {
    Some((le16(head, 6)?, le16(head, 8)?))
}

fn webp_size(head: &[u8]) -> Option<(u32, u32)> {
    match head.get(12..16)? {
        b"VP8X" => Some((le24(head, 24)? + 1, le24(head, 27)? + 1)),
        b"VP8 " if head.get(23..26) == Some(&[0x9D, 0x01, 0x2A]) => {
            Some((le16(head, 26)? & 0x3FFF, le16(head, 28)? & 0x3FFF))
        }
        b"VP8L" if head.get(20) == Some(&0x2F) => {
            let bits = u32::from_le_bytes(head.get(21..25)?.try_into().ok()?);
            Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
        }
        _ => None,
    }
}

/// A TIFF's size from its first directory (a DNG's first is often its preview: smaller, so
/// never a refusal it doesn't deserve).
fn tiff_size<R: Read + Seek>(s: &mut Io<R>, big: bool) -> Option<(u32, u32)> {
    let ifd = u64::from(s.u32_at(4, big)?);
    let count = s.u16_at(ifd, big)?;
    let (mut w, mut h) = (None, None);
    for i in 0..u64::from(count.min(1024)) {
        let e = ifd + 2 + 12 * i;
        let tag = s.u16_at(e, big)?;
        if tag != 256 && tag != 257 {
            continue;
        }
        let value = match s.u16_at(e + 2, big)? {
            3 => u32::from(s.u16_at(e + 8, big)?),
            4 => s.u32_at(e + 8, big)?,
            _ => return None,
        };
        if tag == 256 {
            w = Some(value);
        } else {
            h = Some(value);
        }
        if let (Some(w), Some(h)) = (w, h) {
            return Some((w, h));
        }
    }
    None
}

/// One ISO box: its type, where its contents start, and where it ends.
#[derive(Clone, Copy)]
struct Bx {
    kind: [u8; 4],
    body: u64,
    end: u64,
}

/// The boxes between `at` and `end`, read by their headers alone. Stops at the first that
/// doesn't fit (a damaged or cut-off file keeps what came before it), and once `budget` boxes
/// have been read across the whole file.
fn boxes<R: Read + Seek>(s: &mut Io<R>, mut at: u64, end: u64, budget: &mut u32) -> Vec<Bx> {
    let mut out = Vec::new();
    while at.saturating_add(8) <= end && *budget > 0 {
        *budget -= 1;
        let (Some(size), Some(kind)) = (s.u32_at(at, true), s.kind_at(at + 4)) else {
            break;
        };
        let (size, header) = match size {
            0 => (end - at, 8),
            1 => {
                let mut l = [0u8; 8];
                if s.read_at(at + 8, &mut l).is_none() {
                    break;
                }
                (u64::from_be_bytes(l), 16)
            }
            n => (u64::from(n), 8),
        };
        if size < header || size > end - at {
            break;
        }
        out.push(Bx {
            kind,
            body: at + header,
            end: at + size,
        });
        at += size;
    }
    out
}

fn find(list: &[Bx], kind: &[u8; 4]) -> Option<Bx> {
    list.iter().find(|b| &b.kind == kind).copied()
}

/// A video's tracks, each's handler and sample descriptions read, for a codec Apple refuses.
fn video<R: Read + Seek>(s: &mut Io<R>, len: u64) -> Option<&'static str> {
    let mut left = BUDGET;
    let budget = &mut left;
    let top = boxes(s, 0, len, budget);
    let moov = find(&top, b"moov")?;
    let traks: Vec<Bx> = boxes(s, moov.body, moov.end, budget)
        .into_iter()
        .filter(|b| &b.kind == b"trak")
        .take(ENTRIES)
        .collect();
    for trak in traks {
        let Some(mdia) = find(&boxes(s, trak.body, trak.end, budget), b"mdia") else {
            continue;
        };
        let parts = boxes(s, mdia.body, mdia.end, budget);
        // hdlr: version and flags, then pre_defined (QuickTime: the component type), then
        // the handler (`vide`, `soun`).
        let handler = find(&parts, b"hdlr")
            .filter(|h| h.body + 12 <= h.end)
            .and_then(|h| s.kind_at(h.body + 8));
        let Some(handler) = handler else { continue };
        let Some(minf) = find(&parts, b"minf") else {
            continue;
        };
        let Some(stbl) = find(&boxes(s, minf.body, minf.end, budget), b"stbl") else {
            continue;
        };
        let Some(stsd) = find(&boxes(s, stbl.body, stbl.end, budget), b"stsd") else {
            continue;
        };
        // stsd: version and flags, an entry count, then one box per sample description.
        for entry in boxes(s, stsd.body + 8, stsd.end, budget)
            .into_iter()
            .take(ENTRIES)
        {
            if let Some(why) = refused_codec(&handler, &entry.kind) {
                return Some(why);
            }
        }
    }
    None
}

/// The codecs Apple's server has turned away in real uploads, by track kind and fourcc.
fn refused_codec(handler: &[u8; 4], codec: &[u8; 4]) -> Option<&'static str> {
    match (handler, codec) {
        (b"vide", b"mp4v") => Some("MPEG-4 Part 2 video, as early phones and cameras made"),
        (b"vide", b"jpeg" | b"mjpa" | b"mjpb") => {
            Some("Motion JPEG video, as early digital cameras made")
        }
        (b"soun", b"twos" | b"sowt" | b"raw ") => {
            Some("a camera\u{2019}s PCM sound, which only an iPhone\u{2019}s form of is taken")
        }
        _ => None,
    }
}

/// A HEIF's size: the `ispe` property of its primary item (`pitm`), found through `ipma`.
fn heif_size<R: Read + Seek>(s: &mut Io<R>, len: u64) -> Option<(u32, u32)> {
    let mut left = BUDGET;
    let budget = &mut left;
    let top = boxes(s, 0, len, budget);
    let meta = find(&top, b"meta")?;
    let kids = boxes(s, meta.body + 4, meta.end, budget);
    let pitm = find(&kids, b"pitm")?;
    let primary = if s.u8_at(pitm.body)? == 0 {
        u32::from(s.u16_at(pitm.body + 4, true)?)
    } else {
        s.u32_at(pitm.body + 4, true)?
    };
    let iprp = find(&kids, b"iprp")?;
    let iprp = boxes(s, iprp.body, iprp.end, budget);
    let ipco = find(&iprp, b"ipco")?;
    let props = boxes(s, ipco.body, ipco.end, budget);
    for ipma in iprp.iter().filter(|b| &b.kind == b"ipma") {
        let version = s.u8_at(ipma.body)?;
        let wide = s.u8_at(ipma.body + 3)? & 1 == 1;
        let count = s.u32_at(ipma.body + 4, true)?;
        let mut at = ipma.body + 8;
        for _ in 0..count.min(BUDGET) {
            let id = if version < 1 {
                let id = u32::from(s.u16_at(at, true)?);
                at += 2;
                id
            } else {
                let id = s.u32_at(at, true)?;
                at += 4;
                id
            };
            let n = s.u8_at(at)?;
            at += 1;
            for _ in 0..n {
                if at >= ipma.end {
                    return None;
                }
                let index = if wide {
                    let v = s.u16_at(at, true)? & 0x7FFF;
                    at += 2;
                    usize::from(v)
                } else {
                    let v = s.u8_at(at)? & 0x7F;
                    at += 1;
                    usize::from(v)
                };
                if id != primary || index == 0 {
                    continue;
                }
                let Some(p) = props.get(index - 1) else {
                    continue;
                };
                if &p.kind == b"ispe" && p.body + 12 <= p.end {
                    return Some((s.u32_at(p.body + 4, true)?, s.u32_at(p.body + 8, true)?));
                }
            }
            if at > ipma.end {
                return None;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn look(bytes: &[u8]) -> Option<Refusal> {
        inspect(&mut Cursor::new(bytes), bytes.len() as u64)
    }

    fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    fn full(kind: &[u8; 4], version: u8, body: &[u8]) -> Vec<u8> {
        let mut b = vec![version, 0, 0, 0];
        b.extend_from_slice(body);
        bx(kind, &b)
    }

    /// A JPEG of this size: SOI, an APP1 holding a decoy frame header (a thumbnail's), the
    /// real SOF0, then a scan.
    fn jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut j = vec![0xFF, 0xD8];
        let decoy = [0xFF, 0xC0, 0, 11, 8, 0, 1, 0, 1, 3, 0, 0, 0];
        j.extend_from_slice(&[0xFF, 0xE1]);
        j.extend_from_slice(&((decoy.len() + 2) as u16).to_be_bytes());
        j.extend_from_slice(&decoy);
        j.extend_from_slice(&[0xFF, 0xC0, 0, 11, 8]);
        j.extend_from_slice(&h.to_be_bytes());
        j.extend_from_slice(&w.to_be_bytes());
        j.extend_from_slice(&[1, 1, 0x11, 0]);
        j.extend_from_slice(&[0xFF, 0xDA, 0, 8, 1, 1, 0, 0, 0x3F, 0]);
        j.extend_from_slice(&[0u8; 200]);
        j.extend_from_slice(&[0xFF, 0xD9]);
        j
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut p = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        p.extend_from_slice(b"IHDR");
        p.extend_from_slice(&w.to_be_bytes());
        p.extend_from_slice(&h.to_be_bytes());
        p.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
        p.extend_from_slice(&[0u8; 200]);
        p
    }

    fn gif(w: u16, h: u16) -> Vec<u8> {
        let mut g = b"GIF89a".to_vec();
        g.extend_from_slice(&w.to_le_bytes());
        g.extend_from_slice(&h.to_le_bytes());
        g.extend_from_slice(&[0u8; 200]);
        g
    }

    fn webp_x(w: u32, h: u32) -> Vec<u8> {
        let mut v = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        v.extend_from_slice(&10u32.to_le_bytes());
        v.extend_from_slice(&[0, 0, 0, 0]);
        v.extend_from_slice(&(w - 1).to_le_bytes()[..3]);
        v.extend_from_slice(&(h - 1).to_le_bytes()[..3]);
        v.extend_from_slice(&[0u8; 200]);
        v
    }

    fn tiff(w: u32, h: u32, big: bool) -> Vec<u8> {
        let u16b = |v: u16| {
            if big {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        let u32b = |v: u32| {
            if big {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        let mut t = if big {
            b"MM\0*".to_vec()
        } else {
            b"II*\0".to_vec()
        };
        t.extend_from_slice(&u32b(8));
        t.extend_from_slice(&u16b(2));
        for (tag, v) in [(256u16, w), (257, h)] {
            t.extend_from_slice(&u16b(tag));
            t.extend_from_slice(&u16b(4));
            t.extend_from_slice(&u32b(1));
            t.extend_from_slice(&u32b(v));
        }
        t.extend_from_slice(&[0u8; 200]);
        t
    }

    /// A HEIC whose primary item (2) is `w` × `h`; item 1, a thumbnail, is larger still so a
    /// reader that took any `ispe` would be caught.
    fn heic(w: u32, h: u32) -> Vec<u8> {
        let mut out = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
        let ispe = |w: u32, h: u32| {
            let mut b = w.to_be_bytes().to_vec();
            b.extend_from_slice(&h.to_be_bytes());
            full(b"ispe", 0, &b)
        };
        let mut ipco = ispe(30_000, 30_000);
        ipco.extend(ispe(w, h));
        let ipco = bx(b"ipco", &ipco);
        // Item 1 → property 1, item 2 → property 2 (essential bit set on the second).
        let ipma = full(b"ipma", 0, &[0, 0, 0, 2, 0, 1, 1, 1, 0, 2, 1, 0x82]);
        let mut iprp = ipco;
        iprp.extend(ipma);
        let mut meta = full(b"pitm", 0, &[0, 2]);
        meta.extend(bx(b"iprp", &iprp));
        out.extend(full(b"meta", 0, &meta));
        out.extend(bx(b"mdat", &[0u8; 200]));
        out
    }

    /// An MP4 with one track per (handler, codec): moov → trak → mdia → hdlr + minf → stbl →
    /// stsd. `moov_last` puts the bytes first, as most cameras write it.
    fn mp4(tracks: &[(&[u8; 4], &[u8; 4])], moov_last: bool) -> Vec<u8> {
        let mut moov = Vec::new();
        for (handler, codec) in tracks {
            let mut hdlr = b"\0\0\0\0".to_vec();
            hdlr.extend_from_slice(*handler);
            hdlr.extend_from_slice(&[0u8; 13]);
            let mut stsd = 1u32.to_be_bytes().to_vec();
            stsd.extend(bx(codec, &[0u8; 30]));
            let stbl = bx(b"stbl", &full(b"stsd", 0, &stsd));
            let minf = bx(b"minf", &stbl);
            let mut mdia = full(b"mdhd", 0, &[0u8; 20]);
            mdia.extend(full(b"hdlr", 0, &hdlr));
            mdia.extend(minf);
            let mut trak = full(b"tkhd", 0, &[0u8; 80]);
            trak.extend(bx(b"mdia", &mdia));
            moov.extend(bx(b"trak", &trak));
        }
        let mut out = bx(b"ftyp", b"isom\0\0\0\0isommp42");
        let moov = bx(b"moov", &moov);
        let mdat = bx(b"mdat", &[0u8; 300]);
        if moov_last {
            out.extend(mdat);
            out.extend(moov);
        } else {
            out.extend(moov);
            out.extend(mdat);
        }
        out
    }

    #[test]
    fn photos_within_apple_sizes_go() {
        assert_eq!(look(&jpeg(4032, 3024)), None);
        assert_eq!(look(&png(20_480, 100)), None);
        assert_eq!(look(&gif(640, 480)), None);
        assert_eq!(look(&webp_x(16_000, 16_000)), None);
        assert_eq!(look(&tiff(6000, 4000, false)), None);
        assert_eq!(look(&tiff(6000, 4000, true)), None);
        assert_eq!(look(&heic(4032, 3024)), None);
    }

    /// Past 20 480 px a side or 419 430 400 px in all, as icloud.com refuses before sending.
    #[test]
    fn photos_past_apple_sizes_are_refused() {
        assert_eq!(
            look(&jpeg(30_000, 2000)),
            Some(Refusal::Dimensions(30_000, 2000))
        );
        assert_eq!(
            look(&png(100, 20_481)),
            Some(Refusal::Dimensions(100, 20_481))
        );
        assert_eq!(
            look(&gif(40_000, 10)),
            Some(Refusal::Dimensions(40_000, 10))
        );
        // Under the side limit both ways, over the area.
        assert_eq!(
            look(&webp_x(20_480, 20_481)),
            Some(Refusal::Dimensions(20_480, 20_481))
        );
        assert_eq!(
            look(&tiff(21_000, 21_000, true)),
            Some(Refusal::Dimensions(21_000, 21_000))
        );
        assert_eq!(
            look(&heic(20_000, 21_000)),
            Some(Refusal::Dimensions(20_000, 21_000))
        );
        let words = Refusal::Dimensions(30_000, 2000).words();
        assert!(words.contains("30000 \u{d7} 2000"), "{words}");
    }

    /// Containers none of Apple's apps read are refused by their first bytes, whatever the name.
    #[test]
    fn foreign_containers_are_refused() {
        let pad = |mut v: Vec<u8>| {
            v.resize(600, 0);
            v
        };
        let mut ts = vec![0u8; 600];
        for i in [0, 188, 376] {
            ts[i] = 0x47;
        }
        for (bytes, what) in [
            (pad(b"RIFF\0\0\0\0AVI LIST".to_vec()), "AVI"),
            (pad(vec![0x1A, 0x45, 0xDF, 0xA3]), "MKV"),
            (pad(vec![0, 0, 1, 0xBA, 0x44]), "program stream"),
            (ts, "transport stream"),
            (
                pad(vec![0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11]),
                "Windows Media",
            ),
            (pad(b"FLV\x01\x05".to_vec()), "Flash"),
            (pad(b"wxgf\x01".to_vec()), "WXGF"),
        ] {
            let Some(Refusal::Unsupported(said)) = look(&bytes) else {
                panic!("{what} refused");
            };
            assert!(said.contains(what), "{said}");
        }
        // WebP is RIFF too, and goes.
        assert_eq!(look(&webp_x(100, 100)), None);
    }

    /// Codecs Apple's server turns away, read from the sample descriptions, wherever `moov` is.
    #[test]
    fn refused_codecs_are_found() {
        for last in [false, true] {
            assert_eq!(
                look(&mp4(&[(b"vide", b"avc1"), (b"soun", b"mp4a")], last)),
                None
            );
            assert_eq!(
                look(&mp4(&[(b"vide", b"hvc1"), (b"soun", b"lpcm")], last)),
                None
            );
            let refused: [(&[u8; 4], &[u8; 4], &str); 6] = [
                (b"vide", b"mp4v", "MPEG-4 Part 2"),
                (b"vide", b"jpeg", "Motion JPEG"),
                (b"vide", b"mjpa", "Motion JPEG"),
                (b"soun", b"twos", "PCM"),
                (b"soun", b"sowt", "PCM"),
                (b"soun", b"raw ", "PCM"),
            ];
            for (handler, codec, what) in refused {
                // Alongside a track that is fine, in either order.
                let other: (&[u8; 4], &[u8; 4]) = if handler == b"vide" {
                    (b"soun", b"mp4a")
                } else {
                    (b"vide", b"avc1")
                };
                for tracks in [[other, (handler, codec)], [(handler, codec), other]] {
                    let Some(Refusal::Unsupported(said)) = look(&mp4(&tracks, last)) else {
                        panic!("{what} refused");
                    };
                    assert!(said.contains(what), "{said}");
                }
            }
        }
        // `raw ` in a video track is uncompressed picture, not camera sound: not proven refused.
        assert_eq!(look(&mp4(&[(b"vide", b"raw ")], false)), None);
    }

    /// Damaged, cut-off and hostile files: never a panic, never a refusal on a guess.
    #[test]
    fn malformed_files_neither_panic_nor_refuse() {
        let samples = [
            jpeg(30_000, 2000),
            png(30_000, 10),
            tiff(30_000, 30_000, false),
            heic(30_000, 30_000),
            mp4(&[(b"vide", b"mp4v")], false),
            mp4(&[(b"vide", b"mp4v")], true),
            webp_x(30_000, 30_000),
        ];
        for sample in &samples {
            // Every prefix: none may panic.
            for cut in 0..sample.len() {
                let _ = look(&sample[..cut]);
            }
            // Every single byte flipped: none may panic.
            for i in 0..sample.len().min(400) {
                let mut bad = sample.clone();
                bad[i] ^= 0xFF;
                let _ = look(&bad);
            }
        }
        // Garbage, short files and empty ones go as they are: Apple decides.
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        for n in [0usize, 1, 3, 8, 64, 511, 512, 4096] {
            let junk: Vec<u8> = (0..n)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed as u8
                })
                .collect();
            let _ = look(&junk);
        }
        assert_eq!(look(&[7u8; 200]), None);
        assert_eq!(look(&[]), None);
        // A box that claims to be bigger than the file, and a 64-bit size past it.
        let mut huge = bx(b"ftyp", b"isom\0\0\0\0isom");
        huge.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
        huge.extend_from_slice(b"moov");
        assert_eq!(look(&huge), None);
        let mut big64 = bx(b"ftyp", b"isom\0\0\0\0isom");
        big64.extend_from_slice(&1u32.to_be_bytes());
        big64.extend_from_slice(b"moov");
        big64.extend_from_slice(&u64::MAX.to_be_bytes());
        assert_eq!(look(&big64), None);
        // Endless tiny boxes are cut off by the budget, not walked forever.
        let mut many = bx(b"ftyp", b"isom\0\0\0\0isom");
        for _ in 0..50_000 {
            many.extend(bx(b"free", &[]));
        }
        many.extend(mp4(&[(b"vide", b"mp4v")], false));
        assert_eq!(look(&many), None);
        // A JPEG of nothing but fill bytes ends.
        let mut fill = vec![0xFF, 0xD8];
        fill.extend(std::iter::repeat_n(0xFF, 100_000));
        assert_eq!(look(&fill), None);
    }
}
