//! Reading the text out of CloudKit "document" fields.
//!
//! Reminders and Notes keep their title and body in fields CloudKit types as
//! `ENCRYPTED_BYTES`. On an account without Advanced Data Protection that name is a
//! misnomer: the payload begins `1f 8b 08` and is simply gzip. Inside is a protobuf whose
//! text sits at field path 2 → 3 → 2.
//!
//! Rather than model Apple's whole document schema — which is large, undocumented, and would
//! break on any change — this walks to that one path. Everything here treats the input as
//! hostile: a malformed payload yields `None`, never a panic and never an allocation sized
//! from an attacker-controlled length.

use base64::{engine::general_purpose::STANDARD as B64, Engine};

/// Where the readable text lives inside the document protobuf.
const TEXT_PATH: &[u64] = &[2, 3, 2];

/// The most a document may unpack to. It bounds a hostile payload (a few kilobytes that would
/// inflate to gigabytes), and is far above any real note: 8 MB, the limit once guessed here,
/// is a long journal or a pasted book, and such a note opened as nothing at all. A note's text
/// is several bytes a character with its formatting, so this is tens of millions of characters.
const MAX_DECOMPRESSED: u64 = 128 * 1024 * 1024;

/// Wire types we can advance past. Anything else means the bytes are not what we expect.
const WIRE_VARINT: u64 = 0;
const WIRE_64BIT: u64 = 1;
const WIRE_LEN: u64 = 2;
const WIRE_32BIT: u64 = 5;

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn done(&self) -> bool {
        self.at >= self.bytes.len()
    }

    /// Protobuf base-128 varint. Bounded at ten bytes so a run of continuation bits cannot
    /// spin forever.
    fn varint(&mut self) -> Option<u64> {
        let mut value: u64 = 0;
        for shift in 0..10 {
            let byte = *self.bytes.get(self.at)?;
            self.at += 1;
            value |= u64::from(byte & 0x7F) << (shift * 7);
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let slice = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(slice)
    }

    /// Skip a field whose contents we do not need.
    fn skip(&mut self, wire: u64) -> Option<()> {
        match wire {
            WIRE_VARINT => self.varint().map(|_| ()),
            WIRE_64BIT => self.take(8).map(|_| ()),
            WIRE_LEN => {
                let n = usize::try_from(self.varint()?).ok()?;
                self.take(n).map(|_| ())
            }
            WIRE_32BIT => self.take(4).map(|_| ()),
            _ => None,
        }
    }
}

/// Follow `path` through nested length-delimited fields and return the bytes at the end.
fn field_at<'a>(bytes: &'a [u8], path: &[u64]) -> Option<&'a [u8]> {
    let Some((&wanted, rest)) = path.split_first() else {
        return Some(bytes);
    };
    let mut reader = Reader::new(bytes);
    while !reader.done() {
        let key = reader.varint()?;
        let (field, wire) = (key >> 3, key & 7);
        if field == wanted && wire == WIRE_LEN {
            let n = usize::try_from(reader.varint()?).ok()?;
            let chunk = reader.take(n)?;
            // The first match that resolves wins; a later sibling with the same tag is a
            // different run of the document, not the text.
            if let Some(found) = field_at(chunk, rest) {
                return Some(found);
            }
        } else {
            reader.skip(wire)?;
        }
    }
    None
}

/// Decompress a CloudKit document payload.
///
/// Apple uses more than one wrapper. Most payloads are gzip, but some are raw zlib and some
/// are not compressed at all — a note body arriving as zlib decoded to nothing for as long as
/// only gzip was handled, which looked like an empty note rather than a decoder gap.
fn inflate(compressed: &[u8]) -> Option<Vec<u8>> {
    if compressed.len() < 2 {
        return Some(compressed.to_vec());
    }

    // gzip
    if compressed[0] == 0x1f && compressed[1] == 0x8b {
        return crate::bounded::gunzip(compressed, MAX_DECOMPRESSED);
    }

    // zlib: low nibble of the first byte is the method (8 = deflate), and the two-byte
    // header is a multiple of 31.
    let header = u16::from(compressed[0]) << 8 | u16::from(compressed[1]);
    if compressed[0] & 0x0f == 8 && header % 31 == 0 {
        if let Some(out) = crate::bounded::unzlib(compressed, MAX_DECOMPRESSED) {
            return Some(out);
        }
    }

    // Not compressed: titles and folder names arrive as bare bytes.
    Some(compressed.to_vec())
}

/// The readable text of a CloudKit document field, or `None` if there is none to find.
///
/// Takes the base64 string CloudKit puts in the field's `value`.
#[must_use]
pub fn text_from_base64(value: &str) -> Option<String> {
    let raw = B64.decode(value.trim()).ok()?;
    text_from_bytes(&raw)
}

#[must_use]
pub fn text_from_bytes(raw: &[u8]) -> Option<String> {
    let plain = inflate(raw)?;

    // Reminder titles and note bodies are wrapped in Apple's document protobuf — whole (a
    // Document holding a Version holding the String), or as a bare Version or a bare String,
    // which Apple's records hold too (timlaing/pyicloud CHANGELOG 2026.10.1, "documents that
    // are wrapped in a bare CloudKit Version"). Read only whole, a reminder titled the other
    // way showed with no title. Text never begins with the bytes these wrappers begin with.
    for path in [TEXT_PATH, &TEXT_PATH[1..], &TEXT_PATH[2..]] {
        if let Some(text) = field_at(&plain, path).and_then(usable_text) {
            return Some(text);
        }
    }

    // Note and folder titles are not wrapped at all — the payload is the text. Falling back
    // rather than failing is what lets one decoder serve both.
    usable_text(&plain)
}

/// The document's text exactly as it is stored — what an edit is made to, unlike
/// [`text_from_base64`], which declines text with unusual characters in it. `None` only for
/// a document that is not one.
#[must_use]
pub fn stored_text_from_base64(value: &str) -> Option<String> {
    let raw = B64.decode(value.trim()).ok()?;
    let plain = inflate(&raw)?;
    std::str::from_utf8(field_at(&plain, TEXT_PATH)?)
        .ok()
        .map(ToString::to_string)
}

/// A reminder's title as a bare `Version` (field 3, the topotext `String`, its text at 2)
/// and as a bare `String`, read like a whole document.
#[cfg(test)]
mod bare_wrapper_tests {
    use super::*;

    fn len(field: u64, inner: &[u8]) -> Vec<u8> {
        let mut out = vec![u8::try_from(field << 3 | 2).unwrap()];
        out.push(u8::try_from(inner.len()).unwrap());
        out.extend_from_slice(inner);
        out
    }

    /// Formatting carried from a note with none (emptied, then typed into) is plain text over
    /// the new text — never a panic, which left the note's save stuck and its edit replayed
    /// over the editor at every retry.
    #[test]
    fn carrying_no_formatting_is_plain_text_never_a_panic() {
        let carried = carry_runs("", &[], "Fresh start");
        assert_eq!(carried.len(), 1);
        assert_eq!(carried[0].length, 11);
        assert_eq!(
            carried[0],
            Run {
                length: 11,
                ..Run::default()
            }
        );
        assert!(carry_runs("", &[], "").is_empty());
        assert_eq!(carry_runs("ab", &[], "abc")[0].length, 3);
    }

    /// A stray control character in a note (a DEL pasted in with other text) is the text as
    /// stored; the soundness check reads it so. It read it with the display reader, which
    /// declines such text, and refused every save of the note as "an attachment referenced
    /// twice".
    #[test]
    fn a_control_character_never_makes_a_sound_edit_unsound() {
        let original = text_to_base64("Plans\nmove more");
        let edited = text_to_base64("Plans\nmove\u{7f} more");
        assert!(attachments_sound_in_base64(&original, &edited));
    }

    #[test]
    fn a_title_in_a_bare_version_or_string_reads() {
        let string = len(2, "Pick up the cake".as_bytes());
        let version = [vec![0x08, 0x00, 0x10, 0x00], len(3, &string)].concat();
        let document = [vec![0x08, 0x00], len(2, &version)].concat();
        for (label, bytes) in [
            ("whole", &document),
            ("version", &version),
            ("string", &string),
        ] {
            assert_eq!(
                text_from_bytes(&gzip(bytes)).as_deref(),
                Some("Pick up the cake"),
                "{label}"
            );
        }
        // Plain text is still plain text.
        assert_eq!(text_from_bytes(b"Groceries").as_deref(), Some("Groceries"));
    }
}

/// Valid, non-empty UTF-8 with nothing unprintable in it.
///
/// The printability check is what stops a protobuf being mistaken for text: raw wire bytes
/// occasionally decode as UTF-8 but are full of control characters.
fn usable_text(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return None;
    }
    // Trimmed to decide whether this is text at all, never to decide what the text is.
    // Handing back the trimmed version lost every blank line a note opened or ended with —
    // and worse, an edit then wrote the shortened version back, so opening a note in this
    // window and typing one character deleted the spacing above it for good. It also made a
    // note whose first line is blank look titled here and untitled on the phone, because the
    // phone takes the first line exactly as it finds it.
    (!text.trim().is_empty()).then(|| text.to_string())
}

/// Write `text` back in the shape Apple reads: protobuf at field path 2 → 3 → 2, gzipped,
/// base64-encoded.
///
/// Writes must speak the dialect reads decode, or a reminder created here would show up
/// blank on the phone.
#[must_use]
pub fn text_to_base64(text: &str) -> String {
    B64.encode(gzip(&document_bytes(text)))
}

/// Encode text the way Apple stores a note's title and snippet: plain UTF-8, base64 — no
/// compression and no document wrapper.
///
/// Measured on a real account: every title and snippet a phone or icloud.com wrote is plain;
/// every one this app wrote gzipped showed on icloud.com as the gzip's bytes — "▒▒▒KNU(…" —
/// in the note list. Writing the wrapped form would put protobuf framing there instead.
#[must_use]
pub fn bare_text_to_base64(text: &str) -> String {
    B64.encode(text.as_bytes())
}

/// A length-delimited protobuf field: `(tag << 3) | 2`, then the length, then the payload.
fn length_delimited(tag: u64, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 4);
    write_varint((tag << 3) | WIRE_LEN, &mut out);
    write_varint(payload.len() as u64, &mut out);
    out.extend_from_slice(payload);
    out
}

fn write_varint(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            return;
        }
    }
}

/// Which field of the note holds an attribute run.
const ATTRIBUTE_RUN: u64 = 5;

/// Records inside the note saying which writer produced which stretch of text.
///
/// Measured against the text that was there when they were written, so an edit makes them
/// false. See the comment where they are dropped.
const PROVENANCE: u64 = 3;

/// The list of writers the records' ids refer to.
const WRITERS: u64 = 4;

/// A protobuf varint field: `(tag << 3) | 0`, then the value.
fn varint_field(tag: u64, value: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    write_varint(tag << 3, &mut out);
    write_varint(value, &mut out);
    out
}

/// The length Apple's attribute runs are measured in.
///
/// Not bytes and not characters: UTF-16 code units, which is what an emoji makes differ
/// from both. Measured against real notes, where the runs summed to exactly this.
fn utf16_len(text: &str) -> u64 {
    text.encode_utf16().count() as u64
}

/// The document Apple accepts: version, then a document, then a note holding the text and an
/// attribute run covering it.
///
/// Two things had to be right and only one of them was.
///
/// The attribute run is how Apple's clients map styling onto text, and something written with
/// no run has nothing to render against. A bare `{ length }` is a complete run, which is what
/// Apple writes for unstyled text.
///
/// The version fields are the other half, and they were simply missing. Every document the
/// phone writes opens `08 00` — a version at the top — and its inner document opens `08 00
/// 10 00` before the note itself. Ours went straight to the note. Our own reader did not mind
/// and handed the text back perfectly, which is why this survived so long; Apple's reader is
/// stricter, and a reminder or note created here arrived on the phone with nothing in it.
///
/// Checked by laying our bytes beside a reminder the phone wrote: the same fields, in the same
/// order, differing only in the text.
fn document_bytes(text: &str) -> Vec<u8> {
    let mut note = length_delimited(TEXT_PATH[2], text.as_bytes());
    // The records a phone rebuilds the text from. Without them the text is only a cache,
    // and a phone shows a note with nothing in it. See `topotext`.
    note.extend(crate::topotext::Topo::fresh(utf16_len(text)).encode());
    note.extend(length_delimited(
        ATTRIBUTE_RUN,
        &varint_field(1, utf16_len(text)),
    ));

    // The note, inside a document that states its versions, inside a store that states its own.
    let mut document = varint_field(1, 0);
    document.extend(varint_field(2, 0));
    document.extend(length_delimited(TEXT_PATH[1], &note));

    let mut store = varint_field(1, 0);
    store.extend(length_delimited(TEXT_PATH[0], &document));
    store
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    // Writing to an in-memory buffer cannot fail.
    let _ = encoder.write_all(bytes);
    encoder.finish().unwrap_or_default()
}

/// What a text edit did, in the UTF-16 units attribute runs are measured in.
///
/// Derived from the common prefix and suffix rather than a full diff: an edit is almost
/// always one contiguous change, and this describes exactly that — `removed` units at
/// `from` became `inserted` units.
fn utf16_change(old: &str, new: &str) -> (u64, u64, u64) {
    let old: Vec<u16> = old.encode_utf16().collect();
    let new: Vec<u16> = new.encode_utf16().collect();

    let prefix = old
        .iter()
        .zip(&new)
        .take_while(|(a, b)| a == b)
        .count()
        .min(old.len())
        .min(new.len());

    // The suffix may not run back past the prefix on either side.
    let max_suffix = (old.len() - prefix).min(new.len() - prefix);
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(max_suffix);

    (
        prefix as u64,
        (old.len() - prefix - suffix) as u64,
        (new.len() - prefix - suffix) as u64,
    )
}

/// The edits that turn `old` into `new`, each `(from, removed, inserted)` in UTF-16 units of
/// `old`, in order. Text both have in common between two edits is not part of either: an edit
/// above a table and another below it are two edits, and the table between them — and every
/// character between, with its merge record and its formatting — stays as it was. Measured
/// as one stretch from the first difference to the last, the table's place in the text came
/// back as plain text and the unchanged middle as deleted and typed again.
///
/// Compared character by character, so no edit splits a surrogate pair; a stretch too large
/// to compare cheaply is one edit, as before.
fn utf16_hunks(old: &str, new: &str) -> Vec<(u64, u64, u64)> {
    let a: Vec<char> = old.chars().collect();
    let b: Vec<char> = new.chars().collect();
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let max_suf = a.len().min(b.len()) - pre;
    let suf = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(max_suf)
        .take_while(|(x, y)| x == y)
        .count();
    let (am, bm) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    // UTF-16 offset of each character of `old`.
    let mut at = Vec::with_capacity(a.len() + 1);
    let mut n = 0u64;
    for c in &a {
        at.push(n);
        n += c.len_utf16() as u64;
    }
    at.push(n);
    let units = |cs: &[char]| cs.iter().map(|c| c.len_utf16() as u64).sum::<u64>();
    if am.is_empty() && bm.is_empty() {
        return Vec::new();
    }
    if am.len().saturating_mul(bm.len()) > 4_000_000 || am.is_empty() || bm.is_empty() {
        return vec![(at[pre], units(am), units(bm))];
    }
    let w = bm.len() + 1;
    let mut lcs = vec![0u32; (am.len() + 1) * w];
    for i in (0..am.len()).rev() {
        for j in (0..bm.len()).rev() {
            lcs[i * w + j] = if am[i] == bm[j] {
                lcs[(i + 1) * w + j + 1] + 1
            } else {
                lcs[(i + 1) * w + j].max(lcs[i * w + j + 1])
            };
        }
    }
    let mut hunks: Vec<(u64, u64, u64)> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    let mut open: Option<(u64, u64, u64)> = None;
    while i < am.len() || j < bm.len() {
        if i < am.len() && j < bm.len() && am[i] == bm[j] {
            if let Some(h) = open.take() {
                hunks.push(h);
            }
            i += 1;
            j += 1;
        } else if j < bm.len() && (i == am.len() || lcs[i * w + j + 1] >= lcs[(i + 1) * w + j]) {
            let h = open.get_or_insert((at[pre + i], 0, 0));
            h.2 += bm[j].len_utf16() as u64;
            j += 1;
        } else {
            let h = open.get_or_insert((at[pre + i], 0, 0));
            h.1 += am[i].len_utf16() as u64;
            i += 1;
        }
    }
    if let Some(h) = open {
        hunks.push(h);
    }
    hunks
}

/// The field of a run that says it holds an attachment — a table, a picture — by id.
const ATTACHMENT_INFO: u64 = 12;

