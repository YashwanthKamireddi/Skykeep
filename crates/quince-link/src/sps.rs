//! One hint in an H.264 stream's sequence header, raised so decoders hand pictures on at once.
//!
//! The header's `max_dec_frame_buffering` tells a decoder how many pictures to keep room for.
//! Apple's low-latency encoder sets it equal to its reference pictures (4), so the buffer is
//! always full of references, and decoders that hand a picture on early only while there is a
//! free slot (GStreamer's, for the graphics chip: gsth264decoder.c, max_dpb_size and the
//! low-latency bump) hold every picture until the next one, about 17 ms at 60 fps. One more
//! slot fixes that. The hint only sizes the decoder's buffer; no picture decodes differently.
//!
//! Every other field is copied bit for bit: the header is read field by field (H.264 7.3.2.1.1
//! and E.1.1) and written back with only that number changed.

/// The header (one NAL unit, without its start code) with room for one picture more than its
/// references, or `None` when it already has room or isn't one this reads.
pub fn with_room(nal: &[u8]) -> Option<Vec<u8>> {
    if nal.first()? & 0x1f != 7 {
        return None;
    }
    let rbsp = unescape(&nal[1..]);
    let mut r = Reader {
        data: &rbsp,
        bit: 0,
    };
    let mut w = Writer::default();
    let num_ref_frames = copy_until_buffering(&mut r, &mut w)?;
    let buffering = r.ue()?;
    let wanted = num_ref_frames + 1;
    if buffering >= wanted || wanted > 16 {
        return None;
    }
    w.ue(wanted);
    // The rest (and the trailing bits) as it was.
    while r.bit < rbsp.len() * 8 {
        w.bit(r.bit_at()?);
    }
    let mut out = vec![nal[0]];
    out.extend(escape(&w.bytes()));
    Some(out)
}

/// Every field before `max_dec_frame_buffering`, copied; the reference count, returned.
/// `None` when the header has no bitstream restriction to change.
fn copy_until_buffering(r: &mut Reader, w: &mut Writer) -> Option<u32> {
    let profile = r.copy(w, 8)?;
    r.copy(w, 16)?; // constraint flags, reserved bits, level
    r.copy_ue(w)?; // seq_parameter_set_id
    if matches!(
        profile,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    ) {
        let chroma = r.copy_ue(w)?;
        if chroma == 3 {
            r.copy(w, 1)?;
        }
        r.copy_ue(w)?; // bit_depth_luma_minus8
        r.copy_ue(w)?; // bit_depth_chroma_minus8
        r.copy(w, 1)?; // qpprime_y_zero_transform_bypass
        if r.copy(w, 1)? == 1 {
            for i in 0..(if chroma == 3 { 12 } else { 8 }) {
                if r.copy(w, 1)? == 1 {
                    let size = if i < 6 { 16 } else { 64 };
                    let (mut last, mut next) = (8i32, 8i32);
                    for _ in 0..size {
                        if next != 0 {
                            let delta = r.copy_se(w)?;
                            next = (last + delta + 256) % 256;
                        }
                        last = if next == 0 { last } else { next };
                    }
                }
            }
        }
    }
    r.copy_ue(w)?; // log2_max_frame_num_minus4
    match r.copy_ue(w)? {
        0 => {
            r.copy_ue(w)?;
        }
        1 => {
            r.copy(w, 1)?;
            r.copy_se(w)?;
            r.copy_se(w)?;
            for _ in 0..r.copy_ue(w)? {
                r.copy_se(w)?;
            }
        }
        _ => {}
    }
    let num_ref_frames = r.copy_ue(w)?;
    r.copy(w, 1)?; // gaps_in_frame_num_value_allowed
    r.copy_ue(w)?;
    r.copy_ue(w)?;
    if r.copy(w, 1)? == 0 {
        r.copy(w, 1)?; // mb_adaptive_frame_field
    }
    r.copy(w, 1)?; // direct_8x8_inference
    if r.copy(w, 1)? == 1 {
        for _ in 0..4 {
            r.copy_ue(w)?;
        }
    }
    if r.copy(w, 1)? == 0 {
        return None; // no VUI
    }
    if r.copy(w, 1)? == 1 && r.copy(w, 8)? == 255 {
        r.copy(w, 32)?;
    }
    if r.copy(w, 1)? == 1 {
        r.copy(w, 1)?;
    }
    if r.copy(w, 1)? == 1 {
        r.copy(w, 4)?;
        if r.copy(w, 1)? == 1 {
            r.copy(w, 24)?;
        }
    }
    if r.copy(w, 1)? == 1 {
        r.copy_ue(w)?;
        r.copy_ue(w)?;
    }
    if r.copy(w, 1)? == 1 {
        r.copy(w, 32)?;
        r.copy(w, 32)?;
        r.copy(w, 1)?;
    }
    let nal_hrd = r.copy(w, 1)? == 1;
    if nal_hrd {
        hrd(r, w)?;
    }
    let vcl_hrd = r.copy(w, 1)? == 1;
    if vcl_hrd {
        hrd(r, w)?;
    }
    if nal_hrd || vcl_hrd {
        r.copy(w, 1)?;
    }
    r.copy(w, 1)?; // pic_struct_present
    if r.copy(w, 1)? == 0 {
        return None; // no bitstream restriction
    }
    r.copy(w, 1)?;
    for _ in 0..5 {
        r.copy_ue(w)?; // max_bytes, max_bits, log2 mv lengths, max_num_reorder_frames
    }
    Some(num_ref_frames)
}