/// A note's runs after one edit — `removed` units at `from` replaced by `inserted` — each
/// keeping its own bytes.
///
/// Every run is matched to itself, not to its place in the list: a run the edit removes
/// entirely is dropped without the ones after it moving onto the next run's styling. New text
/// joins the run it extends, as typing does — never an attachment's run, which holds its one
/// placeholder character and nothing else; typed beside a table, it goes into the run on its
/// other side, or a plain run of its own. Then, as before, a run set that did not add up is
/// put right at the tail rather than flattened.
fn rebuild_runs(
    runs: &[Vec<u8>],
    from: u64,
    removed: u64,
    inserted: u64,
    wanted: u64,
) -> Option<Vec<Vec<u8>>> {
    let is_attachment = |run: &[u8]| bytes_field(run, ATTACHMENT_INFO).is_some();
    let removal_end = from + removed;
    // (template, length): templates are the runs' own bytes.
    let mut out: Vec<(Vec<u8>, u64)> = Vec::with_capacity(runs.len() + 1);
    let mut placed = inserted == 0;
    let mut cursor = 0u64;
    for (i, run) in runs.iter().enumerate() {
        let (start, len) = (cursor, run_length(run));
        let end = start + len;
        cursor = end;
        let overlap = removal_end.min(end).saturating_sub(from.max(start));
        let mut next = len - overlap;
        if !placed && (end > from || (end == from && start <= from)) {
            if !is_attachment(run) {
                next += inserted;
                placed = true;
            } else if from <= start {
                // Before the attachment: into the run before it, if that is plain text.
                match out.last_mut() {
                    Some((t, n)) if !is_attachment(t) => *n += inserted,
                    _ => out.push((without_field(run, ATTACHMENT_INFO)?, inserted)),
                }
                placed = true;
            } else {
                // After it: into the run after it, if it begins right there and is plain.
                out.push((run.clone(), next));
                match runs.get(i + 1) {
                    Some(after) if !is_attachment(after) => {}
                    _ => {
                        out.push((without_field(run, ATTACHMENT_INFO)?, inserted));
                        placed = true;
                    }
                }
                continue;
            }
        } else if !placed && start == from && i > 0 && is_attachment(&runs[i - 1]) {
            next += inserted;
            placed = true;
        }
        out.push((run.clone(), next));
    }
    if !placed {
        match out.last_mut() {
            Some((t, n)) if !is_attachment(t) => *n += inserted,
            Some((t, _)) => {
                let plain = without_field(t, ATTACHMENT_INFO)?;
                out.push((plain, inserted));
            }
            None => out.push((Vec::new(), inserted)),
        }
    }
    out.retain(|(_, n)| *n > 0);

    // Runs that did not add up before this edit are put right at the tail.
    let total: u64 = out.iter().map(|(_, n)| n).sum();
    if total < wanted {
        match out.last_mut() {
            Some((t, n)) if !is_attachment(t) => *n += wanted - total,
            _ => out.push((Vec::new(), wanted - total)),
        }
    } else if total > wanted {
        let mut excess = total - wanted;
        while excess > 0 {
            let Some((_, n)) = out.last_mut() else { break };
            let given = (*n).min(excess);
            *n -= given;
            excess -= given;
            if *n == 0 {
                out.pop();
            }
        }
    }
    out.into_iter()
        .map(|(t, n)| set_varint_field(&t, 1, n))
        .collect()
}

/// Whether a message carries this field at all.
fn has_field(message: &[u8], tag: u64) -> bool {
    let mut reader = Reader::new(message);
    while !reader.done() {
        let Some(key) = reader.varint() else {
            return false;
        };
        let (field, wire) = (key >> 3, key & 7);
        if field == tag {
            return true;
        }
        if reader.skip(wire).is_none() {
            return false;
        }
    }
    false
}

/// Put the version fields back where they are missing.
///
/// Documents this app wrote before it knew about them went straight to the note: no version at
/// the top, none on the document inside. Apple's reader is stricter than ours and such a
/// record arrives on a phone with nothing in it. Rewriting one is the chance to put them back,
/// so anything written the old way heals the first time it is edited rather than staying
/// broken for ever.
///
/// A version already there is left exactly as it is — this adds, it never overwrites.
fn ensure_versions(plain: &[u8]) -> Option<Vec<u8>> {
    let document = field_at(plain, &TEXT_PATH[..1])?;

    let mut fixed_document = document.to_vec();
    for tag in [2, 1] {
        if !has_field(&fixed_document, tag) {
            let mut with = varint_field(tag, 0);
            with.extend_from_slice(&fixed_document);
            fixed_document = with;
        }
    }

    let mut out = if fixed_document == document {
        plain.to_vec()
    } else {
        rewrite(plain, &TEXT_PATH[..1], &fixed_document)?
    };

    if !has_field(&out, 1) {
        let mut with = varint_field(1, 0);
        with.extend_from_slice(&out);
        out = with;
    }
    Some(out)
}

/// Replace a varint field's value, copying every other field through untouched.
fn set_varint_field(message: &[u8], tag: u64, value: u64) -> Option<Vec<u8>> {
    let mut reader = Reader::new(message);
    let mut out = Vec::with_capacity(message.len() + 8);
    let mut replaced = false;

    while !reader.done() {
        let start = reader.at;
        let key = reader.varint()?;
        let (field, wire) = (key >> 3, key & 7);
        if field == tag && wire == WIRE_VARINT {
            reader.varint()?;
            out.extend_from_slice(&varint_field(tag, value));
            replaced = true;
        } else {
            reader.skip(wire)?;
            out.extend_from_slice(message.get(start..reader.at)?);
        }
    }

    if !replaced {
        // A run with styling but no length is still a run; give it one.
        let mut with_length = varint_field(tag, value);
        with_length.extend_from_slice(&out);
        return Some(with_length);
    }
    Some(out)
}

/// Rebuild a note with new text and runs that still describe it.
///
/// Every field other than the text and the run lengths is copied through byte for byte, so
/// each run keeps whatever styling it carried — the paragraph style, the font, the
/// identifiers Apple uses to merge edits. Only the lengths move.
fn rebuild_note(note: &[u8], old_text: &str, new_text: &str) -> Option<Vec<u8>> {
    // Each edit on its own, the last first, so the offsets of those before it still hold.
    rebuild_note_with(note, old_text, new_text, &utf16_hunks(old_text, new_text))
}

/// [`rebuild_note`] with the edit's place given rather than worked out from the two texts.
/// A text of identical characters, such as a table's ordered set, says nothing about where
/// one went in or came out; the caller does.
fn rebuild_note_with(
    note: &[u8],
    old_text: &str,
    new_text: &str,
    hunks: &[(u64, u64, u64)],
) -> Option<Vec<u8>> {
    // The merge records have to say what the text now says, or the phone shows what they
    // say instead: edits made here arrived as text repeated, or lines that would not go.
    let records = match crate::topotext::Topo::read(note) {
        // The usual case: records that agree with the text, and the edit made to them the
        // way a device would make it.
        Some(mut topo) if topo.live_length() == utf16_len(old_text) => {
            for &(from, removed, inserted) in hunks.iter().rev() {
                topo.edit(from, removed, inserted);
            }
            topo
        }
        // Records that already disagree with the text — left by this app's edits before it
        // kept them. No edit can be placed in them truthfully, and replacing them wholesale
        // was tried: a phone holding characters under ids those records never had kept every
        // one of them, merged in the new ones too, and the note came back four times over
        // with conflict copies beside it. A device that holds the real records has to write
        // them first; until it does, this note is not ours to write.
        Some(_) => return None,
        // Records in a shape this does not understand are not ours to rewrite.
        None if has_field(note, PROVENANCE) => return None,
        // None at all: a document written before any of this, or a template stripped of
        // someone else's history. It gets records of its own.
        None => crate::topotext::Topo::fresh(utf16_len(new_text)),
    };

    // Collect the existing runs, whole.
    let mut runs: Vec<Vec<u8>> = Vec::new();
    let mut reader = Reader::new(note);
    while !reader.done() {
        let key = reader.varint()?;
        let (field, wire) = (key >> 3, key & 7);
        if field == ATTRIBUTE_RUN && wire == WIRE_LEN {
            let n = usize::try_from(reader.varint()?).ok()?;
            runs.push(reader.take(n)?.to_vec());
        } else {
            reader.skip(wire)?;
        }
    }

    let wanted = utf16_len(new_text);
    let mut rebuilt_runs = runs;
    let mut length = utf16_len(old_text);
    for &(from, removed, inserted) in hunks.iter().rev() {
        length = length - removed + inserted;
        rebuilt_runs = rebuild_runs(&rebuilt_runs, from, removed, inserted, length)?;
    }
    // A run set that did not add up before any of this is put right at the tail.
    if length != wanted || rebuilt_runs.iter().map(|r| run_length(r)).sum::<u64>() != wanted {
        rebuilt_runs = rebuild_runs(&rebuilt_runs, 0, 0, 0, wanted)?;
    }
    if rebuilt_runs.is_empty() && wanted > 0 {
        rebuilt_runs.push(varint_field(1, wanted));
    }

    // Now the note itself: the text replaced, the runs replaced, everything else copied.
    let mut out = Vec::with_capacity(note.len() + new_text.len());
    let mut reader = Reader::new(note);
    let mut wrote_text = false;
    let mut wrote_runs = false;

    while !reader.done() {
        let start = reader.at;
        let key = reader.varint()?;
        let (field, wire) = (key >> 3, key & 7);

        if field == TEXT_PATH[2] && wire == WIRE_LEN && !wrote_text {
            let n = usize::try_from(reader.varint()?).ok()?;
            reader.take(n)?;
            out.extend_from_slice(&length_delimited(field, new_text.as_bytes()));
            // The records follow the text, where Apple puts them.
            out.extend(records.encode());
            wrote_text = true;
        } else if field == PROVENANCE || field == WRITERS {
            // Written afresh above.
            reader.skip(wire)?;
        } else if field == ATTRIBUTE_RUN && wire == WIRE_LEN {
            let n = usize::try_from(reader.varint()?).ok()?;
            reader.take(n)?;
            // All of them go back in the place the first one held.
            if !wrote_runs {
                for run in &rebuilt_runs {
                    out.extend_from_slice(&length_delimited(ATTRIBUTE_RUN, run));
                }
                wrote_runs = true;
            }
        } else {
            reader.skip(wire)?;
            out.extend_from_slice(note.get(start..reader.at)?);
        }
    }

    if !wrote_text {
        return None;
    }
    // A note that had no runs at all still needs them now.
    if !wrote_runs {
        for run in &rebuilt_runs {
            out.extend_from_slice(&length_delimited(ATTRIBUTE_RUN, run));
        }
    }
    Some(out)
}

/// A text object rewritten as a device would edit it — the merge records edited in step and
/// every other field copied through — for one edit at a known place: `removed` characters
/// from character `at` replaced by `inserted`. The same message shape serves a note's body
/// and a Notes table's texts (its rows' and columns' ordered sets).
pub(crate) fn rewrite_text_object_at(
    object: &[u8],
    old_text: &str,
    at: usize,
    removed: usize,
    inserted: &str,
) -> Option<Vec<u8>> {
    let chars: Vec<char> = old_text.chars().collect();
    if at + removed > chars.len() {
        return None;
    }
    let units = |cs: &[char]| cs.iter().map(|c| c.len_utf16() as u64).sum::<u64>();
    let from = units(&chars[..at]);
    let gone = units(&chars[at..at + removed]);
    let mut new_text: String = chars[..at].iter().collect();
    new_text.push_str(inserted);
    new_text.extend(&chars[at + removed..]);
    rebuild_note_with(
        object,
        old_text,
        &new_text,
        &[(from, gone, utf16_len(inserted))],
    )
}

/// What to change about one paragraph's setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParagraphChange {
    /// Apple's style number: 0 title … 3 body, 4 monospaced, 100–103 the lists.
    Style(u64),
    /// Tick or untick a checklist item.
    Checked(bool),
}

/// Fields of a message, copied through, with every occurrence of `tag` taken out.
fn without_field(message: &[u8], tag: u64) -> Option<Vec<u8>> {
    let mut reader = Reader::new(message);
    let mut out = Vec::with_capacity(message.len());
    while !reader.done() {
        let start = reader.at;
        let key = reader.varint()?;
        reader.skip(key & 7)?;
        if key >> 3 != tag {
            out.extend_from_slice(message.get(start..reader.at)?);
        }
    }
    Some(out)
}

/// A length-delimited field's contents, if the message has one.
fn bytes_field(message: &[u8], tag: u64) -> Option<Vec<u8>> {
    let mut reader = Reader::new(message);
    while !reader.done() {
        let key = reader.varint()?;
        if key >> 3 == tag && key & 7 == WIRE_LEN {
            let n = usize::try_from(reader.varint()?).ok()?;
            return Some(reader.take(n)?.to_vec());
        }
        reader.skip(key & 7)?;
    }
    None
}

/// Set one paragraph — the one beginning at UTF-16 offset `start` — as `change` says, and
/// leave every other byte of the document as it was.
///
/// The text, its merge records and every run outside the paragraph are copied through. The
/// runs are split exactly at the paragraph's edges, so a style never leaks onto the line
/// above or below, and only the paragraph-style field of the runs inside is rewritten — the
/// font, links and the rest of each run go back untouched. A checklist item keeps the id it
/// had; one made a checklist here is given a fresh one, as a phone gives it.
#[must_use]
pub fn set_paragraph(
    original: &[u8],
    start: u64,
    change: ParagraphChange,
    expect: Option<&str>,
) -> Option<Vec<u8>> {
    let plain = inflate(original)?;
    let text = std::str::from_utf8(field_at(&plain, TEXT_PATH)?).ok()?;
    let units: Vec<u16> = text.encode_utf16().collect();
    let total = units.len() as u64;
    if start > total {
        return None;
    }
    // The paragraph runs through its own newline, which carries its style.
    let end = units[usize::try_from(start).ok()?..]
        .iter()
        .position(|&u| u == u16::from(b'\n'))
        .map_or(total, |i| start + i as u64 + 1);
    if end == start {
        return None;
    }
    // The line asked about has to be the line there now. If another device edited above it,
    // the same offset is a different line, and a tick or a style would land on that one.
    if let Some(expected) = expect {
        let here =
            String::from_utf16(&units[usize::try_from(start).ok()?..usize::try_from(end).ok()?])
                .ok()?;
        if here.trim_end_matches('\n') != expected {
            return None;
        }
    }

    let note = field_at(&plain, &TEXT_PATH[..2])?;
    let runs = debug_attribute_runs(original);
    if runs.iter().map(|r| run_length(r)).sum::<u64>() != total {
        return None; // runs that do not describe the text are not edited on top of
    }

    let mut rebuilt: Vec<Vec<u8>> = Vec::with_capacity(runs.len() + 2);
    let mut at = 0u64;
    for run in &runs {
        let len = run_length(run);
        let (from, to) = (at, at + len);
        at = to;
        // Cut points inside this run: the paragraph's start and end.
        let mut cuts = vec![from];
        for c in [start, end] {
            if c > from && c < to {
                cuts.push(c);
            }
        }
        cuts.push(to);
        for pair in cuts.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let mut piece = set_varint_field(run, 1, b - a)?;
            if a >= start && b <= end {
                piece = restyle(&piece, change)?;
            }
            rebuilt.push(piece);
        }
    }

    let mut out = Vec::with_capacity(note.len() + 64);
    let mut reader = Reader::new(note);
    let mut wrote = false;
    while !reader.done() {
        let begin = reader.at;
        let key = reader.varint()?;
        let (field, wire) = (key >> 3, key & 7);
        reader.skip(wire)?;
        if field == ATTRIBUTE_RUN && wire == WIRE_LEN {
            if !wrote {
                for r in &rebuilt {
                    out.extend_from_slice(&length_delimited(ATTRIBUTE_RUN, r));
                }
                wrote = true;
            }
        } else {
            out.extend_from_slice(note.get(begin..reader.at)?);
        }
    }
    if !wrote {
        return None;
    }
    ensure_versions(&rewrite(&plain, &TEXT_PATH[..2], &out)?)
}

/// One run's paragraph style, changed.
fn restyle(run: &[u8], change: ParagraphChange) -> Option<Vec<u8>> {
    let paragraph = bytes_field(run, 2).unwrap_or_default();
    let next = match change {
        ParagraphChange::Style(style) => {
            let mut p = set_varint_field(&paragraph, 1, style)?;
            if style == 103 {
                if bytes_field(&p, 5).is_none() {
                    let mut item = length_delimited(1, uuid::Uuid::new_v4().as_bytes());
                    item.extend(varint_field(2, 0));
                    p.extend(length_delimited(5, &item));
                }
            } else {
                p = without_field(&p, 5)?;
            }
            p
        }
        ParagraphChange::Checked(done) => {
            let item = bytes_field(&paragraph, 5)?; // only a checklist item has a tick
            let item = set_varint_field(&item, 2, u64::from(done))?;
            // In its place: every other field of the paragraph as it was, where it was.
            rewrite(&paragraph, &[5], &item)?
        }
    };
    // In its place too, so whatever else the run holds — fields this app does not know, a
    // newer phone's — stays as it was, in the order it was written; appended only where the
    // run had no paragraph yet.
    match rewrite(run, &[2], &next) {
        Some(out) => Some(out),
        None => {
            let mut out = run.to_vec();
            out.extend(length_delimited(2, &next));
            Some(out)
        }
    }
}

/// Set a note's formatting from `runs` — what an editor shows — over a document whose text
/// is already the text they describe.
///
/// Each stretch keeps the rest of the run it falls in as Apple wrote it — fonts, colours,
/// attachments, anything this does not model — and only the fields an editor sets are
/// written: the paragraph's style, indent and checklist tick, bold and italic, underline,
/// strikethrough and the link. A paragraph's setting is the one its newline carries, laid
/// over the whole line, so a style never covers half a line; a checklist item keeps the id
/// it had, and one new here — or a line split off another — is given its own.
#[must_use]
pub fn set_runs(original: &[u8], runs: &[Run]) -> Option<Vec<u8>> {
    let plain = inflate(original)?;
    let text = std::str::from_utf8(field_at(&plain, TEXT_PATH)?).ok()?;
    let units: Vec<u16> = text.encode_utf16().collect();
    let total = units.len() as u64;
    if runs.iter().map(|r| r.length).sum::<u64>() != total {
        return None;
    }
    let note = field_at(&plain, &TEXT_PATH[..2])?;
    let old = debug_attribute_runs(original);
    if old.iter().map(|r| run_length(r)).sum::<u64>() != total {
        return None;
    }

    // Where each paragraph ends (just past its newline, or the end of the text).
    let mut ends: Vec<u64> = units
        .iter()
        .enumerate()
        .filter(|(_, &u)| u == u16::from(b'\n'))
        .map(|(i, _)| i as u64 + 1)
        .collect();
    if ends.last() != Some(&total) {
        ends.push(total);
    }
    let run_at = |list: &[Run], at: u64| -> Option<Run> {
        let mut from = 0;
        for r in list {
            if at < from + r.length {
                return Some(r.clone());
            }
            from += r.length;
        }
        list.last().cloned()
    };

    // Every point anything changes: old runs, new runs, paragraph ends.
    let mut cuts: Vec<u64> = vec![0, total];
    let mut at = 0;
    for r in &old {
        at += run_length(r);
        cuts.push(at);
    }
    at = 0;
    for r in runs {
        at += r.length;
        cuts.push(at);
    }
    cuts.extend(ends.iter().copied());
    cuts.sort_unstable();
    cuts.dedup();

    let mut pieces: Vec<Vec<u8>> = Vec::new();
    let mut seen_items: Vec<Vec<u8>> = Vec::new();
    let mut paragraph_item: Option<(u64, Vec<u8>)> = None;
    for pair in cuts.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if a == b {
            continue;
        }
        // The old run this stretch lies in: everything not ours to set comes from it.
        let mut from = 0;
        let template = old
            .iter()
            .find(|r| {
                let hit = a < from + run_length(r);
                from += run_length(r);
                hit
            })
            .cloned()
            .unwrap_or_default();
        let inline = run_at(runs, a)?;
        let end = *ends.iter().find(|&&e| e > a)?;
        let para = run_at(runs, end - 1)?;

        let mut piece = set_varint_field(&template, 1, b - a)?;
        // The paragraph: its style, indent and checklist item.
        let mut p = bytes_field(&template, 2).unwrap_or_default();
        p = match para.style {
            Some(style) => set_varint_field(&p, 1, style)?,
            None => without_field(&p, 1)?,
        };
        p = if para.indent > 0 {
            set_varint_field(&p, 4, para.indent)?
        } else {
            without_field(&p, 4)?
        };
        // A block quote line, or not.
        p = if para.quote {
            set_varint_field(&p, 8, 1)?
        } else {
            without_field(&p, 8)?
        };
        // Where a numbered list starts, on its line only, as Notes keeps it.
        p = match para.start {
            Some(n) if para.style == Some(102) && n != 1 => set_varint_field(&p, 7, n)?,
            _ => without_field(&p, 7)?,
        };
        if para.style == Some(103) {
            let item = match &paragraph_item {
                Some((e, item)) if *e == end => item.clone(),
                _ => {
                    // The id this line had, unless a line before it already took it.
                    let had = bytes_field(&p, 5)
                        .and_then(|i| bytes_field(&i, 1))
                        .filter(|id| !seen_items.contains(id));
                    let id = had.unwrap_or_else(|| uuid::Uuid::new_v4().as_bytes().to_vec());
                    seen_items.push(id.clone());
                    let mut item = length_delimited(1, &id);
                    item.extend(varint_field(2, u64::from(para.checked.unwrap_or(false))));
                    paragraph_item = Some((end, item.clone()));
                    item
                }
            };
            p = without_field(&p, 5)?;
            p.extend(length_delimited(5, &item));
        } else {
            p = without_field(&p, 5)?;
        }
        piece = without_field(&piece, 2)?;
        if !p.is_empty() {
            piece.extend(length_delimited(2, &p));
        }
        // The characters: bold and italic, underline, strikethrough, link.
        let hints = u64::from(inline.bold) | (u64::from(inline.italic) << 1);
        piece = if hints > 0 {
            set_varint_field(&piece, 5, hints)?
        } else {
            without_field(&piece, 5)?
        };
        piece = if inline.underline {
            set_varint_field(&piece, 6, 1)?
        } else {
            without_field(&piece, 6)?
        };
        piece = if inline.strike {
            set_varint_field(&piece, 7, 1)?
        } else {
            without_field(&piece, 7)?
        };
        // The text's colour as the screen has it, so a save never drops one.
        piece = match inline.emphasis {
            Some(e) => set_varint_field(&piece, 14, e)?,
            None => without_field(&piece, 14)?,
        };
        piece = without_field(&piece, 9)?;
        if let Some(link) = inline.link.as_deref().filter(|l| !l.is_empty()) {
            piece.extend(length_delimited(9, link.as_bytes()));
        }
        // Joined to the run before when nothing but the length tells them apart.
        if let Some(last) = pieces.last_mut() {
            if without_field(last, 1)? == without_field(&piece, 1)? {
                let joined = run_length(last) + (b - a);
                *last = set_varint_field(last, 1, joined)?;
                continue;
            }
        }
        pieces.push(piece);
    }

    // An attachment has one place in one note. Notes on a phone lagged and crashed over a note
    // this app had saved with a table referenced twice and references to deleted tables, so
    // nothing is written unless every reference is one the note already had, used once, on a
    // single attachment character.
    attachments_sound(&old, &pieces, &units)?;

    let mut out = Vec::with_capacity(note.len() + 64);
    let mut reader = Reader::new(note);
    let mut wrote = false;
    while !reader.done() {
        let begin = reader.at;
        let key = reader.varint()?;
        let (field, wire) = (key >> 3, key & 7);
        reader.skip(wire)?;
        if field == ATTRIBUTE_RUN && wire == WIRE_LEN {
            if !wrote {
                for r in &pieces {
                    out.extend_from_slice(&length_delimited(ATTRIBUTE_RUN, r));
                }
                wrote = true;
            }
        } else {
            out.extend_from_slice(note.get(begin..reader.at)?);
        }
    }
    if !wrote {
        for r in &pieces {
            out.extend_from_slice(&length_delimited(ATTRIBUTE_RUN, r));
        }
    }
    ensure_versions(&rewrite(&plain, &TEXT_PATH[..2], &out)?)
}

/// `Some(())` when the attachment references in `new` runs (over `units`) are sound against
/// `old`: each one already in `old`, none used twice, and each on a run of exactly one
/// object-replacement character.
fn attachments_sound(old: &[Vec<u8>], new: &[Vec<u8>], units: &[u16]) -> Option<()> {
    attachments_sound_with(old, new, units, None)
}

/// [`attachments_sound`], with one attachment `added` allowed that `old` did not have — a
/// fresh id, never one `old` already used — which must then appear exactly once.
fn attachments_sound_with(
    old: &[Vec<u8>],
    new: &[Vec<u8>],
    units: &[u16],
    added: Option<&[u8]>,
) -> Option<()> {
    let id_of =
        |run: &[u8]| bytes_field(run, ATTACHMENT_INFO).and_then(|info| bytes_field(&info, 1));
    let mut had: std::collections::HashSet<Vec<u8>> = old.iter().filter_map(|r| id_of(r)).collect();
    if let Some(added) = added {
        if !had.insert(added.to_vec()) {
            return None;
        }
    }
    let mut used = std::collections::HashSet::new();
    let mut at = 0usize;
    for run in new {
        let length = usize::try_from(run_length(run)).ok()?;
        if let Some(id) = id_of(run) {
            if length != 1
                || units.get(at) != Some(&0xFFFC)
                || !had.contains(&id)
                || !used.insert(id)
            {
                return None;
            }
        }
        at += length;
    }
    if added.is_some_and(|added| !used.contains(added)) {
        return None;
    }
    Some(())
}

/// Whether a note document (as the CloudKit field carries it) refers to any attachment.
#[must_use]
pub fn has_attachments_in_base64(document: &str) -> bool {
    B64.decode(document.trim()).is_ok_and(|raw| {
        debug_attribute_runs(&raw)
            .iter()
            .any(|r| bytes_field(r, ATTACHMENT_INFO).is_some())
    })
}

/// Whether `edited` (a note document, as the CloudKit field carries it) is sound against
/// `original`, what Apple held: every attachment reference one `original` had, none used
/// twice, each on its own object-replacement character. Checked on every write of a note's
/// text, whichever path built it — a note this app once saved with a table referenced twice
/// made Notes on his iPhone lag and crash, and Notes kept copies of the notes it could not
/// merge.
#[must_use]
pub fn attachments_sound_in_base64(original: &str, edited: &str) -> bool {
    let (Ok(old), Ok(new)) = (B64.decode(original.trim()), B64.decode(edited.trim())) else {
        return false;
    };
    // The text exactly as stored, as the save that built `edited` read it — never the display
    // reader, which declines text holding a control character: a stray DEL pasted into a note
    // made every save of it fail this check, reported as an attachment referenced twice.
    let Some(text) = inflate(&new)
        .and_then(|plain| field_at(&plain, TEXT_PATH).map(<[u8]>::to_vec))
        .and_then(|t| String::from_utf8(t).ok())
    else {
        return false;
    };
    let units: Vec<u16> = text.encode_utf16().collect();
    attachments_sound(
        &debug_attribute_runs(&old),
        &debug_attribute_runs(&new),
        &units,
    )
    .is_some()
}

/// [`attachments_sound_in_base64`] for a write that adds one attachment: `edited` may refer
/// to `added` — an id `original` never used — and must, exactly once, on its own
/// object-replacement character; every other reference is held to the same rule as an edit.
#[must_use]
pub fn attachments_sound_adding_in_base64(original: &str, edited: &str, added: &str) -> bool {
    let (Ok(old), Ok(new)) = (B64.decode(original.trim()), B64.decode(edited.trim())) else {
        return false;
    };
    let Some(text) = inflate(&new)
        .and_then(|plain| field_at(&plain, TEXT_PATH).map(<[u8]>::to_vec))
        .and_then(|t| String::from_utf8(t).ok())
    else {
        return false;
    };
    let units: Vec<u16> = text.encode_utf16().collect();
    attachments_sound_with(
        &debug_attribute_runs(&old),
        &debug_attribute_runs(&new),
        &units,
        Some(added.as_bytes()),
    )
    .is_some()
}

/// A note's attribute runs, whole, in order.
fn runs_in(note: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut runs = Vec::new();
    let mut reader = Reader::new(note);
    while !reader.done() {
        let key = reader.varint()?;
        let (field, wire) = (key >> 3, key & 7);
        if field == ATTRIBUTE_RUN && wire == WIRE_LEN {
            let n = usize::try_from(reader.varint()?).ok()?;
            runs.push(reader.take(n)?.to_vec());
        } else {
            reader.skip(wire)?;
        }
    }
    Some(runs)
}

/// A note with its attribute runs replaced by `runs`, written where the first one stood and
/// every other field copied through.
fn with_runs(note: &[u8], runs: &[Vec<u8>]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(note.len() + 64);
    let mut reader = Reader::new(note);
    let mut wrote = false;
    while !reader.done() {
        let begin = reader.at;
        let key = reader.varint()?;
        let (field, wire) = (key >> 3, key & 7);
        reader.skip(wire)?;
        if field == ATTRIBUTE_RUN && wire == WIRE_LEN {
            if !wrote {
                for r in runs {
                    out.extend_from_slice(&length_delimited(ATTRIBUTE_RUN, r));
                }
                wrote = true;
            }
        } else {
            out.extend_from_slice(note.get(begin..reader.at)?);
        }
    }
    if !wrote {
        for r in runs {
            out.extend_from_slice(&length_delimited(ATTRIBUTE_RUN, r));
        }
    }
    Some(out)
}

/// The paragraph setting Notes gives a paragraph holding an attachment: `3` 1 and a fresh
/// paragraph uuid at `9`. Measured on every attachment his devices placed (2026-10-22, all
/// notes): each sits on a paragraph of its own — after a newline or at the start, followed by
/// a newline or the end — with this setting, most with `3` 1, the newline after it sharing it.
fn attachment_paragraph(uuid: &[u8; 16]) -> Vec<u8> {
    let mut p = varint_field(3, 1);
    p.extend(length_delimited(9, uuid));
    p
}

/// The run Notes gives an attachment's one character, as a phone writes it, fields in the
/// phone's order: `1` length 1, `2` the paragraph's setting, `12` the attachment — `1` its id,
/// `2` its type identifier — and `13` when it was placed (Unix seconds).
fn attachment_run(id: &str, uti: &str, stamp: u64, paragraph: &[u8]) -> Vec<u8> {
    let mut info = length_delimited(1, id.as_bytes());
    info.extend(length_delimited(2, uti.as_bytes()));
    let mut run = varint_field(1, 1);
    run.extend(length_delimited(2, paragraph));
    run.extend(length_delimited(ATTACHMENT_INFO, &info));
    run.extend(varint_field(13, stamp));
    run
}

/// A note with an attachment placed at UTF-16 offset `at`. See [`insert_attachment_placed`].
#[must_use]
pub fn insert_attachment(
    original: &[u8],
    at: u64,
    id: &str,
    uti: &str,
    stamp: u64,
) -> Option<Vec<u8>> {
    insert_attachment_placed(original, at, id, uti, stamp).map(|(d, _)| d)
}

/// A note with an attachment placed at UTF-16 offset `at`, as a phone places one: on a
/// paragraph of its own. One object-replacement character (U+FFFC) goes in, with a newline
/// before it when `at` is inside a line, and a newline after it unless it is the note's last
/// character; at the very end of a line it goes on the line below. Its paragraph gets the
/// setting a phone gives one ([`attachment_paragraph`]); the newline before it ends the
/// paragraph it came from, in that paragraph's setting. The merge records are edited for the
/// one insertion as a device edits them.
///
/// Everything else is copied through: the text around it, every other run, the versions.
/// Answers the document and the offset of the attachment's character in it. `None`, and
/// nothing to write, for an offset past the end or inside a surrogate pair, and for any
/// document [`replace_text`] would refuse.
#[must_use]
pub fn insert_attachment_placed(
    original: &[u8],
    at: u64,
    id: &str,
    uti: &str,
    stamp: u64,
) -> Option<(Vec<u8>, u64)> {
    let plain = inflate(original)?;
    let current = field_at(&plain, TEXT_PATH)?;
    // The same proof an edit needs: the document can be walked and put back byte for byte.
    if rewrite(&plain, TEXT_PATH, current)? != plain {
        return None;
    }
    let old_text = std::str::from_utf8(current).ok()?;
    let units: Vec<u16> = old_text.encode_utf16().collect();
    let mut place = usize::try_from(at).ok()?;
    if place > units.len()
        || units
            .get(place)
            .is_some_and(|u| (0xDC00..0xE000).contains(u))
    {
        return None;
    }
    let note = field_at(&plain, &TEXT_PATH[..2])?;
    // Runs that do not describe the text are not built on.
    let old_runs = runs_in(note)?;
    if old_runs.iter().map(|r| run_length(r)).sum::<u64>() != units.len() as u64 {
        return None;
    }
    let newline = u16::from(b'\n');
    let inside_line = |p: usize| p > 0 && units[p - 1] != newline;
    // At the end of a line that has more after it: on the next line instead, so the line's
    // own newline stays its own.
    if inside_line(place) && units.get(place) == Some(&newline) {
        place += 1;
    }
    let before = inside_line(place);
    let after = place < units.len();
    let mut inserted: Vec<u16> = Vec::with_capacity(3);
    if before {
        inserted.push(newline);
    }
    inserted.push(0xFFFC);
    if after {
        inserted.push(newline);
    }
    let n = inserted.len() as u64;
    let at = place as u64;
    let mut new_units = units.clone();
    new_units.splice(place..place, inserted.iter().copied());
    let new_text = String::from_utf16(&new_units).ok()?;
    let rebuilt = rebuild_note_with(note, old_text, &new_text, &[(at, 0, n)])?;
    let runs = runs_in(&rebuilt)?;

    // The runs the new characters get.
    let mut uuid = [0u8; 16];
    uuid.copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    let paragraph = attachment_paragraph(&uuid);
    let mut placed_runs = Vec::with_capacity(3);
    if before {
        // The newline closing the line it was put in: that line's run, one long.
        let mut end = 0u64;
        let line = old_runs.iter().find(|r| {
            end += run_length(r);
            end >= at
        })?;
        let line = without_field(line, ATTACHMENT_INFO)?;
        placed_runs.push(set_varint_field(
            &set_varint_field(&line, 1, 1)?,
            13,
            stamp,
        )?);
    }
    placed_runs.push(attachment_run(id, uti, stamp, &paragraph));
    if after {
        let mut run = varint_field(1, 1);
        run.extend(length_delimited(2, &paragraph));
        placed_runs.push(run);
    }

    let mut out = Vec::with_capacity(runs.len() + 4);
    let mut start = 0u64;
    let mut done = false;
    for run in &runs {
        let end = start + run_length(run);
        if !done && (start..end).contains(&at) {
            // The rebuild put every new character in this one run; they are given theirs.
            if end < at + n {
                return None;
            }
            if at > start {
                out.push(set_varint_field(run, 1, at - start)?);
            }
            out.extend(placed_runs.iter().cloned());
            if end > at + n {
                out.push(set_varint_field(run, 1, end - at - n)?);
            }
            done = true;
        } else {
            out.push(run.clone());
        }
        start = end;
    }
    if !done {
        return None;
    }
    let placed = with_runs(&rebuilt, &out)?;
    let document = ensure_versions(&rewrite(&plain, &TEXT_PATH[..2], &placed)?)?;
    Some((document, at + u64::from(before)))
}