fn hrd(r: &mut Reader, w: &mut Writer) -> Option<()> {
    let count = r.copy_ue(w)? + 1;
    r.copy(w, 8)?;
    for _ in 0..count {
        r.copy_ue(w)?;
        r.copy_ue(w)?;
        r.copy(w, 1)?;
    }
    r.copy(w, 20)?;
    Some(())
}

struct Reader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl Reader<'_> {
    fn bit_at(&mut self) -> Option<u32> {
        let byte = *self.data.get(self.bit / 8)?;
        let b = (byte >> (7 - self.bit % 8)) & 1;
        self.bit += 1;
        Some(b as u32)
    }
    fn bits(&mut self, n: u32) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.bit_at()?;
        }
        Some(v)
    }
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bit_at()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some((1u32 << zeros) - 1 + self.bits(zeros)?)
    }
    fn copy(&mut self, w: &mut Writer, n: u32) -> Option<u32> {
        let v = self.bits(n)?;
        for i in (0..n).rev() {
            w.bit((v >> i) & 1);
        }
        Some(v)
    }
    fn copy_ue(&mut self, w: &mut Writer) -> Option<u32> {
        let v = self.ue()?;
        w.ue(v);
        Some(v)
    }
    fn copy_se(&mut self, w: &mut Writer) -> Option<i32> {
        let k = self.ue()?;
        w.ue(k);
        Some(if k % 2 == 1 {
            k.div_ceil(2) as i32
        } else {
            -((k / 2) as i32)
        })
    }
}

#[derive(Default)]
struct Writer {
    out: Vec<u8>,
    bit: usize,
}

impl Writer {
    fn bit(&mut self, b: u32) {
        if self.bit.is_multiple_of(8) {
            self.out.push(0);
        }
        if b == 1 {
            *self.out.last_mut().unwrap() |= 1 << (7 - self.bit % 8);
        }
        self.bit += 1;
    }
    fn ue(&mut self, v: u32) {
        let n = 32 - (v + 1).leading_zeros();
        for _ in 1..n {
            self.bit(0);
        }
        for i in (0..n).rev() {
            self.bit(((v + 1) >> i) & 1);
        }
    }
    fn bytes(self) -> Vec<u8> {
        self.out
    }
}

/// The payload without its emulation-prevention bytes (00 00 03 → 00 00).
fn unescape(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut zeros = 0;
    for &b in data {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

/// The payload with emulation-prevention bytes, so no start code appears inside it.
fn escape(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 4);
    let mut zeros = 0;
    for &b in data {
        if zeros >= 2 && b <= 3 {
            out.push(3);
            zeros = 0;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

/// Each sequence header in an access unit (Annex B) given room; everything else unchanged.
pub fn unit_with_room(unit: &[u8]) -> Option<Vec<u8>> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= unit.len() {
        if unit[i] == 0 && unit[i + 1] == 0 && unit[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut changed = false;
    let mut out = Vec::with_capacity(unit.len() + 8);
    let mut last = 0;
    for (k, &s) in starts.iter().enumerate() {
        let mut e = starts.get(k + 1).map(|n| n - 3).unwrap_or(unit.len());
        if k + 1 < starts.len() && e > s && unit[e - 1] == 0 {
            e -= 1; // the leading zero of a four-byte start code
        }
        if let Some(sps) = unit
            .get(s)
            .filter(|b| *b & 0x1f == 7)
            .and_then(|_| with_room(&unit[s..e]))
        {
            out.extend_from_slice(&unit[last..s]);
            out.extend_from_slice(&sps);
            last = e;
            changed = true;
        }
    }
    if !changed {
        return None;
    }
    out.extend_from_slice(&unit[last..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An iPhone 13's (iOS 27.3) header from its low-latency encoder: High profile, 4
    /// reference pictures, room for 4.
    const IPHONE: [u8; 21] = [
        0x27, 0x64, 0x00, 0x2a, 0xac, 0x13, 0x14, 0x50, 0x1e, 0x00, 0x89, 0xf9, 0x66, 0xa0, 0x40,
        0x40, 0x40, 0xf0, 0x80, 0x42, 0x58,
    ];

    fn read_back(nal: &[u8]) -> (u32, u32, Vec<u8>) {
        let rbsp = unescape(&nal[1..]);
        let mut r = Reader {
            data: &rbsp,
            bit: 0,
        };
        let mut w = Writer::default();
        let refs = copy_until_buffering(&mut r, &mut w).unwrap();
        let buffering = r.ue().unwrap();
        (refs, buffering, w.bytes())
    }

    #[test]
    fn the_iphones_header_gets_one_more_slot_and_nothing_else_changes() {
        let (refs, buffering, before) = read_back(&IPHONE);
        assert_eq!((refs, buffering), (4, 4));
        let roomier = with_room(&IPHONE).unwrap();
        let (refs, buffering, after) = read_back(&roomier);
        assert_eq!((refs, buffering), (4, 5));
        assert_eq!(before, after, "every field before it is the same");
        assert!(
            with_room(&roomier).is_none(),
            "already has room: left as it is"
        );
    }

    #[test]
    fn a_unit_keeps_its_other_nal_units_as_they_were() {
        let mut unit = vec![0, 0, 0, 1];
        unit.extend_from_slice(&IPHONE);
        unit.extend_from_slice(&[
            0, 0, 0, 1, 0x28, 0xee, 0x3c, 0xb0, 0, 0, 1, 0x25, 0x88, 0x84,
        ]);
        let out = unit_with_room(&unit).unwrap();
        assert!(out.ends_with(&[0, 0, 0, 1, 0x28, 0xee, 0x3c, 0xb0, 0, 0, 1, 0x25, 0x88, 0x84]));
        assert!(unit_with_room(&[0, 0, 0, 1, 0x25, 0x88]).is_none());
    }
}