/// [`insert_attachment_placed`] over the base64 a CloudKit field carries, gzipped on the way
/// back, with where the attachment's character landed.
#[must_use]
pub fn insert_attachment_in_base64(
    original: &str,
    at: u64,
    id: &str,
    uti: &str,
    stamp: u64,
) -> Option<(String, u64)> {
    let raw = B64.decode(original.trim()).ok()?;
    let (document, offset) = insert_attachment_placed(&raw, at, id, uti, stamp)?;
    Some((B64.encode(gzip(&document)), offset))
}

/// The text and its formatting together: the text replaced as [`replace_text`] replaces it,
/// then the runs set as [`set_runs`] sets them. Over the base64 a CloudKit field carries.
#[must_use]
pub fn replace_rich_in_base64(original: &str, text: &str, runs: &[Run]) -> Option<String> {
    let raw = B64.decode(original.trim()).ok()?;
    let replaced = gzip(&replace_text(&raw, text)?);
    Some(B64.encode(gzip(&set_runs(&replaced, runs)?)))
}

/// Each paragraph of `text` — its text with its newline — and the runs over it.
fn paragraphs_of(text: &str, runs: &[Run]) -> Vec<(String, Vec<Run>)> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut per: Vec<&Run> = Vec::with_capacity(units.len());
    for r in runs {
        for _ in 0..r.length {
            per.push(r);
        }
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    for i in 0..=units.len() {
        let end = i == units.len() || units[i] == u16::from(b'\n');
        if !end {
            continue;
        }
        let stop = if i < units.len() { i + 1 } else { i };
        if stop == start && i == units.len() {
            break;
        }
        let mut pruns: Vec<Run> = Vec::new();
        for k in start..stop {
            let Some(r) = per.get(k) else { break };
            match pruns.last_mut() {
                Some(last)
                    if *last
                        == Run {
                            length: last.length,
                            ..(*r).clone()
                        } =>
                {
                    last.length += 1
                }
                _ => pruns.push(Run {
                    length: 1,
                    ..(*r).clone()
                }),
            }
        }
        out.push((String::from_utf16_lossy(&units[start..stop]), pruns));
        start = stop;
    }
    out
}

/// The formatting to write: `mine` — what the editor shows over the text being saved —
/// except on any paragraph this person did not restyle, which keeps the formatting Apple
/// holds now. So a line ticked, unticked or made bold on the phone while the note was open
/// here keeps that, unless it was restyled here too.
///
/// Paragraphs are matched by their text, in order: `base` is the formatting the editor began
/// from, `theirs` Apple's now. A paragraph whose text is not found in both is mine as it is.
#[must_use]
pub fn keep_their_formatting(
    base_text: &str,
    base_runs: &[Run],
    theirs_text: &str,
    theirs_runs: &[Run],
    text: &str,
    mine: &[Run],
) -> Vec<Run> {
    let fits = |t: &str, r: &[Run]| {
        r.iter().map(|x| x.length).sum::<u64>() == t.encode_utf16().count() as u64
    };
    if !fits(base_text, base_runs) || !fits(theirs_text, theirs_runs) || !fits(text, mine) {
        return mine.to_vec();
    }
    let take = |list: Vec<(String, Vec<Run>)>| {
        let mut by: std::collections::HashMap<String, std::collections::VecDeque<Vec<Run>>> =
            std::collections::HashMap::new();
        for (t, r) in list {
            by.entry(t).or_default().push_back(r);
        }
        by
    };
    let mut base = take(paragraphs_of(base_text, base_runs));
    let mut theirs = take(paragraphs_of(theirs_text, theirs_runs));
    let mut out: Vec<Run> = Vec::new();
    for (t, my_runs) in paragraphs_of(text, mine) {
        let b = base.get_mut(&t).and_then(|q| q.pop_front());
        let th = theirs.get_mut(&t).and_then(|q| q.pop_front());
        let chosen = match (b, th) {
            (Some(b), Some(th)) if b == my_runs && th != b => th,
            _ => my_runs,
        };
        for r in chosen {
            match out.last_mut() {
                Some(last)
                    if *last
                        == Run {
                            length: last.length,
                            ..r.clone()
                        } =>
                {
                    last.length += r.length
                }
                _ => out.push(r),
            }
        }
    }
    out
}

/// `runs`, which describe `from`, laid over `to` — the same text with one stretch changed,
/// as another device's edit merged in changes it. The stretch that is new takes the setting
/// of what is just before it.
#[must_use]
pub fn carry_runs(from: &str, runs: &[Run], to: &str) -> Vec<Run> {
    // No formatting to carry (a note emptied, say): `to` is plain text, one run over it. Laid
    // index by index, an empty `runs` panicked the save that asked, and the note it was for
    // stayed "Waiting to sync", its edit put back over the editor at every retry.
    if runs.is_empty() {
        let length = to.encode_utf16().count() as u64;
        return if length == 0 {
            Vec::new()
        } else {
            vec![Run {
                length,
                ..Run::default()
            }]
        };
    }
    let (at, removed, inserted) = utf16_change(from, to);
    let mut each: Vec<usize> = Vec::new();
    for (i, r) in runs.iter().enumerate() {
        each.extend(std::iter::repeat_n(
            i,
            usize::try_from(r.length).unwrap_or(0),
        ));
    }
    let (at, removed, inserted) = (at as usize, removed as usize, inserted as usize);
    if at + removed > each.len() {
        return Vec::new();
    }
    let fill = if at > 0 {
        each[at - 1]
    } else {
        each.get(at + removed).copied().unwrap_or(0)
    };
    let mut next: Vec<usize> = each[..at].to_vec();
    next.extend(std::iter::repeat_n(fill, inserted));
    next.extend_from_slice(&each[at + removed..]);
    let mut out: Vec<Run> = Vec::new();
    for i in next {
        match out.last_mut() {
            Some(last)
                if *last
                    == Run {
                        length: last.length,
                        ..runs.get(i).cloned().unwrap_or_default()
                    } =>
            {
                last.length += 1
            }
            _ => out.push(Run {
                length: 1,
                ..runs.get(i).cloned().unwrap_or_default()
            }),
        }
    }
    out
}

/// [`set_paragraph`] over the base64 a CloudKit field carries, gzipped as Apple sends it.
#[must_use]
pub fn set_paragraph_in_base64(
    original: &str,
    start: u64,
    change: ParagraphChange,
    expect: Option<&str>,
) -> Option<String> {
    let raw = B64.decode(original.trim()).ok()?;
    Some(B64.encode(gzip(&set_paragraph(&raw, start, change, expect)?)))
}

/// Rewrite the text inside an existing document, leaving every other field exactly as it
/// was.
///
/// A note body is not just its text. Around it Apple keeps version vectors, folder links
/// and per-device merge state, and the earlier encoder — which builds the smallest document
/// Apple will read — would have replaced all of it with a bare string. Saving an edit that
/// way silently throws away everything the note knew about itself.
///
/// So this edits in place: walk the message, swap the one field that holds the text, copy
/// every other byte through untouched.
///
/// Returns `None` when the document is not one this understands well enough to rewrite,
/// which is the point. A note whose shape cannot be reproduced exactly is left alone rather
/// than flattened, and the interface says so instead of pretending the save worked.
#[must_use]
pub fn replace_text(original: &[u8], text: &str) -> Option<Vec<u8>> {
    let plain = inflate(original)?;
    let current = field_at(&plain, TEXT_PATH)?;

    // The proof, before anything is written: putting the text back exactly as it came out
    // has to reproduce the original byte for byte. If it does, every other field survived
    // the walk, and swapping the text is safe. If it does not, this document has something
    // in it this code does not model, and it is not ours to rewrite.
    if rewrite(&plain, TEXT_PATH, current)? != plain {
        return None;
    }

    // Swapping the text alone is not enough: the note's attribute runs describe how long
    // the text is, and Apple's clients map styling onto it through them. Left stale they
    // point at units that are no longer there, and the edit does not appear at all on a
    // phone — which is exactly what happened. The note is rebuilt so the runs still
    // describe the text, keeping every other byte of each run intact.
    let old_text = std::str::from_utf8(current).ok()?;
    let note = field_at(&plain, &TEXT_PATH[..2])?;
    let rebuilt_note = rebuild_note(note, old_text, text)?;
    let swapped = rewrite(&plain, &TEXT_PATH[..2], &rebuilt_note)?;
    // An edit is also the chance to put back versions a document written the old way lacks.
    ensure_versions(&swapped)
}

/// Put a document's attribute runs back in step with the text it already holds, without
/// touching a single byte of that text.
///
/// For repairing what this app's own earlier writes damaged: notes left with no runs at
/// all, or with lengths that no longer add up. Going through [`replace_text`] would mean
/// reading the text out and writing it back, and reading it out trims surrounding
/// whitespace — so a repair would quietly edit the note it was meant to be fixing. This
/// hands the existing text bytes straight back instead.
///
/// Returns `None` when the runs are already right, so a caller can tell repair from no-op.
#[must_use]
pub fn repair_runs(original: &[u8]) -> Option<Vec<u8>> {
    let plain = inflate(original)?;
    let text = field_at(&plain, TEXT_PATH)?;
    let text = std::str::from_utf8(text).ok()?;

    let covered: u64 = debug_attribute_lengths(original).iter().sum();
    let runs_right = covered == utf16_len(text);
    let versioned = ensure_versions(&plain)?;
    let versions_right = versioned == plain;
    if runs_right && versions_right {
        return None;
    }

    let note = field_at(&versioned, &TEXT_PATH[..2])?;
    // Same text in and out, so only the run lengths can move.
    let rebuilt = rebuild_note(note, text, text)?;
    rewrite(&versioned, &TEXT_PATH[..2], &rebuilt)
}

/// [`repair_runs`] over the base64 a CloudKit field carries.
#[must_use]
pub fn repair_runs_in_base64(original: &str) -> Option<String> {
    let raw = B64.decode(original.trim()).ok()?;
    Some(B64.encode(gzip(&repair_runs(&raw)?)))
}

/// A document's note message with its runs set to `runs`, raw: for a test to put in fields
/// this app does not model, as a newer phone writes them.
#[cfg(test)]
pub(crate) fn with_raw_runs_for_test(original: &str, runs: &[Vec<u8>]) -> String {
    let raw = B64.decode(original.trim()).unwrap_or_default();
    let plain = inflate(&raw).unwrap_or_default();
    let note = field_at(&plain, &TEXT_PATH[..2]).unwrap_or_default();
    let rebuilt = with_runs(note, runs).unwrap_or_default();
    B64.encode(gzip(
        &rewrite(&plain, &TEXT_PATH[..2], &rebuilt).unwrap_or_default(),
    ))
}

/// A document as a phone would have written it: its records replaced by ones from a writer
/// that is not this app.
#[cfg(test)]
pub(crate) fn add_merge_records_for_test(original: &str) -> String {
    let raw = B64.decode(original.trim()).unwrap_or_default();
    let plain = inflate(&raw).unwrap_or_default();
    let note = field_at(&plain, &TEXT_PATH[..2]).unwrap_or_default();
    let text = std::str::from_utf8(field_at(note, &TEXT_PATH[2..]).unwrap_or_default())
        .unwrap_or_default();

    let mut with = length_delimited(TEXT_PATH[2], text.as_bytes());
    with.extend(crate::topotext::Topo::fresh_by([0x50; 16], utf16_len(text)).encode());
    let mut reader = Reader::new(note);
    while !reader.done() {
        let start = reader.at;
        let Some(key) = reader.varint() else { break };
        if reader.skip(key & 7).is_none() {
            break;
        }
        if !matches!(key >> 3, 2..=4) {
            with.extend_from_slice(&note[start..reader.at]);
        }
    }
    let rebuilt = rewrite(&plain, &TEXT_PATH[..2], &with).unwrap_or(plain);
    B64.encode(gzip(&rebuilt))
}

/// How much text a document's records say it holds, when they can be read.
#[doc(hidden)]
#[must_use]
pub fn debug_records(original: &str) -> Option<u64> {
    let raw = B64.decode(original.trim()).ok()?;
    let plain = inflate(&raw)?;
    Some(crate::topotext::Topo::read(field_at(&plain, &TEXT_PATH[..2])?)?.live_length())
}

/// How many writers a document's records name.
#[cfg(test)]
pub(crate) fn debug_writers(original: &str) -> usize {
    let raw = B64.decode(original.trim()).unwrap_or_default();
    let plain = inflate(&raw).unwrap_or_default();
    field_at(&plain, &TEXT_PATH[..2])
        .and_then(crate::topotext::Topo::read)
        .map_or(0, |t| t.writer_clocks().len())
}

/// What a document says about the note it belongs to: whether it names a writer, and whether
/// it still carries a record of which stretch of text came from where.
#[doc(hidden)]
#[must_use]
pub fn debug_note_fields(original: &str) -> (bool, bool) {
    let Ok(raw) = B64.decode(original.trim()) else {
        return (false, false);
    };
    let Some(plain) = inflate(&raw) else {
        return (false, false);
    };
    let Some(note) = field_at(&plain, &TEXT_PATH[..2]) else {
        return (false, false);
    };
    let (mut history, mut writer) = (false, false);
    let mut reader = Reader::new(note);
    while !reader.done() {
        let Some(key) = reader.varint() else { break };
        match key >> 3 {
            PROVENANCE => history = true,
            4 => writer = true,
            _ => {}
        }
        if reader.skip(key & 7).is_none() {
            break;
        }
    }
    (writer, history)
}

/// A document borrowed from another note, with that note's history taken out.
///
/// The records at [`PROVENANCE`] say which writer produced which stretch of text, and their
/// lengths are measured against the text that was there. Carried onto a different note they
/// describe something that is not in it, and Apple's reader believes them: a fifteen-character
/// note carrying a longer note's records came out on a phone as the same word four times over,
/// with gaps where the arithmetic ran off the end.
///
/// The writer itself — the record at field 4 — is not history and does travel. A phone stamps
/// every note it makes with the same writer, so that much is true of a new note as well.
///
/// Returns `None` when the document cannot be read, so a caller keeps whatever it had.
#[must_use]
pub fn without_history(original: &str) -> Option<String> {
    let raw = B64.decode(original.trim()).ok()?;
    let plain = inflate(&raw)?;
    let note = field_at(&plain, &TEXT_PATH[..2])?;

    let mut kept = Vec::with_capacity(note.len());
    let mut reader = Reader::new(note);
    while !reader.done() {
        let start = reader.at;
        let key = reader.varint()?;
        let (field, wire) = (key >> 3, key & 7);
        reader.skip(wire)?;
        // Its formatting goes too: the borrowed note's first run — a checklist with that
        // note's item id, a link to its URL — became the whole new note's.
        if field != PROVENANCE && field != ATTRIBUTE_RUN {
            kept.extend_from_slice(note.get(start..reader.at)?);
        }
    }

    Some(B64.encode(gzip(&rewrite(&plain, &TEXT_PATH[..2], &kept)?)))
}

/// Whether a document carries the merge records Apple's own reader needs.
///
/// Inside the note message Apple keeps records saying which writer produced which stretch of
/// text — fields 3 and 4. A document without them reads back here perfectly and shows on a
/// phone as an empty note, so this is what tells a document worth copying from one that is
/// not.
#[must_use]
pub fn has_merge_records(original: &str) -> bool {
    let Ok(raw) = B64.decode(original.trim()) else {
        return false;
    };
    let Some(plain) = inflate(&raw) else {
        return false;
    };
    let Some(note) = field_at(&plain, &TEXT_PATH[..2]) else {
        return false;
    };

    let mut reader = Reader::new(note);
    let mut provenance = false;
    let mut writer = false;
    while !reader.done() {
        let Some(key) = reader.varint() else { break };
        match key >> 3 {
            3 => provenance = true,
            4 => writer = true,
            _ => {}
        }
        if reader.skip(key & 7).is_none() {
            break;
        }
    }
    provenance && writer
}

/// Whether [`replace_text`] would accept this document, without building the result.
///
/// Answers "can this note be edited?" at sync time, so the interface can say so before
/// someone starts typing rather than after they press save.
#[must_use]
pub fn is_rewritable(original: &[u8]) -> bool {
    let Some(plain) = inflate(original) else {
        return false;
    };
    let Some(current) = field_at(&plain, TEXT_PATH) else {
        return false;
    };
    if !rewrite(&plain, TEXT_PATH, current).is_some_and(|rebuilt| rebuilt == plain) {
        return false;
    }
    // Merge records have to be ones an edit can be made to — readable, and describing the text
    // there now — the same test a save applies.
    let text = std::str::from_utf8(current).unwrap_or_default();
    field_at(&plain, &TEXT_PATH[..2]).is_some_and(|note| {
        !has_field(note, PROVENANCE)
            || crate::topotext::Topo::read(note).is_some_and(|t| t.live_length() == utf16_len(text))
    })
}

/// One run's length, for looking at a document's runs.
#[must_use]
pub fn debug_run_length(run: &[u8]) -> u64 {
    run_length(run)
}

/// The merge graph of a note document, for looking at one that will not edit.
#[must_use]
pub fn debug_merge_graph(original: &[u8]) -> Vec<String> {
    inflate(original)
        .and_then(|plain| field_at(&plain, &TEXT_PATH[..2]).map(crate::topotext::debug_graph))
        .unwrap_or_default()
}

/// Why [`is_rewritable`] says what it says, for looking at a note that will not edit.
#[must_use]
pub fn debug_rewritable(original: &[u8]) -> String {
    let Some(plain) = inflate(original) else {
        return "does not inflate".into();
    };
    let Some(current) = field_at(&plain, TEXT_PATH) else {
        return "no text field".into();
    };
    if !rewrite(&plain, TEXT_PATH, current).is_some_and(|rebuilt| rebuilt == plain) {
        return "does not rebuild byte for byte".into();
    }
    let text = std::str::from_utf8(current).unwrap_or_default();
    let Some(note) = field_at(&plain, &TEXT_PATH[..2]) else {
        return "no note message".into();
    };
    if !has_field(note, PROVENANCE) {
        return "ok (no merge records)".into();
    }
    match crate::topotext::Topo::read(note) {
        None => "merge records unreadable".into(),
        Some(t) if t.live_length() != utf16_len(text) => format!(
            "merge records say {} units, text has {}",
            t.live_length(),
            utf16_len(text)
        ),
        Some(_) => "ok".into(),
    }
}

/// [`replace_text`] over the base64 a CloudKit field carries, gzipped on the way back.
#[must_use]
pub fn replace_text_in_base64(original: &str, text: &str) -> Option<String> {
    let raw = B64.decode(original.trim()).ok()?;
    Some(B64.encode(gzip(&replace_text(&raw, text)?)))
}

/// Copy a message through, replacing the length-delimited field at `path` with `payload`.
///
/// Every field that is not on the path is copied as raw bytes, so nothing is re-encoded and
/// nothing can be lost in translation.
fn rewrite(message: &[u8], path: &[u64], payload: &[u8]) -> Option<Vec<u8>> {
    let (&wanted, rest) = path.split_first()?;
    let mut reader = Reader::new(message);
    let mut out = Vec::with_capacity(message.len() + payload.len());
    let mut replaced = false;

    while !reader.done() {
        let start = reader.at;
        let key = reader.varint()?;
        let (field, wire) = (key >> 3, key & 7);

        if field == wanted && wire == WIRE_LEN && !replaced {
            let n = usize::try_from(reader.varint()?).ok()?;
            let chunk = reader.take(n)?;
            let inner = if rest.is_empty() {
                payload.to_vec()
            } else {
                // A sibling with the same tag that does not contain the path is a different
                // part of the document, so copy it and keep looking.
                match rewrite(chunk, rest, payload) {
                    Some(done) => done,
                    None => {
                        out.extend_from_slice(message.get(start..reader.at)?);
                        continue;
                    }
                }
            };
            out.extend_from_slice(&length_delimited(field, &inner));
            replaced = true;
        } else {
            reader.skip(wire)?;
            out.extend_from_slice(message.get(start..reader.at)?);
        }
    }

    replaced.then_some(out)
}

/// Decompress a base64 document payload without interpreting it.
///
/// For measuring what Apple actually sends before writing anything back.
#[must_use]
pub fn debug_inflate(value: &str) -> Option<Vec<u8>> {
    inflate(&B64.decode(value.trim()).ok()?)
}

/// The length an attribute run declares, or 0 if it declares none.
///
/// Scans for the field rather than assuming it comes first: protobuf does not promise an
/// order, and a run with styling written ahead of its length is still a run. Reading only
/// the first field silently dropped such runs, which made healthy notes measure short.
fn run_length(run: &[u8]) -> u64 {
    let mut reader = Reader::new(run);
    while !reader.done() {
        let Some(key) = reader.varint() else { break };
        let (field, wire) = (key >> 3, key & 7);
        if field == 1 && wire == WIRE_VARINT {
            return reader.varint().unwrap_or(0);
        }
        if reader.skip(wire).is_none() {
            break;
        }
    }
    // An omitted length is protobuf's default, which is zero — not "skip this run".
    0
}

/// The `length` of every attribute run inside a note document.
///
/// Apple's note is `NoteStoreProto{2: Document{3: Note}}`, and the Note carries its text at
/// field 2 and a repeated AttributeRun at field 5. Each run's own field 1 is a length. If
/// those lengths do not add up to the text, Apple's own clients have runs that no longer
/// describe what is there — which is what a note arriving blank on a phone looks like.
///
/// Returns the lengths in order, for comparing against the text they claim to cover.
#[must_use]
pub fn debug_attribute_lengths(bytes: &[u8]) -> Vec<u64> {
    let Some(plain) = inflate(bytes) else {
        return Vec::new();
    };
    // Down to the Note itself: NoteStoreProto -> Document -> Note is 2 -> 3.
    let Some(note) = field_at(&plain, &[2, 3]) else {
        return Vec::new();
    };

    let mut lengths = Vec::new();
    let mut reader = Reader::new(note);
    while !reader.done() {
        let Some(key) = reader.varint() else { break };
        let (field, wire) = (key >> 3, key & 7);
        if field == 5 && wire == WIRE_LEN {
            let Some(n) = reader.varint().and_then(|n| usize::try_from(n).ok()) else {
                break;
            };
            let Some(run) = reader.take(n) else { break };
            lengths.push(run_length(run));
        } else if reader.skip(wire).is_none() {
            break;
        }
    }
    lengths
}

/// The raw bytes of every attribute run in a note document, for seeing exactly what Apple
/// puts in one before writing one ourselves.
#[must_use]
pub fn debug_attribute_runs(bytes: &[u8]) -> Vec<Vec<u8>> {
    let Some(plain) = inflate(bytes) else {
        return Vec::new();
    };
    let Some(note) = field_at(&plain, &[2, 3]) else {
        return Vec::new();
    };

    let mut runs = Vec::new();
    let mut reader = Reader::new(note);
    while !reader.done() {
        let Some(key) = reader.varint() else { break };
        let (field, wire) = (key >> 3, key & 7);
        if field == 5 && wire == WIRE_LEN {
            let Some(n) = reader.varint().and_then(|n| usize::try_from(n).ok()) else {
                break;
            };
            let Some(run) = reader.take(n) else { break };
            runs.push(run.to_vec());
        } else if reader.skip(wire).is_none() {
            break;
        }
    }
    runs
}

/// One run of a note's text and how it is set, as Apple's attribute runs describe it.
///
/// Measured against notes written on a phone and on icloud.com: a run is field 5 of the
/// note message; inside it `1` is its length in UTF-16 units, `2` the paragraph style —
/// `1` its kind (0 title, 1 heading, 2 subheading, 4 monospaced, 100 bulleted, 101 dashed,
/// 102 numbered, 103 checklist; none is body), `4` its indent, `5` the checklist item with
/// `2` ticked — `5` the font hints (1 bold, 2 italic), `6` underline, `7` strikethrough and
/// `9` a link.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Run {
    pub length: u64,
    pub style: Option<u64>,
    pub indent: u64,
    pub checked: Option<bool>,
    /// The number a numbered list starts at, when not 1 (paragraph field 7,
    /// `startingListItemNumber`, set on the list's first item).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<u64>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub link: Option<String>,
    /// The attachment this run's one character stands for — a table, a picture — by id.
    /// Read only: an attachment's place is kept from Apple's own run, never written from this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<String>,
    /// Notes' text colour (iOS 18 and later), field 14: 1 purple, 2 pink, 3 orange, 4 mint,
    /// 5 blue. Measured on a note coloured on a phone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emphasis: Option<u64>,
    /// A block quote line, paragraph field 8 (Format › Block Quote; measured on a note quoted
    /// on a phone: 1 on every quoted line).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub quote: bool,
}

/// Every run of a note document, in order. Empty for anything that is not one.
#[must_use]
pub fn runs_from_base64(value: &str) -> Vec<Run> {
    let Ok(raw) = B64.decode(value.trim()) else {
        return Vec::new();
    };
    debug_attribute_runs(&raw)
        .iter()
        .filter_map(|r| parse_run(r))
        .collect()
}

/* ---- line breaks ------------------------------------------------------------------------- */

/// `text` with its line breaks as Notes on iPhone draws them: a line ends at `\n`.
///
/// Text pasted from Windows (on icloud.com, or by another app) can reach a note with `\r\n`
/// or a bare `\r` between its lines. The phone joins lines at a bare `\r`, and so did this
/// window, so each becomes `\n` — a carriage return before a newline goes, one before
/// anything else becomes the newline.
#[must_use]
pub fn apple_line_breaks(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains('\r') {
        return std::borrow::Cow::Borrowed(text);
    }
    std::borrow::Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
}

/// `runs` over `text`, moved onto [`apple_line_breaks`]`(text)`: the run that held a
/// carriage return dropped before a newline is one UTF-16 unit shorter (a run left with
/// nothing goes); one that became the newline keeps its length, as the character keeps its
/// place. Runs that do not describe `text` come back as they are.
#[must_use]
pub fn runs_with_apple_line_breaks(text: &str, runs: Vec<Run>) -> Vec<Run> {
    if !text.contains("\r\n") {
        return runs;
    }
    let units: Vec<u16> = text.encode_utf16().collect();
    if runs.iter().map(|r| r.length).sum::<u64>() != units.len() as u64 {
        return runs;
    }
    let mut out = Vec::with_capacity(runs.len());
    let mut at = 0usize;
    for mut run in runs {
        // The lengths add up to the text's, so each is a length within it.
        let end = at + usize::try_from(run.length).unwrap_or(0);
        let dropped = (at..end)
            .filter(|&i| {
                units[i] == u16::from(b'\r') && units.get(i + 1) == Some(&u16::from(b'\n'))
            })
            .count();
        run.length -= dropped as u64;
        at = end;
        if run.length > 0 {
            out.push(run);
        }
    }
    out
}

/// A note document's text as this app shows it: [`stored_text_from_base64`] with
/// [`apple_line_breaks`].
#[must_use]
pub fn shown_text_from_base64(value: &str) -> Option<String> {
    stored_text_from_base64(value).map(|t| apple_line_breaks(&t).into_owned())
}

/// A note document's runs over [`shown_text_from_base64`].
#[must_use]
pub fn shown_runs_from_base64(value: &str) -> Vec<Run> {
    let runs = runs_from_base64(value);
    match stored_text_from_base64(value) {
        Some(text) => runs_with_apple_line_breaks(&text, runs),
        None => runs,
    }
}

/// The document with its line breaks as [`apple_line_breaks`] has them, rewritten as an edit
/// to its text so its merge history and every other run stay sound — or the document as it is,
/// where it has none to change or cannot be rewritten (the write that follows refuses then,
/// as it would have). A write starts here, so the places it is given (a line, a character)
/// are places in the text this app showed.
#[must_use]
pub fn with_apple_line_breaks_in_base64(value: &str) -> std::borrow::Cow<'_, str> {
    let Some(text) = stored_text_from_base64(value).filter(|t| t.contains('\r')) else {
        return std::borrow::Cow::Borrowed(value);
    };
    match replace_text_in_base64(value, &apple_line_breaks(&text)) {
        Some(fixed) => std::borrow::Cow::Owned(fixed),
        None => std::borrow::Cow::Borrowed(value),
    }
}

fn parse_run(bytes: &[u8]) -> Option<Run> {
    let mut run = Run::default();
    let mut reader = Reader::new(bytes);
    while !reader.done() {
        let key = reader.varint()?;
        let (field, wire) = (key >> 3, key & 7);
        match (field, wire) {
            (1, WIRE_VARINT) => run.length = reader.varint()?,
            (2, WIRE_LEN) => {
                let n = usize::try_from(reader.varint()?).ok()?;
                parse_paragraph(reader.take(n)?, &mut run)?;
            }
            (5, WIRE_VARINT) => {
                let hints = reader.varint()?;
                run.bold = hints & 1 != 0;
                run.italic = hints & 2 != 0;
            }
            (6, WIRE_VARINT) => run.underline = reader.varint()? != 0,
            (7, WIRE_VARINT) => run.strike = reader.varint()? != 0,
            (9, WIRE_LEN) => {
                let n = usize::try_from(reader.varint()?).ok()?;
                run.link = std::str::from_utf8(reader.take(n)?)
                    .ok()
                    .map(ToString::to_string);
            }
            (12, WIRE_LEN) => {
                let n = usize::try_from(reader.varint()?).ok()?;
                let info = reader.take(n)?;
                run.attachment = bytes_field(info, 1).and_then(|id| String::from_utf8(id).ok());
            }
            (14, WIRE_VARINT) => run.emphasis = Some(reader.varint()?).filter(|&e| e > 0),
            _ => reader.skip(wire)?,
        }
    }
    Some(run)
}

fn parse_paragraph(bytes: &[u8], run: &mut Run) -> Option<()> {
    let mut reader = Reader::new(bytes);
    while !reader.done() {
        let key = reader.varint()?;
        let (field, wire) = (key >> 3, key & 7);
        match (field, wire) {
            (1, WIRE_VARINT) => run.style = Some(reader.varint()?),
            (4, WIRE_VARINT) => run.indent = reader.varint()?,
            (7, WIRE_VARINT) => run.start = Some(reader.varint()?).filter(|&n| n != 1),
            (8, WIRE_VARINT) => run.quote = reader.varint()? != 0,
            (5, WIRE_LEN) => {
                let n = usize::try_from(reader.varint()?).ok()?;
                let mut item = Reader::new(reader.take(n)?);
                let mut done = false;
                while !item.done() {
                    let k = item.varint()?;
                    if k >> 3 == 2 && k & 7 == WIRE_VARINT {
                        done = item.varint()? != 0;
                    } else {
                        item.skip(k & 7)?;
                    }
                }
                run.checked = Some(done);
            }
            _ => reader.skip(wire)?,
        }
    }
    Some(())
}

/// A readable outline of a protobuf message: one line per field, nested.
///
/// Rewriting a note body means preserving every field that is not the text, so the shape
/// has to be known rather than assumed. This is how it gets looked at.
#[must_use]
pub fn debug_tree(bytes: &[u8], depth_limit: usize) -> Vec<String> {
    let mut lines = Vec::new();
    walk(bytes, 1, depth_limit, &mut lines);
    lines
}

fn walk(bytes: &[u8], depth: usize, limit: usize, out: &mut Vec<String>) {
    let pad = "  ".repeat(depth + 1);
    let mut reader = Reader::new(bytes);
    while !reader.done() {
        let Some(key) = reader.varint() else { return };
        let (field, wire) = (key >> 3, key & 7);
        match wire {
            WIRE_VARINT => {
                let Some(value) = reader.varint() else { return };
                out.push(format!("{pad}{field}: varint {value}"));
            }
            WIRE_LEN => {
                let Some(len) = reader.varint().and_then(|n| usize::try_from(n).ok()) else {
                    return;
                };
                let Some(chunk) = reader.take(len) else {
                    return;
                };
                match std::str::from_utf8(chunk) {
                    // Printable UTF-8 of any length is text, and text is what a writer has
                    // to be able to find.
                    Ok(text) if usable_text(chunk).is_some() => out.push(format!(
                        "{pad}{field}: text {len}B {:?}",
                        text.chars().take(48).collect::<String>()
                    )),
                    _ if depth < limit && looks_like_message(chunk) => {
                        out.push(format!("{pad}{field}: message {len}B"));
                        walk(chunk, depth + 1, limit, out);
                    }
                    // Short opaque fields are shown as hex: at this size the bytes are the
                    // evidence, and a length alone says nothing.
                    _ if len <= 16 => out.push(format!(
                        "{pad}{field}: bytes {len}B {}",
                        chunk
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    )),
                    _ => out.push(format!("{pad}{field}: bytes {len}B")),
                }
            }
            WIRE_64BIT => {
                if reader.take(8).is_none() {
                    return;
                }
                out.push(format!("{pad}{field}: fixed64"));
            }
            WIRE_32BIT => {
                if reader.take(4).is_none() {
                    return;
                }
                out.push(format!("{pad}{field}: fixed32"));
            }
            _ => return,
        }
    }
}

/// Whether a chunk parses cleanly as a protobuf message all the way to its end.
///
/// Length-delimited bytes are ambiguous on the wire: a nested message and an opaque blob
/// look identical until you try to read one. Parsing to the exact end is the test.
fn looks_like_message(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let mut reader = Reader::new(bytes);
    while !reader.done() {
        let Some(key) = reader.varint() else {
            return false;
        };
        if key >> 3 == 0 || reader.skip(key & 7).is_none() {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {

    /// A very long note, far past the 8 MB once refused, reads whole; a payload that would
    /// unpack past the ceiling is still refused, not decompressed.
    #[test]
    fn a_very_long_note_reads_and_a_bomb_does_not() {
        use std::io::Write as _;
        let long = "All work and no play. ".repeat(600_000); // ~13 MB of text
        let encoded = text_to_base64(&long);
        assert_eq!(text_from_base64(&encoded).as_deref(), Some(long.as_str()));
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        gz.write_all(&vec![0u8; 200 * 1024 * 1024]).unwrap();
        let bomb = gz.finish().unwrap();
        assert!(
            inflate(&bomb).is_none(),
            "a payload past the ceiling was unpacked"
        );
    }
    use super::*;
    use std::io::Write;

    /// Build a length-delimited protobuf field.
    fn field(tag: u64, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut key = (tag << 3) | WIRE_LEN;
        loop {
            let mut byte = (key & 0x7F) as u8;
            key >>= 7;
            if key != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if key == 0 {
                break;
            }
        }
        let mut len = payload.len() as u64;
        loop {
            let mut byte = (len & 0x7F) as u8;
            len >>= 7;
            if len != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if len == 0 {
                break;
            }
        }
        out.extend_from_slice(payload);
        out
    }

    /// A document shaped the way Apple's is, carrying `text`.
    fn document(text: &str) -> Vec<u8> {
        field(2, &field(3, &field(2, text.as_bytes())))
    }

    fn gzipped(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn reads_text_from_a_gzipped_document() {
        let payload = gzipped(&document("Move more"));
        assert_eq!(text_from_bytes(&payload).as_deref(), Some("Move more"));
    }

    /// Real titles carry emoji and accents, so the decode must be UTF-8, not ASCII.
    #[test]
    fn keeps_emoji_and_accents_intact() {
        for text in ["🏃 Move more", "Café — déjà vu", "日本語のリマインダー"] {
            let payload = gzipped(&document(text));
            assert_eq!(text_from_bytes(&payload).as_deref(), Some(text));
        }
    }

    fn zlib(bytes: &[u8]) -> Vec<u8> {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    /// Apple compresses some payloads with zlib rather than gzip. Handling only gzip made
    /// those notes render as empty, which reads as data loss rather than a decoder gap.
    #[test]
    fn reads_a_zlib_document_as_well_as_gzip() {
        let text = "🔴 UNIT 2: STATIC & DYNAMIC SOFTWARE TESTING";
        assert_eq!(
            text_from_bytes(&zlib(&document(text))).as_deref(),
            Some(text)
        );
        assert_eq!(
            text_from_bytes(&gzipped(&document(text))).as_deref(),
            Some(text)
        );
    }

    #[test]
    fn reads_zlib_wrapped_bare_text() {
        assert_eq!(
            text_from_bytes(&zlib(b"Recently Deleted")).as_deref(),
            Some("Recently Deleted")
        );
    }

    /// Bytes that merely start with 0x78 must not be mistaken for zlib.
    #[test]
    fn a_leading_0x78_that_is_not_zlib_still_reads_as_text() {
        // 'x' is 0x78, and "xy" fails the header checksum, so it must fall through to text.
        assert_eq!(text_from_bytes(b"xy note").as_deref(), Some("xy note"));
    }

    #[test]
    fn reads_an_uncompressed_document_too() {
        assert_eq!(
            text_from_bytes(&document("plain")).as_deref(),
            Some("plain")
        );
    }

    #[test]
    fn accepts_the_base64_cloudkit_actually_sends() {
        let encoded = B64.encode(gzipped(&document("Milk")));
        assert_eq!(text_from_base64(&encoded).as_deref(), Some("Milk"));
    }

    /// Anything malformed must yield None. These bytes come off the network, so a panic here
    /// would be a crash triggered by a remote value.
    #[test]
    fn malformed_input_is_none_not_a_panic() {
        let cases: Vec<Vec<u8>> = vec![
            vec![],
            vec![0xff; 32],
            vec![0x1f, 0x8b, 0x08],       // gzip header, no body
            vec![0x12, 0xff, 0xff, 0xff], // length far beyond the buffer
            vec![0x80; 64],               // varint continuation forever
            field(2, &[0x12, 0x7f]),      // nested length overruns
            document("").to_vec(),        // present but empty
            gzipped(b"not a protobuf at all"),
        ];
        for bytes in cases {
            let _ = text_from_bytes(&bytes);
        }
        assert!(text_from_bytes(&[0x80; 64]).is_none());
        assert!(
            text_from_bytes(&document("")).is_none(),
            "empty text is not text"
        );
    }

    /// The decisive property: anything written must read back identically, or a reminder
    /// created here shows up blank on the phone.
    #[test]
    fn text_survives_a_write_then_read() {
        for text in [
            "Milk",
            "🏃 Move more",
            "Café — déjà vu",
            "日本語のリマインダー",
            "a line\nand another",
            "  leading and trailing  ",
            &"long ".repeat(400),
        ] {
            let encoded = text_to_base64(text);
            assert_eq!(
                text_from_base64(&encoded).as_deref(),
                Some(text),
                "round trip failed for {text:?}"
            );
        }
    }

    /// What we produce must be the same shape we parse out of Apple's own payloads.
    #[test]
    fn an_encoded_document_is_gzip_at_the_expected_path() {
        let raw = B64.decode(text_to_base64("Milk")).unwrap();
        assert_eq!(&raw[..3], &[0x1f, 0x8b, 0x08], "Apple's payloads are gzip");
        let plain = inflate(&raw).unwrap();
        assert_eq!(field_at(&plain, TEXT_PATH), Some(&b"Milk"[..]));
    }

    /// Note and folder titles arrive as bare UTF-8 rather than wrapped in the document
    /// protobuf, so one decoder has to serve both shapes.
    #[test]
    fn reads_bare_text_that_was_never_wrapped() {
        assert_eq!(
            text_from_bytes(b"Recently Deleted").as_deref(),
            Some("Recently Deleted")
        );
        assert_eq!(text_from_bytes(b"Notes").as_deref(), Some("Notes"));
        assert_eq!(
            text_from_bytes(&gzipped(b"Shopping")).as_deref(),
            Some("Shopping")
        );
    }

    /// The wrapped form must still win, or a document would be returned as its raw wire bytes.
    #[test]
    fn the_wrapped_form_takes_precedence() {
        let payload = gzipped(&document("Real title"));
        assert_eq!(text_from_bytes(&payload).as_deref(), Some("Real title"));
    }

    /// Binary that happens to be valid UTF-8 must not be mistaken for text.
    #[test]
    fn control_characters_mean_it_is_not_text() {
        assert!(text_from_bytes(&[0x01, 0x02, 0x03]).is_none());
        assert!(text_from_bytes(&[0x00; 16]).is_none());
    }

    #[test]
    fn a_document_without_the_text_field_is_none() {
        // Right nesting, wrong final tag — and binary enough that the plain-text fallback
        // does not rescue it either.
        let wrong = field(2, &field(3, &field(9, &[0x01, 0x02, 0x03])));
        assert!(text_from_bytes(&wrong).is_none());
    }
}

#[cfg(test)]
mod rewrite_tests {
    /// Plain runs of these lengths after an edit, as the save rebuilds them.
    fn lengths_after(lengths: &[u64], from: u64, removed: u64, inserted: u64) -> Vec<u64> {
        let total: u64 = lengths.iter().sum();
        let runs: Vec<Vec<u8>> = lengths.iter().map(|&n| varint_field(1, n)).collect();
        rebuild_runs(&runs, from, removed, inserted, total - removed + inserted)
            .unwrap()
            .iter()
            .map(|r| run_length(r))
            .collect()
    }
    /// Plain runs of these lengths made to add up to `wanted`.
    fn reconciled(lengths: Vec<u64>, wanted: u64) -> Vec<u64> {
        let runs: Vec<Vec<u8>> = lengths.iter().map(|&n| varint_field(1, n)).collect();
        rebuild_runs(&runs, 0, 0, 0, wanted)
            .unwrap()
            .iter()
            .map(|r| run_length(r))
            .collect()
    }

    use super::*;

    /// The writer id a phone put on the fixture's text.
    const PHONE: [u8; 16] = [0xAB; 16];

    /// The note, with its text, runs and merge records taken out: everything an edit has no
    /// business changing.
    fn untouched_by_an_edit(document: &[u8]) -> Vec<u8> {
        let plain = inflate(document).unwrap();
        let note = field_at(&plain, &TEXT_PATH[..2]).unwrap();
        let mut rest = Vec::new();
        let mut reader = Reader::new(note);
        while !reader.done() {
            let start = reader.at;
            let key = reader.varint().unwrap();
            reader.skip(key & 7).unwrap();
            if !matches!(key >> 3, 2..=5) {
                rest.extend_from_slice(&note[start..reader.at]);
            }
        }
        rewrite(&plain, &TEXT_PATH[..2], &rest).unwrap()
    }

    fn records(document: &[u8]) -> crate::topotext::Topo {
        let plain = inflate(document).unwrap();
        crate::topotext::Topo::read(field_at(&plain, &TEXT_PATH[..2]).unwrap()).unwrap()
    }

    fn varint(tag: u64, value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        write_varint(tag << 3, &mut out);
        write_varint(value, &mut out);
        out
    }

    /// The shape a real note body has: the text at 2 -> 3 -> 2, with version-vector
    /// messages and stray scalars around it that a save must not disturb.
    fn document(text: &str) -> Vec<u8> {
        let mut note = Vec::new();
        note.extend(length_delimited(2, text.as_bytes()));
        // Merge records exactly as a phone writes them, by a writer that is not this app.
        note.extend(crate::topotext::Topo::fresh_by(PHONE, utf16_len(text)).encode());
        // The attribute run that makes the text renderable, carrying styling alongside its
        // length exactly as Apple's own runs do.
        note.extend(length_delimited(
            5,
            &[
                varint(1, utf16_len(text)),
                length_delimited(2, &[0x4a, 0x02, 0xAA, 0xBB]),
            ]
            .concat(),
        ));

        let mut inner = Vec::new();
        inner.extend(varint(1, 0));
        inner.extend(varint(2, 0));
        inner.extend(length_delimited(3, &note));

        let mut doc = varint(1, 0);
        doc.extend(length_delimited(2, &inner));
        doc
    }

    #[test]
    fn the_text_is_swapped_and_nothing_else_moves() {
        let before = document("the original body");
        let after = replace_text(&before, "a longer body than the one it replaced").unwrap();

        assert_eq!(
            text_from_bytes(&after).as_deref(),
            Some("a longer body than the one it replaced")
        );
        // Everything that is not the text or its records is still there, byte for byte.
        assert_eq!(untouched_by_an_edit(&after), untouched_by_an_edit(&before));
        // And the records describe the new text, not the old.
        assert_eq!(
            records(&after).live_length(),
            utf16_len("a longer body than the one it replaced")
        );
    }

    /// The bug both of these fix: Apple's clients map styling onto the text through
    /// attribute runs, and a run whose length no longer matches leaves the note unrenderable
    /// — a note edited here simply never changed on the phone.
    #[test]
    fn the_runs_always_add_up_to_the_text_they_describe() {
        for (from, to) in [
            ("short", "a considerably longer body"),
            ("a considerably longer body", "short"),
            ("same size", "SAME SIZE"),
            ("plain", "with an emoji 🔴 in it"),
            ("with an emoji 🔴 in it", "plain"),
            ("anything", ""),
        ] {
            let after = replace_text(&document(from), to).unwrap();
            let lengths = debug_attribute_lengths(&after);
            let text = text_from_bytes(&after).unwrap_or_default();
            assert_eq!(
                lengths.iter().sum::<u64>(),
                text.encode_utf16().count() as u64,
                "{from:?} -> {to:?}: runs {lengths:?} must cover the text exactly"
            );
        }
    }

    /// A document written from scratch needs a run too — without one, everything created
    /// from here arrived blank.
    #[test]
    fn a_created_document_carries_a_run_covering_its_text() {
        for text in ["hello", "🔴 emoji counts as two", ""] {
            let encoded = text_to_base64(text);
            let raw = B64.decode(&encoded).unwrap();
            let lengths = debug_attribute_lengths(&raw);
            assert_eq!(
                lengths.iter().sum::<u64>(),
                text.encode_utf16().count() as u64,
                "{text:?} must be covered by its runs"
            );
            assert_eq!(text_from_base64(&encoded).as_deref().unwrap_or(""), text);
        }
    }

    /// An edit in the middle must leave the runs on either side of it alone — that is what
    /// keeps a heading still a heading after you change a word underneath it.
    #[test]
    fn an_edit_only_disturbs_the_runs_it_touches() {
        // Three runs of ten units each.
        let lengths = [10, 10, 10];
        // Replace two units in the middle run with five.
        let adjusted = lengths_after(&lengths, 14, 2, 5);
        assert_eq!(adjusted, vec![10, 13, 10], "only the middle run moves");
        assert_eq!(adjusted.iter().sum::<u64>(), 33);
    }

    #[test]
    fn a_run_left_covering_nothing_is_dropped() {
        // Remove the whole middle run.
        assert_eq!(lengths_after(&[10, 10, 10], 10, 10, 0), vec![10, 10]);
    }

    /// Repairing a note whose runs were already wrong must not cost it its formatting.
    #[test]
    fn reconciling_takes_the_difference_at_the_end() {
        // Short by 169, as one real note on the account was: 16612 covered, 16781 there.
        assert_eq!(
            reconciled(vec![10, 20, 16_582], 16_781),
            vec![10, 20, 16_751],
            "the shortfall is added to the trailing run, not spread over the styled ones"
        );
        // Over by 29, as another was: 167 claimed over 138 of text.
        assert_eq!(
            reconciled(vec![100, 37, 30], 138),
            vec![100, 37, 1],
            "the excess is given back from the end"
        );
        // Over by enough to swallow whole trailing runs.
        assert_eq!(reconciled(vec![10, 20, 30], 10), vec![10]);
        // Already right: untouched.
        assert_eq!(reconciled(vec![10, 20], 30), vec![10, 20]);
        // Nothing at all to work with.
        assert_eq!(reconciled(Vec::new(), 42), vec![42]);
        assert_eq!(reconciled(Vec::new(), 0), Vec::<u64>::new());
    }

    /// Repair must not touch the text, including whitespace a reader would trim off. This
    /// is the guarantee that makes it safe to run over someone's real notes.
    #[test]
    fn repairing_leaves_the_text_byte_for_byte() {
        // Text with whitespace at both ends, which reading it back would have trimmed.
        let text = "\n  a note with padding around it  \n";
        let mut note = length_delimited(2, text.as_bytes());
        note.extend(length_delimited(5, &varint(1, 9_999))); // a nonsense length
        let mut inner = varint(1, 0);
        inner.extend(length_delimited(3, &note));
        let mut broken = varint(1, 0);
        broken.extend(length_delimited(2, &inner));

        let repaired = repair_runs(&broken).expect("it needed repair");
        assert_eq!(
            field_at(&repaired, TEXT_PATH),
            Some(text.as_bytes()),
            "the text must come through untouched, padding and all"
        );
        assert_eq!(
            debug_attribute_lengths(&repaired).iter().sum::<u64>(),
            text.encode_utf16().count() as u64
        );
    }

    /// A note that is already correct is left completely alone, so a repair pass can tell
    /// what it actually changed.
    #[test]
    fn repairing_a_healthy_note_is_a_no_op() {
        assert!(
            repair_runs(&document("already fine")).is_none(),
            "nothing to do, and it says so"
        );
    }

    /// A note that arrived with no runs — one of ours, written before any of this was
    /// understood — is repaired simply by saving it again.
    #[test]
    fn a_note_with_no_runs_is_repaired_by_an_edit() {
        // A document with text but no attribute run at all, as our own writes produced.
        let mut note = length_delimited(2, b"some text that was never covered");
        note.extend(length_delimited(4, &length_delimited(1, &[0xAB; 4])));
        let mut inner = varint(1, 0);
        inner.extend(length_delimited(3, &note));
        let mut broken = varint(1, 0);
        broken.extend(length_delimited(2, &inner));

        assert_eq!(debug_attribute_lengths(&broken), Vec::<u64>::new());

        let text = text_from_bytes(&broken).unwrap();
        let repaired = replace_text(&broken, &text).unwrap();
        assert_eq!(
            debug_attribute_lengths(&repaired).iter().sum::<u64>(),
            text.encode_utf16().count() as u64,
            "saving it unchanged is enough to put it right"
        );
    }

    #[test]
    fn appending_extends_the_last_run() {
        assert_eq!(lengths_after(&[10, 10], 20, 0, 5), vec![10, 15]);
    }

    /// The change is described in UTF-16 units, because that is what the runs count in.
    #[test]
    fn a_change_is_measured_the_way_the_runs_measure() {
        // An emoji is one char but two UTF-16 units.
        assert_eq!(utf16_change("ab", "a🔴b"), (1, 0, 2));
        assert_eq!(utf16_change("a🔴b", "ab"), (1, 2, 0));
        assert_eq!(utf16_change("same", "same"), (4, 0, 0));
        assert_eq!(utf16_change("", "new"), (0, 0, 3));
    }

    /// The whole point of rewriting in place rather than re-encoding: an edit touches the
    /// text and the records that describe it, and nothing else.
    #[test]
    fn a_same_length_edit_changes_only_the_text_and_its_records() {
        let before = document("first");
        let after = replace_text(&before, "FIRST").unwrap();
        assert_eq!(untouched_by_an_edit(&after), untouched_by_an_edit(&before));
        assert_eq!(
            field_at(&inflate(&after).unwrap(), TEXT_PATH),
            Some(&b"FIRST"[..])
        );
        // And the run still covers it, unchanged, because the length did not change.
        assert_eq!(debug_attribute_lengths(&after), vec![5]);
    }

    /// What a phone merges on. The characters it wrote and this edit did not touch keep their
    /// ids, and its entry in the list of writers comes through as it was; what was typed here
    /// is this app's own. Rewriting the records wholesale would make the phone's copy and
    /// this one disagree about every character — which is how text came back doubled.
    #[test]
    fn a_phones_characters_keep_their_ids_through_a_save() {
        let before = document("first draft");
        let after = replace_text(&before, "first final draft").unwrap();

        let (was, now) = (records(&before), records(&after));
        let (was_ids, now_ids) = (was.live_ids(), now.live_ids());
        assert_eq!(now.live_length(), 17);
        // The list stays sorted by uuid, as Apple keeps it: the phone's number is its place.
        let writers = now.writer_clocks();
        assert!(writers.windows(2).all(|w| w[0].0 < w[1].0));
        let phone = writers.iter().position(|w| w.0 == PHONE).unwrap() as u64 + 1;
        let me = 3 - phone;
        let theirs = |ids: &[(u64, u64)]| ids.iter().map(|&(_, c)| (phone, c)).collect::<Vec<_>>();
        assert_eq!(
            now_ids[..6],
            theirs(&was_ids[..6])[..],
            "\"first \" is still the phone's"
        );
        assert_eq!(
            now_ids[12..],
            theirs(&was_ids[6..])[..],
            "\"draft\" is still the phone's"
        );
        assert!(
            now_ids[6..12].iter().all(|&(writer, _)| writer == me),
            "the typed words are this app's"
        );
        assert_eq!(writers[phone as usize - 1], (PHONE.to_vec(), vec![11, 1]));
    }

    #[test]
    fn an_empty_body_is_a_legitimate_edit() {
        let after = replace_text(&document("something"), "").unwrap();
        assert_eq!(field_at(&after, TEXT_PATH), Some(&[][..]));
    }

    /// Growing and shrinking both rewrite the enclosing lengths, which is where a naive
    /// splice corrupts the message.
    #[test]
    fn lengths_are_rewritten_in_both_directions() {
        for (from, to) in [
            ("short", &"x".repeat(5_000)[..]),
            (&"y".repeat(5_000), "tiny"),
        ] {
            let after = replace_text(&document(from), to).unwrap();
            assert_eq!(text_from_bytes(&after).as_deref(), Some(to));
            assert!(looks_like_message(&after), "still a valid message");
        }
    }

    /// A document without the path is not ours to rewrite, and saying so is the safe answer.
    #[test]
    fn a_document_we_do_not_recognise_is_refused() {
        assert!(replace_text(&varint(1, 0), "anything").is_none());
        assert!(replace_text(&[0xFF, 0xFF, 0xFF], "anything").is_none());
        assert!(replace_text(&[], "anything").is_none());
    }

    /// Real payloads arrive compressed, and a save has to survive the round trip.
    #[test]
    fn a_gzipped_document_round_trips() {
        let encoded = B64.encode(gzip(&document("before")));
        let saved = replace_text_in_base64(&encoded, "after").unwrap();
        assert_eq!(text_from_base64(&saved).as_deref(), Some("after"));
    }

    /// Unicode is measured in bytes on the wire, not characters.
    #[test]
    fn emoji_and_accents_survive() {
        let text = "🔴 UNIT 2 — Prüfung · 测试\nsecond line";
        let after = replace_text(&document("x"), text).unwrap();
        assert_eq!(text_from_bytes(&after).as_deref(), Some(text));
    }
}

/// Documents written before this app knew Apple's reader wanted a version on them.
#[cfg(test)]
mod version_tests {
    use super::*;

    /// Exactly the shape this app used to write: straight to the note, no version anywhere.
    /// Our own reader never minded, which is why it survived — the reader that mattered was
    /// Apple's, and such a record arrived on a phone with nothing in it.
    fn written_the_old_way(text: &str) -> Vec<u8> {
        let mut note = length_delimited(TEXT_PATH[2], text.as_bytes());
        note.extend(length_delimited(
            ATTRIBUTE_RUN,
            &varint_field(1, utf16_len(text)),
        ));
        let document = length_delimited(TEXT_PATH[1], &note);
        length_delimited(TEXT_PATH[0], &document)
    }

    fn shape(plain: &[u8]) -> (Vec<u64>, Vec<u64>) {
        let tags = |m: &[u8]| {
            let mut reader = Reader::new(m);
            let mut out = Vec::new();
            while !reader.done() {
                let Some(key) = reader.varint() else { break };
                out.push(key >> 3);
                if reader.skip(key & 7).is_none() {
                    break;
                }
            }
            out
        };
        let document = field_at(plain, &TEXT_PATH[..1]).unwrap_or_default();
        (tags(plain), tags(document))
    }

    /// What this app writes today is what the phone writes: a version at the top, and two on
    /// the document inside, before the note.
    #[test]
    fn a_new_document_carries_the_versions_apple_writes() {
        let (top, document) = shape(&document_bytes("Buy milk"));
        assert_eq!(top, vec![1, 2]);
        assert_eq!(document, vec![1, 2, 3]);
    }

    /// And one written the old way is put right the first time it is edited, rather than
    /// staying broken for as long as it exists.
    #[test]
    fn editing_a_document_written_the_old_way_puts_the_versions_back() {
        let old = written_the_old_way("Buy milk");
        assert_eq!(
            shape(&old),
            (vec![2], vec![3]),
            "the fixture is the old shape"
        );

        let edited = replace_text(&gzip(&old), "Buy oat milk").expect("still rewritable");
        assert_eq!(shape(&edited), (vec![1, 2], vec![1, 2, 3]));
        assert_eq!(
            text_from_bytes(&edited).as_deref(),
            Some("Buy oat milk"),
            "and the edit itself landed"
        );
    }

    /// Repair notices a missing version even where the runs are already right, because the
    /// text renders from the run and the record is read from the version.
    #[test]
    fn repair_puts_the_versions_back_on_its_own() {
        let old = written_the_old_way("Buy milk");
        let repaired = repair_runs(&gzip(&old)).expect("a missing version is worth repairing");
        assert_eq!(shape(&repaired), (vec![1, 2], vec![1, 2, 3]));
        assert_eq!(text_from_bytes(&repaired).as_deref(), Some("Buy milk"));
    }

    /// A document that is already right is left alone — repair must be able to say "nothing
    /// to do" or every sync rewrites every note.
    #[test]
    fn a_sound_document_is_not_rewritten() {
        assert_eq!(repair_runs(&gzip(&document_bytes("Buy milk"))), None);
    }

    /// A version Apple set to something other than zero is not ours to overwrite.
    #[test]
    fn an_existing_version_is_left_exactly_as_it_was() {
        let mut note = length_delimited(TEXT_PATH[2], b"Buy milk");
        note.extend(length_delimited(ATTRIBUTE_RUN, &varint_field(1, 8)));
        let mut document = varint_field(1, 7);
        document.extend(varint_field(2, 3));
        document.extend(length_delimited(TEXT_PATH[1], &note));
        let mut store = varint_field(1, 9);
        store.extend(length_delimited(TEXT_PATH[0], &document));

        let out = ensure_versions(&store).unwrap();
        assert_eq!(out, store, "nothing was missing, so nothing changed");
    }
}

/// Telling a document Apple's reader will show from one it will not.
#[cfg(test)]
mod merge_record_tests {
    use super::*;

    /// What this app builds from nothing carries records of its own that describe its text.
    /// Without them it read back here perfectly and a phone showed it as an empty note.
    #[test]
    fn a_document_built_from_nothing_has_merge_records() {
        let built = text_to_base64("Buy milk");
        assert_eq!(text_from_base64(&built).as_deref(), Some("Buy milk"));
        assert!(has_merge_records(&built));
        let raw = B64.decode(&built).unwrap();
        let plain = inflate(&raw).unwrap();
        let topo = crate::topotext::Topo::read(field_at(&plain, &TEXT_PATH[..2]).unwrap())
            .expect("records in Apple's shape");
        assert_eq!(topo.live_length(), 8);
    }

    /// The shape Apple writes: records saying which writer produced which stretch of text.
    #[test]
    fn a_document_shaped_like_apples_is_recognised() {
        let mut note = length_delimited(TEXT_PATH[2], b"Buy milk");
        // One provenance record and one writer, which is the least Apple ever writes.
        note.extend(length_delimited(3, &varint_field(5, 1)));
        note.extend(length_delimited(4, &varint_field(1, 1)));
        note.extend(length_delimited(ATTRIBUTE_RUN, &varint_field(1, 8)));

        let mut document = varint_field(1, 0);
        document.extend(varint_field(2, 0));
        document.extend(length_delimited(TEXT_PATH[1], &note));
        let mut store = varint_field(1, 0);
        store.extend(length_delimited(TEXT_PATH[0], &document));

        assert!(has_merge_records(&B64.encode(gzip(&store))));
    }

    /// Both halves are needed — provenance without a writer is not the shape.
    #[test]
    fn half_the_shape_is_not_the_shape() {
        for tag in [3, 4] {
            let mut note = length_delimited(TEXT_PATH[2], b"Buy milk");
            note.extend(length_delimited(tag, &varint_field(5, 1)));
            note.extend(length_delimited(ATTRIBUTE_RUN, &varint_field(1, 8)));
            let mut document = varint_field(1, 0);
            document.extend(varint_field(2, 0));
            document.extend(length_delimited(TEXT_PATH[1], &note));
            let mut store = varint_field(1, 0);
            store.extend(length_delimited(TEXT_PATH[0], &document));
            assert!(
                !has_merge_records(&B64.encode(gzip(&store))),
                "field {tag} alone"
            );
        }
    }

    /// Nonsense is not a document worth copying from.
    #[test]
    fn rubbish_is_not_mistaken_for_a_document() {
        assert!(!has_merge_records("not base64 at all !!"));
        assert!(!has_merge_records(""));
        assert!(!has_merge_records(&B64.encode(b"hello")));
    }

    /// The whole point: swapping the text of a document that has them keeps them, which is
    /// what makes a note written here one a phone will show.
    #[test]
    fn swapping_the_text_keeps_what_makes_it_readable() {
        let mut note = length_delimited(TEXT_PATH[2], b"Old words");
        note.extend(crate::topotext::Topo::fresh_by([7; 16], 9).encode());
        note.extend(length_delimited(ATTRIBUTE_RUN, &varint_field(1, 9)));
        let mut document = varint_field(1, 0);
        document.extend(varint_field(2, 0));
        document.extend(length_delimited(TEXT_PATH[1], &note));
        let mut store = varint_field(1, 0);
        store.extend(length_delimited(TEXT_PATH[0], &document));
        let apples = B64.encode(gzip(&store));

        let swapped = replace_text_in_base64(&apples, "Entirely new words").unwrap();
        assert_eq!(
            text_from_base64(&swapped).as_deref(),
            Some("Entirely new words")
        );
        assert!(has_merge_records(&swapped));
    }
}

/// Text comes back as it was written, including the parts that look like nothing.
#[cfg(test)]
mod whitespace_tests {
    use super::*;

    /// A note that opens with blank lines keeps them. Handing back a trimmed version was not
    /// only wrong on screen — the next edit wrote the shortened text back, so opening such a
    /// note here and typing one character deleted the spacing above it for good.
    #[test]
    fn blank_lines_a_note_opens_or_ends_with_survive() {
        for text in [
            "\n\n\nhello",
            "hello\n\n\n",
            "\n\n\nhello\n\n",
            "   indented",
            "trailing spaces   ",
        ] {
            assert_eq!(
                text_from_base64(&text_to_base64(text)).as_deref(),
                Some(text),
                "{text:?} did not survive"
            );
        }
    }

    /// And an edit to such a note only changes what was edited.
    #[test]
    fn editing_a_note_that_opens_with_blank_lines_keeps_them() {
        let original = text_to_base64("\n\n\nhello");
        let edited = replace_text_in_base64(&original, "\n\n\nhello there").unwrap();
        assert_eq!(
            text_from_base64(&edited).as_deref(),
            Some("\n\n\nhello there")
        );
    }

    /// Nothing at all is still nothing — the emptiness test is what the trim was for, and it
    /// still does that job.
    #[test]
    fn whitespace_alone_is_still_not_text() {
        assert_eq!(usable_text(b""), None);
        assert_eq!(usable_text(b"   "), None);
        assert_eq!(usable_text(b"\n\n\n"), None);
        assert_eq!(usable_text(b"\t \n"), None);
    }

    /// The run has to measure the whole text, blank lines included, or the phone renders
    /// against a length that is short.
    #[test]
    fn the_run_covers_the_blank_lines_too() {
        let encoded = text_to_base64("\n\n\nhello");
        let raw = B64.decode(encoded).unwrap();
        assert_eq!(debug_attribute_lengths(&raw), vec![8]);
    }
}

/// A note whose records disagree with its text is not written.
#[cfg(test)]
mod run_tests {
    use super::*;

    fn varint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                break;
            }
            out.push(b | 0x80);
        }
    }
    fn len_field(field: u64, body: &[u8], out: &mut Vec<u8>) {
        varint(field << 3 | 2, out);
        varint(body.len() as u64, out);
        out.extend_from_slice(body);
    }

    /// A dashed item, a ticked checklist item and a bold word, as Apple lays the runs out.
    /// A document: `text`, one run per (length, extra bytes) given.
    fn doc_with(text: &str, runs: &[(u64, Vec<u8>)]) -> Vec<u8> {
        let mut note = Vec::new();
        len_field(2, text.as_bytes(), &mut note);
        for (len, extra) in runs {
            let mut run = Vec::new();
            varint(1 << 3, &mut run);
            varint(*len, &mut run);
            run.extend_from_slice(extra);
            len_field(5, &run, &mut note);
        }
        let mut inner = Vec::new();
        len_field(3, &note, &mut inner);
        let mut doc = Vec::new();
        len_field(2, &inner, &mut doc);
        doc
    }

    fn body(len: u64) -> Run {
        Run {
            length: len,
            ..Run::default()
        }
    }

    /// Bold on one word, a line made a checklist, and an attachment's bytes kept through.
    #[test]
    fn runs_are_set_from_an_editor_and_the_rest_kept() {
        // "Buy milk\nCall\n" — the run under "Call" carries a field (12) this does not model.
        let mut unknown = Vec::new();
        len_field(12, b"attachment-ish", &mut unknown);
        let doc = doc_with("Buy milk\nCall\n", &[(9, Vec::new()), (5, unknown)]);
        let want = vec![
            Run {
                length: 3,
                bold: true,
                ..body(3)
            },
            Run {
                length: 6,
                ..body(6)
            },
            Run {
                length: 5,
                style: Some(103),
                checked: Some(true),
                ..body(5)
            },
        ];
        let out = set_runs(&doc, &want).expect("set");
        let read = runs_from_base64(&B64.encode(&out));
        assert_eq!(read.iter().map(|r| r.length).collect::<Vec<_>>(), [3, 6, 5]);
        assert!(read[0].bold && !read[1].bold);
        assert_eq!((read[2].style, read[2].checked), (Some(103), Some(true)));
        let raw = debug_attribute_runs(&out);
        assert!(
            raw[2].windows(14).any(|w| w == b"attachment-ish"),
            "the unknown field went"
        );
    }

    /// Text coloured on a phone (field 14) is read, kept through a save that leaves it, and
    /// taken off only where the editor took it off.
    #[test]
    fn text_colour_is_read_kept_and_removed() {
        let mut mint = Vec::new();
        varint(14 << 3, &mut mint);
        varint(4, &mut mint);
        let doc = doc_with("Hello world", &[(5, mint), (6, Vec::new())]);
        let read = runs_from_base64(&B64.encode(&doc));
        assert_eq!(read[0].emphasis, Some(4));
        assert_eq!(read[1].emphasis, None);
        let kept = runs_from_base64(&B64.encode(set_runs(&doc, &read).unwrap()));
        assert_eq!(kept[0].emphasis, Some(4), "a save dropped the colour");
        let plain = vec![body(11)];
        let cleared = runs_from_base64(&B64.encode(set_runs(&doc, &plain).unwrap()));
        assert!(cleared.iter().all(|r| r.emphasis.is_none()));
    }

    /// A numbered list starting at 5 (paragraph field 7) is read, kept through a save, and
    /// dropped when the line stops being numbered.
    #[test]
    fn a_numbered_lists_start_is_read_kept_and_dropped() {
        let mut para = Vec::new();
        varint(1 << 3, &mut para);
        varint(102, &mut para);
        varint(7 << 3, &mut para);
        varint(5, &mut para);
        let mut extra = Vec::new();
        len_field(2, &para, &mut extra);
        let doc = doc_with("five\n", &[(5, extra)]);
        let read = runs_from_base64(&B64.encode(&doc));
        assert_eq!((read[0].style, read[0].start), (Some(102), Some(5)));
        let kept = runs_from_base64(&B64.encode(set_runs(&doc, &read).unwrap()));
        assert_eq!(kept[0].start, Some(5), "a save dropped the start");
        let plain = vec![Run {
            style: None,
            ..read[0].clone()
        }];
        let cleared = runs_from_base64(&B64.encode(set_runs(&doc, &plain).unwrap()));
        assert_eq!(cleared[0].start, None, "a plain line kept a list's start");
    }

    /// A save never writes an attachment twice, one the note did not have, or one off its own
    /// object-replacement character: the shape that made Notes on a phone lag and crash.
    #[test]
    fn attachment_references_stay_sound() {
        let run = |len: u64, id: Option<&[u8]>| {
            let mut r = Vec::new();
            varint(1 << 3, &mut r);
            varint(len, &mut r);
            if let Some(id) = id {
                let mut info = Vec::new();
                len_field(1, id, &mut info);
                len_field(12, &info, &mut r);
            }
            r
        };
        let old = vec![run(1, Some(b"A")), run(1, None)];
        let units: Vec<u16> = "\u{FFFC}\u{FFFC}".encode_utf16().collect();
        assert!(attachments_sound(&old, &[run(1, Some(b"A")), run(1, None)], &units).is_some());
        assert!(
            attachments_sound(&old, &[run(1, Some(b"A")), run(1, Some(b"A"))], &units).is_none(),
            "twice"
        );
        assert!(
            attachments_sound(&old, &[run(1, Some(b"B")), run(1, None)], &units).is_none(),
            "not the note's"
        );
        let text: Vec<u16> = "a\u{FFFC}".encode_utf16().collect();
        assert!(
            attachments_sound(&old, &[run(1, Some(b"A")), run(1, None)], &text).is_none(),
            "off its character"
        );
    }

    /// A style is the line's: one set on half a line covers all of it, from its newline.
    #[test]
    fn a_paragraph_style_covers_its_whole_line() {
        let doc = doc_with("ab\ncd", &[(5, Vec::new())]);
        let want = vec![
            body(1),
            Run {
                length: 2,
                style: Some(1),
                ..body(2)
            },
            body(2),
        ];
        let read = runs_from_base64(&B64.encode(set_runs(&doc, &want).unwrap()));
        assert_eq!(
            read.iter().map(|r| (r.length, r.style)).collect::<Vec<_>>(),
            [(3, Some(1)), (2, None)]
        );
    }

    /// A checklist line split in two gives the second its own id, as a phone does.
    #[test]
    fn a_split_checklist_line_gets_its_own_id() {
        let mut item = Vec::new();
        len_field(1, &[7u8; 16], &mut item);
        let mut para = Vec::new();
        varint(1 << 3, &mut para);
        varint(103, &mut para);
        len_field(5, &item, &mut para);
        let mut extra = Vec::new();
        len_field(2, &para, &mut extra);
        let doc = doc_with("a\nb", &[(3, extra)]);
        let want = vec![Run {
            length: 3,
            style: Some(103),
            checked: Some(false),
            ..body(3)
        }];
        let out = set_runs(&doc, &want).unwrap();
        let ids: Vec<Vec<u8>> = debug_attribute_runs(&out)
            .iter()
            .filter_map(|r| {
                bytes_field(r, 2)
                    .and_then(|p| bytes_field(&p, 5))
                    .and_then(|i| bytes_field(&i, 1))
            })
            .collect();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], vec![7u8; 16], "the first line keeps the id it had");
        assert_ne!(ids[0], ids[1]);
    }

    /// Typing right after a table never joins the table's run: the new text is plain text in
    /// a run of its own, and the attachment's run still holds its one character.
    #[test]
    fn text_typed_beside_an_attachment_is_not_part_of_it() {
        let mut info = Vec::new();
        len_field(12, b"\x0a\x02id", &mut info);
        // The attachment's one character in a run of its own, then the newline.
        let doc = doc_with("\u{FFFC}\n", &[(1, info), (1, Vec::new())]);
        let out = replace_text(&doc, "\u{FFFC}hi\n").expect("edit");
        let raw = debug_attribute_runs(&out);
        let lens: Vec<u64> = raw.iter().map(|r| run_length(r)).collect();
        assert_eq!(lens.iter().sum::<u64>(), 4);
        let attached: Vec<u64> = raw
            .iter()
            .filter(|r| bytes_field(r, 12).is_some())
            .map(|r| run_length(r))
            .collect();
        assert_eq!(
            attached,
            [1],
            "the attachment run took typed text: {lens:?}"
        );
    }

    /// Every write of a note's text is checked against what Apple held: an attachment kept
    /// once passes; one referenced twice, or one the note never had, is refused.
    #[test]
    fn a_write_that_doubles_or_borrows_an_attachment_is_refused() {
        let info = |id: &[u8]| {
            let mut inner = Vec::new();
            len_field(1, id, &mut inner);
            let mut out = Vec::new();
            len_field(12, &inner, &mut out);
            out
        };
        let had = B64.encode(doc_with("\u{FFFC}\n", &[(1, info(b"T1")), (1, Vec::new())]));
        let kept = B64.encode(replace_text(&B64.decode(&had).unwrap(), "\u{FFFC}hi\n").unwrap());
        assert!(attachments_sound_in_base64(&had, &kept));
        let twice = B64.encode(doc_with(
            "\u{FFFC}\u{FFFC}\n",
            &[(1, info(b"T1")), (1, info(b"T1")), (1, Vec::new())],
        ));
        assert!(
            !attachments_sound_in_base64(&had, &twice),
            "a table referenced twice"
        );
        let borrowed = B64.encode(doc_with("\u{FFFC}\n", &[(1, info(b"T2")), (1, Vec::new())]));
        assert!(
            !attachments_sound_in_base64(&had, &borrowed),
            "another note's table"
        );
        let gone = B64.encode(doc_with("hi\n", &[(3, Vec::new())]));
        assert!(
            attachments_sound_in_base64(&had, &gone),
            "a table deleted is fine"
        );
        assert!(has_attachments_in_base64(&had) && !has_attachments_in_base64(&gone));
    }

    /// An edit that removes a whole run leaves every later run with its own styling.
    #[test]
    fn removing_a_whole_run_does_not_shift_the_styles_after_it() {
        let mut bold = Vec::new();
        varint(5 << 3, &mut bold);
        varint(1, &mut bold);
        let mut heading = Vec::new();
        let mut p = Vec::new();
        varint(1 << 3, &mut p);
        varint(1, &mut p);
        len_field(2, &p, &mut heading);
        // "ab" plain, "CD" bold, "ef\n" heading → delete "CD".
        let doc = doc_with("abCDef\n", &[(2, Vec::new()), (2, bold), (3, heading)]);
        let out = replace_text(&doc, "abef\n").expect("edit");
        let read = runs_from_base64(&B64.encode(&out));
        assert_eq!(
            read.iter()
                .map(|r| (r.length, r.style, r.bold))
                .collect::<Vec<_>>(),
            [(2, None, false), (3, Some(1), false)]
        );
    }

    /// One save that changes the line above a table and the line below it: two edits, and the
    /// table between keeps its place and its attachment, its run untouched.
    #[test]
    fn edits_either_side_of_a_table_leave_it_attached() {
        let mut info = Vec::new();
        len_field(12, b"\x0a\x02id", &mut info);
        let doc = doc_with(
            "Above\n\u{FFFC}\nBelow",
            &[(6, Vec::new()), (1, info), (6, Vec::new())],
        );
        let out = replace_text(&doc, "Above it\n\u{FFFC}\nBelow it").expect("edit");
        let raw = debug_attribute_runs(&out);
        let mut at = 0u64;
        let mut attached_at = None;
        for r in &raw {
            if bytes_field(r, 12).is_some() {
                attached_at = Some((at, run_length(r)));
            }
            at += run_length(r);
        }
        assert_eq!(
            attached_at,
            Some((9, 1)),
            "the table is where it was, one character, still attached"
        );
        assert_eq!(
            at,
            "Above it\n\u{FFFC}\nBelow it".encode_utf16().count() as u64
        );
    }

    #[test]
    fn a_change_in_two_places_is_two_edits() {
        assert_eq!(
            utf16_hunks("Above\nX\nBelow", "Above!\nX\nBelow!"),
            [(5, 0, 1), (13, 0, 1)]
        );
        assert_eq!(utf16_hunks("same", "same"), []);
        assert_eq!(
            utf16_hunks("a😀b", "a😀c"),
            [(3, 1, 1)],
            "counted in UTF-16 units, never splitting a pair"
        );
    }

    /// The phone ticked "Milk" while the note was open here; here, a line was typed at the
    /// end. The save keeps the phone's tick — this window never restyled that line — and a
    /// line restyled here takes this window's style.
    #[test]
    fn a_tick_made_elsewhere_survives_a_save_here() {
        let item = |len: u64, done: bool| Run {
            length: len,
            style: Some(103),
            checked: Some(done),
            ..body(len)
        };
        let base_text = "Milk\nEggs\n";
        let base = vec![item(5, false), item(5, false)];
        let theirs = vec![item(5, true), item(5, false)];
        let text = "Milk\nEggs\nJam";
        let mine = vec![
            item(5, false),
            Run {
                length: 5,
                style: Some(1),
                ..body(5)
            },
            body(3),
        ];
        let out = keep_their_formatting(base_text, &base, base_text, &theirs, text, &mine);
        let read: Vec<(u64, Option<u64>, Option<bool>)> =
            out.iter().map(|r| (r.length, r.style, r.checked)).collect();
        assert_eq!(
            read,
            [
                (5, Some(103), Some(true)),
                (5, Some(1), None),
                (3, None, None)
            ]
        );
    }

    #[test]
    fn runs_follow_a_merged_edit() {
        let runs = vec![
            Run {
                length: 3,
                bold: true,
                ..body(3)
            },
            body(4),
        ];
        // "abc defg" → another device typed "XY" after "abc".
        let out = carry_runs("abc\ndef", &runs, "abcXY\ndef");
        assert_eq!(
            out.iter().map(|r| (r.length, r.bold)).collect::<Vec<_>>(),
            [(5, true), (4, false)]
        );
    }

    #[test]
    fn a_paragraph_is_restyled_alone() {
        // One run over three lines; restyling the middle one splits it, and only it changes.
        let mut run = Vec::new();
        varint(1 << 3, &mut run);
        varint(8, &mut run);
        let mut style = Vec::new();
        varint(1 << 3, &mut style);
        varint(3, &mut style);
        len_field(2, &style, &mut run);
        let mut note = Vec::new();
        len_field(2, b"ab\ncd\nef", &mut note);
        len_field(5, &run, &mut note);
        let mut inner = Vec::new();
        len_field(3, &note, &mut inner);
        let mut doc = Vec::new();
        len_field(2, &inner, &mut doc);

        let out = set_paragraph_in_base64(
            &B64.encode(&doc),
            3,
            ParagraphChange::Style(103),
            Some("cd"),
        )
        .unwrap();
        let runs = runs_from_base64(&out);
        assert_eq!(runs.iter().map(|r| r.length).collect::<Vec<_>>(), [3, 3, 2]);
        assert_eq!(
            runs.iter().map(|r| r.style).collect::<Vec<_>>(),
            [Some(3), Some(103), Some(3)]
        );
        assert_eq!(
            runs[1].checked,
            Some(false),
            "a new checklist item starts unticked"
        );
    }

    #[test]
    fn runs_carry_their_paragraph_style_ticks_and_weight() {
        let mut dashed = Vec::new();
        varint(1 << 3, &mut dashed);
        varint(5, &mut dashed);
        let mut style = Vec::new();
        varint(1 << 3, &mut style);
        varint(101, &mut style);
        len_field(2, &style, &mut dashed);

        let mut ticked = Vec::new();
        varint(1 << 3, &mut ticked);
        varint(4, &mut ticked);
        let mut item = Vec::new();
        len_field(1, b"0123456789abcdef", &mut item);
        varint(2 << 3, &mut item);
        varint(1, &mut item);
        let mut style = Vec::new();
        varint(1 << 3, &mut style);
        varint(103, &mut style);
        len_field(5, &item, &mut style);
        len_field(2, &style, &mut ticked);

        let mut bold = Vec::new();
        varint(1 << 3, &mut bold);
        varint(2, &mut bold);
        varint(5 << 3, &mut bold);
        varint(1, &mut bold);

        let mut note = Vec::new();
        len_field(2, b"milk\negg\nhi", &mut note);
        for r in [&dashed, &ticked, &bold] {
            len_field(5, r, &mut note);
        }
        let mut doc = Vec::new();
        let mut inner = Vec::new();
        len_field(3, &note, &mut inner);
        len_field(2, &inner, &mut doc);

        let runs = runs_from_base64(&B64.encode(&doc));
        assert_eq!(runs.len(), 3);
        assert_eq!((runs[0].length, runs[0].style), (5, Some(101)));
        assert_eq!((runs[1].style, runs[1].checked), (Some(103), Some(true)));
        assert!(runs[2].bold && runs[2].style.is_none());

        // Untick "egg" (the paragraph starting at 5): only its tick changes.
        let unticked = set_paragraph_in_base64(
            &B64.encode(&doc),
            5,
            ParagraphChange::Checked(false),
            Some("egg"),
        )
        .unwrap();
        let after = runs_from_base64(&unticked);
        assert_eq!(after[1].checked, Some(false));
        assert_eq!(after[0].style, Some(101), "the line above is untouched");
        assert_eq!(
            text_from_base64(&unticked).as_deref(),
            Some("milk\negg\nhi"),
            "the text is untouched"
        );

        // Make "hi" — inside a run of its own, no newline — a heading: only it changes.
        let headed =
            set_paragraph_in_base64(&B64.encode(&doc), 9, ParagraphChange::Style(1), None).unwrap();
        let after = runs_from_base64(&headed);
        assert_eq!(after.last().unwrap().style, Some(1));
        assert!(after.last().unwrap().bold, "the run keeps its weight");
        assert_eq!(after[1].style, Some(103));
    }
}

#[cfg(test)]
mod out_of_step_tests {
    use super::*;

    /// Text of one length with records describing another, as this app's early edits left
    /// Curtains: 140 characters of text over records for 167.
    fn out_of_step() -> Vec<u8> {
        let mut note = length_delimited(TEXT_PATH[2], b"Curtains");
        note.extend(crate::topotext::Topo::fresh_by([0x44; 16], 12).encode());
        note.extend(length_delimited(ATTRIBUTE_RUN, &varint_field(1, 8)));
        let mut document = varint_field(1, 0);
        document.extend(varint_field(2, 0));
        document.extend(length_delimited(TEXT_PATH[1], &note));
        let mut store = varint_field(1, 0);
        store.extend(length_delimited(TEXT_PATH[0], &document));
        gzip(&store)
    }

    /// Replacing such records wholesale was tried, and the phone merged its own characters
    /// back in beside the new ones: the note came back four times over, with copies.
    #[test]
    fn it_is_refused_rather_than_replaced() {
        assert!(replace_text(&out_of_step(), "Curtains\nKnife").is_none());
        assert!(
            !is_rewritable(&out_of_step()),
            "and it is shown read-only before anyone types"
        );
    }
}
