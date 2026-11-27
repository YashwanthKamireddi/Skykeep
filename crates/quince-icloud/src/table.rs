//! A Notes table, read from the attachment that holds it.
//!
//! A table is an `Attachment` record of type `com.apple.notes.table` whose
//! `MergeableDataEncrypted` is a versioned CRDT document (gzip or zlib, then protobuf):
//!
//! - `Document{2: Version{3: data}}`; `data` is `{3: object*, 4: key*, 5: type*, 6: uuid*}`.
//! - Object 0 is a custom object of type `com.apple.notes.ICTable` whose entries name, by
//!   object index, `crRows`, `crColumns` and `cellColumns`.
//! - `crRows` and `crColumns` are ordered sets (object kind 16): a list of elements — each an
//!   index and a uuid — and a dictionary from each element's own `NSUUID` object to the row's
//!   or column's `NSUUID` object. The elements in index order are the rows (or columns) in
//!   reading order.
//! - `cellColumns` is a dictionary: column object → dictionary: row object → a text object
//!   (kind 10) whose field 2 is the cell's text. A cell never typed in may be missing; it is
//!   empty.
//!
//! Measured on tables made on a phone and on icloud.com: a 2×2 never typed in, and an 11×2
//! with text in its cells, read back exactly as Notes shows them.

use std::collections::HashMap;

/// The attachment type a table is.
pub const UTI: &str = "com.apple.notes.table";

/// Where a table's cells are kept in the library, by attachment id.
#[must_use]
pub fn state_key(id: &str) -> String {
    format!("notes.table.{id}")
}

/// A table's cells as the last pass stored them; `None` when none are known.
#[must_use]
pub fn stored(library: &quince_core::Library, id: &str) -> Option<Vec<Vec<String>>> {
    library
        .state(&state_key(id))
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn varint(b: &[u8], mut at: usize) -> Option<(u64, usize)> {
    let mut v = 0u64;
    for shift in 0..10 {
        let byte = *b.get(at)?;
        at += 1;
        v |= u64::from(byte & 0x7f) << (shift * 7);
        if byte & 0x80 == 0 {
            return Some((v, at));
        }
    }
    None
}

#[derive(Clone, Copy)]
enum Wire<'a> {
    Int(u64),
    Bytes(&'a [u8]),
}

fn fields(b: &[u8]) -> Option<Vec<(u64, Wire<'_>)>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < b.len() {
        let (key, next) = varint(b, at)?;
        at = next;
        match key & 7 {
            0 => {
                let (v, next) = varint(b, at)?;
                at = next;
                out.push((key >> 3, Wire::Int(v)));
            }
            2 => {
                let (n, next) = varint(b, at)?;
                let end = next.checked_add(usize::try_from(n).ok()?)?;
                out.push((key >> 3, Wire::Bytes(b.get(next..end)?)));
                at = end;
            }
            1 => at = at.checked_add(8).filter(|&e| e <= b.len())?,
            5 => at = at.checked_add(4).filter(|&e| e <= b.len())?,
            _ => return None,
        }
    }
    Some(out)
}

fn bytes<'a, 'f>(fs: &'f [(u64, Wire<'a>)], tag: u64) -> impl Iterator<Item = &'a [u8]> + 'f {
    fs.iter().filter_map(move |(t, w)| match (t, w) {
        (t, Wire::Bytes(b)) if *t == tag => Some(*b),
        _ => None,
    })
}

/// The version of a table's document that holds the table, and its place among them: the last
/// with any data. A document may keep more than one (field 2 of the top): a table made on a
/// newer phone was measured with an empty first version and the table in the second, which a
/// reader of the first alone took for no table at all.
fn live_version(document: &[u8]) -> Option<(usize, &[u8])> {
    let top = fields(document)?;
    let found = bytes(&top, 2)
        .enumerate()
        .filter(|(_, v)| {
            fields(v)
                .and_then(|f| bytes(&f, 3).next().map(|d| !d.is_empty()))
                .unwrap_or(false)
        })
        .last();
    found
}

fn int(fs: &[(u64, Wire<'_>)], tag: u64) -> Option<u64> {
    fs.iter().find_map(|(t, w)| match (t, w) {
        (t, Wire::Int(v)) if *t == tag => Some(*v),
        _ => None,
    })
}

/// An object id's object index, where it is one (field 6).
fn object_index(id: &[u8]) -> Option<usize> {
    usize::try_from(int(&fields(id)?, 6)?).ok()
}

/// A table's data, inflated — within a ceiling: a shared note's table is whoever shared it's
/// to write, and a bomb is refused rather than read into memory (crate::bounded).
fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    use crate::bounded::{gunzip, MOST_INFLATED};
    use std::io::Read;
    if data.starts_with(&[0x1f, 0x8b]) {
        return gunzip(data, MOST_INFLATED);
    }
    // Zlib, or (when it is not) bare bytes. One that is zlib but too large is refused, not
    // taken for bare bytes.
    let mut out = Vec::new();
    match flate2::read::ZlibDecoder::new(data)
        .take(MOST_INFLATED + 1)
        .read_to_end(&mut out)
    {
        Ok(_) if out.len() as u64 > MOST_INFLATED => None,
        Ok(_) => Some(out),
        Err(_) => Some(data.to_vec()),
    }
}

/// What a cell is keyed by; see [`Doc::who`].
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Who {
    Uuid(usize),
    Object(usize),
}

/// A table as rows × columns of text objects; see [`Doc::grid`].
struct Grid {
    rows: usize,
    columns: usize,
    cells: HashMap<(usize, usize), usize>,
}

struct Doc<'a> {
    objects: Vec<&'a [u8]>,
    keys: Vec<String>,
    uuids: Vec<&'a [u8]>,
}

impl<'a> Doc<'a> {
    /// An object's kind (its one field's number) and payload.
    fn object(&self, i: usize) -> Option<(u64, &'a [u8])> {
        let fs = fields(self.objects.get(i)?)?;
        match fs.first()? {
            (kind, Wire::Bytes(b)) => Some((*kind, *b)),
            _ => None,
        }
    }

    /// A custom object's entries: key name → the object id's bytes.
    fn custom(&self, i: usize) -> Option<HashMap<String, &'a [u8]>> {
        let (kind, body) = self.object(i)?;
        if kind != 13 {
            return None;
        }
        let mut out = HashMap::new();
        for entry in bytes(&fields(body)?, 3) {
            let e = fields(entry)?;
            let key = usize::try_from(int(&e, 1)?).ok()?;
            let value = bytes(&e, 2).next()?;
            out.insert(self.keys.get(key)?.clone(), value);
        }
        Some(out)
    }

    /// A dictionary object (kind 6, or a dictionary message itself): object → object.
    fn dictionary(body: &[u8]) -> Option<HashMap<usize, usize>> {
        Some(Self::entries(body)?.into_iter().collect())
    }

    /// A dictionary's entries as written, in order, a key that comes again included.
    fn entries(body: &[u8]) -> Option<Vec<(usize, usize)>> {
        let mut out = Vec::new();
        for element in bytes(&fields(body)?, 1) {
            let e = fields(element)?;
            let (Some(k), Some(v)) = (bytes(&e, 1).next(), bytes(&e, 2).next()) else {
                continue;
            };
            if let (Some(k), Some(v)) = (object_index(k), object_index(v)) {
                out.push((k, v));
            }
        }
        Some(out)
    }

    /// The `NSUUID` object holding each uuid.
    /// Where two objects hold the same uuid, the later one.
    fn uuid_objects(&self) -> HashMap<&'a [u8], usize> {
        let mut held: Vec<(usize, usize)> = self.uuid_at().into_iter().collect();
        held.sort_unstable();
        held.into_iter()
            .filter_map(|(i, at)| Some((*self.uuids.get(at)?, i)))
            .collect()
    }

    /// Every `NSUUID` object, and the place among the document's uuids of the one it holds.
    fn uuid_at(&self) -> HashMap<usize, usize> {
        let mut out = HashMap::new();
        for i in 0..self.objects.len() {
            if let Some(entries) = self.custom(i) {
                if let Some(at) = entries
                    .get("UUIDIndex")
                    .and_then(|id| int(&fields(id)?, 2).or_else(|| int(&fields(id)?, 1)))
                    .and_then(|at| usize::try_from(at).ok())
                    .filter(|&at| at < self.uuids.len())
                {
                    out.insert(i, at);
                }
            }
        }
        out
    }

    /// An ordered set's members, in order, as the objects the table keys its cells by.
    fn ordered(&self, i: usize, by_uuid: &HashMap<&[u8], usize>) -> Option<Vec<usize>> {
        Some(
            self.ordered_pairs(i, by_uuid)?
                .into_iter()
                .map(|(_, identity)| identity)
                .collect(),
        )
    }

    /// An ordered set's members, in order: each one's element object and its identity (the
    /// object the element maps to; the element itself when it maps to none).
    fn ordered_pairs(
        &self,
        i: usize,
        by_uuid: &HashMap<&[u8], usize>,
    ) -> Option<Vec<(usize, usize)>> {
        let (_, body) = self.object(i)?;
        let top = fields(body)?;
        let array = fields(bytes(&top, 1).next()?)?;
        let list = fields(bytes(&array, 1).next()?)?;
        let mut elements: Vec<(u64, &[u8])> = Vec::new();
        for element in bytes(&list, 2) {
            let e = fields(element)?;
            elements.push((int(&e, 1).unwrap_or(0), bytes(&e, 2).next()?));
        }
        elements.sort_by_key(|(at, _)| *at);
        let mut members: HashMap<usize, usize> = HashMap::new();
        for d in bytes(&array, 2) {
            members.extend(Self::dictionary(d)?);
        }
        Some(
            elements
                .into_iter()
                .filter_map(|(_, u)| {
                    let own = *by_uuid.get(u)?;
                    Some((own, members.get(&own).copied().unwrap_or(own)))
                })
                .collect(),
        )
    }

    /// What an object stands for when the table keys a cell by it: the uuid an `NSUUID`
    /// object holds — two objects may hold the same one — or, for any other object, itself.
    fn who(&self, i: usize, uuid_at: &HashMap<usize, usize>) -> Who {
        uuid_at.get(&i).map_or(Who::Object(i), |&u| Who::Uuid(u))
    }

    /// The document as a grid: how many rows and columns, and each cell's text object by
    /// (row, column).
    ///
    /// Read as `apple_cloud_notes_parser` reads one: a row (or column) is found in `cellColumns`
    /// by the uuid of its identity or of its element, and every entry counts (a column or row
    /// named twice gives its cells from both). A cell keyed by identity wins over one keyed by
    /// element.
    /// #289: a table whose two added columns read empty in the rows made before them, and
    /// whole in the row made after, where the reader matched identity objects alone.
    fn grid(&self) -> Option<Grid> {
        let root = self.custom(0)?;
        let at = |name: &str| root.get(name).and_then(|id| object_index(id));
        let by_uuid = self.uuid_objects();
        let uuid_at = self.uuid_at();
        // Where each identity and each element sits; identities first, so an element that is
        // also some member's identity is read as that member.
        let place = |pairs: Vec<(usize, usize)>| {
            let mut out: HashMap<Who, (usize, bool)> = HashMap::new();
            for (n, &(element, _)) in pairs.iter().enumerate() {
                out.insert(self.who(element, &uuid_at), (n, false));
            }
            for (n, &(_, identity)) in pairs.iter().enumerate() {
                out.insert(self.who(identity, &uuid_at), (n, true));
            }
            (pairs.len(), out)
        };
        let (rows, row_at) = place(self.ordered_pairs(at("crRows")?, &by_uuid)?);
        let (columns, column_at) = place(self.ordered_pairs(at("crColumns")?, &by_uuid)?);
        let (_, cells_body) = self.object(at("cellColumns")?)?;
        let mut cells: HashMap<(usize, usize), (usize, bool)> = HashMap::new();
        for (column, inner) in Self::entries(cells_body)? {
            let Some(&(c, column_named)) = column_at.get(&self.who(column, &uuid_at)) else {
                continue;
            };
            let Some(entries) = self.object(inner).and_then(|(_, b)| Self::entries(b)) else {
                continue;
            };
            for (row, cell) in entries {
                let Some(&(r, row_named)) = row_at.get(&self.who(row, &uuid_at)) else {
                    continue;
                };
                let named = column_named && row_named;
                match cells.get(&(r, c)) {
                    Some(&(_, true)) if !named => {}
                    _ => {
                        cells.insert((r, c), (cell, named));
                    }
                }
            }
        }
        Some(Grid {
            rows,
            columns,
            cells: cells
                .into_iter()
                .map(|(at, (cell, _))| (at, cell))
                .collect(),
        })
    }

    fn text(&self, i: usize) -> String {
        self.object(i)
            .filter(|(kind, _)| *kind == 10)
            .and_then(|(_, body)| fields(body))
            .and_then(|fs| {
                bytes(&fs, 2)
                    .next()
                    .map(|b| String::from_utf8_lossy(b).into_owned())
            })
            .unwrap_or_default()
    }
}

fn put_varint(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// `message` with the `nth` length-delimited field `tag` replaced by `payload`, every other
/// byte copied through. `None` when there is no such field.
fn splice(message: &[u8], tag: u64, nth: usize, payload: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(message.len() + payload.len());
    let (mut at, mut seen, mut done) = (0usize, 0usize, false);
    while at < message.len() {
        let start = at;
        let (key, next) = varint(message, at)?;
        at = next;
        match key & 7 {
            0 => at = varint(message, at)?.1,
            2 => {
                let (n, next) = varint(message, at)?;
                let end = next.checked_add(usize::try_from(n).ok()?)?;
                message.get(next..end)?;
                if key >> 3 == tag && !done {
                    if seen == nth {
                        put_varint(key, &mut out);
                        put_varint(payload.len() as u64, &mut out);
                        out.extend_from_slice(payload);
                        done = true;
                        at = end;
                        continue;
                    }
                    seen += 1;
                }
                at = end;
            }
            1 => at = at.checked_add(8).filter(|&e| e <= message.len())?,
            5 => at = at.checked_add(4).filter(|&e| e <= message.len())?,
            _ => return None,
        }
        out.extend_from_slice(message.get(start..at)?);
    }
    done.then_some(out)
}

/// Where a cell's text lives: the index of its text object among the document's objects.
fn cell_object(document: &[u8], row: usize, column: usize) -> Option<usize> {
    let version = fields(live_version(document)?.1)?;
    let data = fields(bytes(&version, 3).next()?)?;
    let doc = Doc {
        objects: bytes(&data, 3).collect(),
        keys: bytes(&data, 4)
            .map(|k| String::from_utf8_lossy(k).into_owned())
            .collect(),
        uuids: bytes(&data, 6).collect(),
    };
    let grid = doc.grid()?;
    if row >= grid.rows || column >= grid.columns {
        return None;
    }
    let cell = *grid.cells.get(&(row, column))?;
    // A text object, as a cell's is.
    (doc.object(cell)?.0 == 10).then_some(cell)
}

/// Who makes a cell edit, and when: what a phone stamps into the records an edit writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellEditor {
    /// This installation's replica id: the uuid it signs text with, the same every run (the
    /// app keeps it in the library as `documents.writer`; see `topotext::set_writer`).
    pub replica: [u8; 16],
    /// Unix seconds, for the new attribute run's field 13.
    pub unix_seconds: u64,
    /// The paragraph uuid a cell gets when it has no run to take one from (a cell never
    /// typed in). A phone gives such a cell a fresh one.
    pub paragraph: [u8; 16],
}

impl CellEditor {
    /// This installation, now.
    #[must_use]
    pub fn this_installation() -> Self {
        Self {
            replica: crate::topotext::writer_bytes(),
            unix_seconds: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            paragraph: *uuid::Uuid::new_v4().as_bytes(),
        }
    }
}

/// The table's document with one cell's text changed, by this installation, now. See
/// [`with_cell_by`].
#[must_use]
pub fn with_cell(document: &[u8], row: usize, column: usize, text: &str) -> Option<Vec<u8>> {
    with_cell_by(
        document,
        row,
        column,
        text,
        &CellEditor::this_installation(),
    )
}

/// The table's document with one cell's text changed, written the way a phone writes it
/// (measured; docs/notes-tables-crdt.md):
///
/// - The table's writer list (data field 7) is a map from writer uuid to its clocks (next
///   character, next timestamp), kept **sorted by uuid**, and a writer number in any text of
///   the table is its 1-based place in that sorted list. An editor new to the table goes in
///   at its sorted place, and every text in the table is renumbered to match — the step whose
///   absence (appending instead) made Apple read the phone's own characters as another
///   writer's, and keep both copies.
/// - The cell's merge records: what was typed is a substring under the editor's number with
///   its next character clock; what was removed is marked deleted (never dropped) and stamped
///   with the editor's next timestamp. The cached string is the new text.
/// - The cell's attribute runs: what was typed gets a run of its own, patterned on the run
///   beside it (so its paragraph style and uuid carry on), stamped with the time in Unix
///   seconds at field 13; a cell never typed in gets `{1: n, 2: {9: paragraph uuid}, 13: t}`.
/// - An empty data field 2, which every document a phone has edited carries.
///
/// The version vector (data field 1), the uuid list (field 6) and every other object are left
/// as they are: a phone's text-only edit was measured leaving them untouched. Nor is the
/// record's `ReplicaIDToNotesVersionDataEncrypted` the document's business: it is an
/// account-wide list of devices, identical on every record, that no edit changes.
///
/// Refused (`None`) for anything this cannot do exactly: a writer list that is not sorted (a
/// document an earlier build of this app wrote), records or runs that would not re-encode
/// byte for byte as Apple wrote them, an object of a kind this does not know when texts must
/// be renumbered, or a result that does not read back as the same table with that one cell
/// changed.
#[must_use]
pub fn with_cell_by(
    document: &[u8],
    row: usize,
    column: usize,
    text: &str,
    editor: &CellEditor,
) -> Option<Vec<u8>> {
    let before = cells(document)?;
    let cell = cell_object(document, row, column)?;
    let (version_at, version_bytes) = live_version(document)?;
    let version = fields(version_bytes)?;
    let data_bytes = bytes(&version, 3).next()?;
    let data = fields(data_bytes)?;

    let mut writers = TableWriters::read(bytes(&data, 7).next()?)?;
    let object = bytes(&data, 3).nth(cell)?;
    let mut cell_text = CellText::read(bytes(&fields(object)?, 10).next()?)?;
    let old: Vec<u16> = cell_text.text.encode_utf16().collect();
    let new: Vec<u16> = text.encode_utf16().collect();
    if old == new {
        return Some(document.to_vec());
    }
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let from = prefix as u64;
    let removed = (old.len() - prefix - suffix) as u64;
    let inserted = (new.len() - prefix - suffix) as u64;

    // The editor's number, and — when it is new to the table and sorts before writers
    // already there — every text renumbered to keep each character with its writer.
    let (me, shift_from) = writers.own(editor.replica);
    let mut objects: Vec<Vec<u8>> = Vec::new();
    for o in bytes(&data, 3) {
        objects.push(match shift_from {
            Some(from) => renumber_object(o, from)?,
            None => o.to_vec(),
        });
    }
    if let Some(from) = shift_from {
        cell_text.renumber(from);
    }

    cell_text.edit(&mut writers, me, from, removed, inserted, editor)?;
    cell_text.text = text.to_owned();
    let string = bytes(&fields(&objects[cell])?, 10).next()?.to_vec();
    let new_string = cell_text.encode(&string)?;
    objects[cell] = splice(&objects[cell], 10, 0, &new_string)?;

    let mut new_data = Vec::with_capacity(data_bytes.len() + 128);
    let mut object_n = 0usize;
    let has_two = data.iter().any(|(t, _)| *t == 2);
    for (tag, raw) in raw_fields(data_bytes)? {
        match tag {
            1 => {
                new_data.extend_from_slice(raw);
                if !has_two {
                    put_len(2, &[], &mut new_data);
                }
            }
            3 => {
                put_len(3, objects.get(object_n)?, &mut new_data);
                object_n += 1;
            }
            7 => put_len(7, &writers.encode(), &mut new_data),
            _ => new_data.extend_from_slice(raw),
        }
    }
    let new_version = splice(version_bytes, 3, 0, &new_data)?;
    let out = splice(document, 2, version_at, &new_version)?;

    // It reads back as the same table with that one cell changed.
    let mut expected = before;
    *expected.get_mut(row)?.get_mut(column)? = text.to_owned();
    (cells(&out)? == expected).then_some(out)
}

/// The end marker's character clock.
const END_CLOCK: u64 = 0xFFFF_FFFF;

/// The table's writer list (data field 7): `{1: {1: uuid, 2: {1: next character},
/// 2: {1: next timestamp}}, …}`, sorted by uuid. A text's writer number is a 1-based place in
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TableWriters(Vec<([u8; 16], Vec<u64>)>);

impl TableWriters {
    /// The list, proven to re-encode as it was written and to be in the order Apple keeps it.
    fn read(list: &[u8]) -> Option<Self> {
        let mut out = Vec::new();
        for (tag, entry) in fields(list)? {
            let (1, Wire::Bytes(entry)) = (tag, entry) else {
                return None;
            };
            let mut uuid = None;
            let mut clocks = Vec::new();
            for (tag, value) in fields(entry)? {
                match (tag, value) {
                    (1, Wire::Bytes(u)) if uuid.is_none() => {
                        uuid = Some(<[u8; 16]>::try_from(u).ok()?);
                    }
                    (2, Wire::Bytes(c)) => {
                        let cf = fields(c)?;
                        if cf.len() != 1 {
                            return None;
                        }
                        clocks.push(int(&cf, 1)?);
                    }
                    _ => return None,
                }
            }
            out.push((uuid?, clocks));
        }
        let writers = Self(out);
        let sorted = writers.0.windows(2).all(|w| w[0].0 < w[1].0);
        (sorted && writers.encode() == list).then_some(writers)
    }

    fn encode(&self) -> Vec<u8> {
        let mut list = Vec::new();
        for (uuid, clocks) in &self.0 {
            let mut entry = Vec::new();
            put_len(1, uuid, &mut entry);
            for c in clocks {
                let mut clock = Vec::new();
                put_int(1, *c, &mut clock);
                put_len(2, &clock, &mut entry);
            }
            put_len(1, &entry, &mut list);
        }
        list
    }

    /// The editor's 1-based number. One new to the list goes in at its sorted place, clocks
    /// at zero; then every number from that place up moves up one, and that place is
    /// answered too (`None` when nothing moves).
    fn own(&mut self, replica: [u8; 16]) -> (u64, Option<u64>) {
        let (at, shift) = match self.0.binary_search_by(|(u, _)| u.cmp(&replica)) {
            Ok(at) => (at, None),
            Err(at) => {
                self.0.insert(at, (replica, vec![0, 0]));
                let moved = at + 1 < self.0.len();
                (at, moved.then_some(at as u64 + 1))
            }
        };
        while self.0[at].1.len() < 2 {
            self.0[at].1.push(0);
        }
        (at as u64 + 1, shift)
    }

    /// Take `n` characters (`which` 0) or timestamps (`which` 1) from writer `me`'s clock.
    fn take(&mut self, me: u64, which: usize, n: u64) -> Option<u64> {
        let clock = self
            .0
            .get_mut(usize::try_from(me.checked_sub(1)?).ok()?)?
            .1
            .get_mut(which)?;
        let at = *clock;
        *clock += n;
        Some(at)
    }
}

/// One substring of a table text's merge records: `{1: id, 2: length, 3: timestamp,
/// 4: deleted, 5: next}`, where an id and a timestamp are `{1: writer, 2: clock}`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Piece {
    id: (u64, u64),
    length: u64,
    stamp: (u64, u64),
    deleted: bool,
}

fn pair_of(b: &[u8]) -> Option<(u64, u64)> {
    let f = fields(b)?;
    if f.iter()
        .any(|(t, w)| !matches!((t, w), (1 | 2, Wire::Int(_))))
    {
        return None;
    }
    Some((int(&f, 1).unwrap_or(0), int(&f, 2).unwrap_or(0)))
}

fn put_pair(tag: u64, (a, b): (u64, u64), out: &mut Vec<u8>) {
    let mut m = Vec::new();
    put_int(1, a, &mut m);
    put_int(2, b, &mut m);
    put_len(tag, &m, out);
}

/// A text's merge records, refused unless they are one chain from the start marker to the
/// end marker, in reading order, that re-encodes byte for byte.
fn read_pieces(text: &[u8]) -> Option<Vec<Piece>> {
    let mut pieces = Vec::new();
    let mut original = Vec::new();
    for (i, raw) in bytes(&fields(text)?, 3).enumerate() {
        put_len(3, raw, &mut original);
        let mut p = Piece {
            id: (0, 0),
            length: 0,
            stamp: (0, 0),
            deleted: false,
        };
        let mut next = None;
        for (tag, value) in fields(raw)? {
            match (tag, value) {
                (1, Wire::Bytes(b)) => p.id = pair_of(b)?,
                (2, Wire::Int(v)) => p.length = v,
                (3, Wire::Bytes(b)) => p.stamp = pair_of(b)?,
                (4, Wire::Int(v)) => p.deleted = v != 0,
                (5, Wire::Int(v)) if next.is_none() => next = Some(v),
                _ => return None,
            }
        }
        if next.is_some_and(|n| n != i as u64 + 1) {
            return None;
        }
        pieces.push(p);
    }
    let n = pieces.len();
    if n < 2
        || pieces[0].id != (0, 0)
        || pieces[0].length != 0
        || pieces[n - 1].id != (0, END_CLOCK)
        || pieces[n - 1].length != 0
    {
        return None;
    }
    (encode_pieces(&pieces) == original).then_some(pieces)
}

fn encode_pieces(pieces: &[Piece]) -> Vec<u8> {
    let mut out = Vec::new();
    let last = pieces.len().saturating_sub(1);
    for (i, p) in pieces.iter().enumerate() {
        let mut m = Vec::new();
        put_pair(1, p.id, &mut m);
        put_int(2, p.length, &mut m);
        put_pair(3, p.stamp, &mut m);
        if p.deleted {
            put_int(4, 1, &mut m);
        }
        if i != last {
            put_int(5, i as u64 + 1, &mut m);
        }
        put_len(3, &m, &mut out);
    }
    out
}

/// Writer numbers from `from` up moved up one, in a run of pieces. Number 0 (the markers)
/// is nobody's and stays.
fn renumber_pieces(pieces: &mut [Piece], from: u64) {
    for p in pieces {
        if p.id.0 >= from {
            p.id.0 += 1;
        }
        if p.stamp.0 >= from {
            p.stamp.0 += 1;
        }
    }
}

/// A text message with its merge records replaced, every other field where it was.
fn with_pieces(text: &[u8], pieces: &[Piece]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() + 16);
    let mut wrote = false;
    for (tag, raw) in raw_fields(text)? {
        match tag {
            3 if !wrote => {
                out.extend(encode_pieces(pieces));
                wrote = true;
            }
            3 => {}
            _ => out.extend_from_slice(raw),
        }
    }
    wrote.then_some(out)
}

/// A text message renumbered (see [`renumber_pieces`]).
fn renumber_text(text: &[u8], from: u64) -> Option<Vec<u8>> {
    let mut pieces = read_pieces(text)?;
    renumber_pieces(&mut pieces, from);
    with_pieces(text, &pieces)
}

/// A table object with the writer numbers in its texts renumbered: a cell's text (kind 10)
/// and an ordered set's (kind 16, at 1 → 1 → 1). Objects that hold no text — registers
/// (1), dictionaries (6), custom objects (13) — are kept as they are; any other kind is one
/// this does not know, and refused.
fn renumber_object(object: &[u8], from: u64) -> Option<Vec<u8>> {
    let of = fields(object)?;
    let [(kind, Wire::Bytes(body))] = of.as_slice() else {
        return None;
    };
    match kind {
        1 | 6 | 13 => Some(object.to_vec()),
        10 => splice(object, 10, 0, &renumber_text(body, from)?),
        16 => {
            let array = bytes(&fields(body)?, 1).next()?;
            let string_array = bytes(&fields(array)?, 1).next()?;
            let text = bytes(&fields(string_array)?, 1).next()?;
            let string_array = splice(string_array, 1, 0, &renumber_text(text, from)?)?;
            let array = splice(array, 1, 0, &string_array)?;
            splice(object, 16, 0, &splice(body, 1, 0, &array)?)
        }
        _ => None,
    }
}

/// A cell's text object, as far as an edit touches it: the cached text, its merge records in
/// reading order, and its attribute runs. Every other field is copied through.
struct CellText {
    text: String,
    pieces: Vec<Piece>,
    runs: Vec<Vec<u8>>,
}

fn run_units(run: &[u8]) -> Option<u64> {
    int(&fields(run)?, 1)
}

impl CellText {
    /// A cell's text, refused unless its records read (see [`read_pieces`]) and agree with
    /// its text, and its runs cover the text exactly.
    fn read(object: &[u8]) -> Option<Self> {
        let f = fields(object)?;
        // A writer list inside the text: no phone writes one in a table.
        if bytes(&f, 4).next().is_some() {
            return None;
        }
        let text = String::from_utf8(bytes(&f, 2).next()?.to_vec()).ok()?;
        let pieces = read_pieces(object)?;
        let runs: Vec<Vec<u8>> = bytes(&f, 5).map(<[u8]>::to_vec).collect();
        let units = text.encode_utf16().count() as u64;
        let live: u64 = pieces.iter().filter(|p| !p.deleted).map(|p| p.length).sum();
        let covered = runs.iter().map(|r| run_units(r)).sum::<Option<u64>>()?;
        (live == units && covered == units).then_some(Self { text, pieces, runs })
    }

    fn renumber(&mut self, from: u64) {
        renumber_pieces(&mut self.pieces, from);
    }

    /// The object with this text, these records and these runs in the places the old ones
    /// held; every other field as it was.
    fn encode(&self, original: &[u8]) -> Option<Vec<u8>> {
        let mut runs = Vec::new();
        for r in &self.runs {
            put_len(5, r, &mut runs);
        }
        let mut out = Vec::with_capacity(original.len() + 64);
        let (mut wrote_pieces, mut wrote_runs) = (false, false);
        for (tag, raw) in raw_fields(original)? {
            match tag {
                2 => put_len(2, self.text.as_bytes(), &mut out),
                3 if !wrote_pieces => {
                    out.extend(encode_pieces(&self.pieces));
                    wrote_pieces = true;
                }
                5 if !wrote_runs => {
                    out.extend_from_slice(&runs);
                    wrote_runs = true;
                }
                3 | 5 => {}
                _ => out.extend_from_slice(raw),
            }
        }
        // A cell never typed in has no runs; they go after its records, where a phone puts
        // them.
        if !wrote_runs {
            out.extend_from_slice(&runs);
        }
        wrote_pieces.then_some(out)
    }

    /// Make live offset `at` fall between two pieces.
    fn split_at(&mut self, at: u64) {
        let mut offset = 0;
        for i in 0..self.pieces.len() {
            if self.pieces[i].deleted {
                continue;
            }
            let len = self.pieces[i].length;
            if offset < at && at < offset + len {
                let keep = at - offset;
                let mut tail = self.pieces[i].clone();
                tail.id.1 += keep;
                tail.length -= keep;
                self.pieces[i].length = keep;
                self.pieces.insert(i + 1, tail);
                return;
            }
            offset += len;
        }
    }

    /// `removed` units at `from` replaced by `inserted` new ones, by writer `me`, in the
    /// records and the runs.
    fn edit(
        &mut self,
        writers: &mut TableWriters,
        me: u64,
        from: u64,
        removed: u64,
        inserted: u64,
        editor: &CellEditor,
    ) -> Option<()> {
        if removed > 0 {
            // Marked deleted, never dropped, and stamped: each deletion a phone makes
            // carries the timestamp it was made at.
            let stamp = (me, writers.take(me, 1, 1)?);
            self.split_at(from);
            self.split_at(from + removed);
            let mut offset = 0;
            for p in &mut self.pieces {
                if p.deleted {
                    continue;
                }
                if offset >= from && offset + p.length <= from + removed && p.length > 0 {
                    p.deleted = true;
                    p.stamp = stamp;
                }
                offset += p.length;
            }
        }
        if inserted > 0 {
            self.split_at(from);
            // Right after the live character before `from` (after the start marker at 0).
            let mut offset = 0;
            let mut place = self.pieces.len() - 1;
            for (i, p) in self.pieces.iter().enumerate().skip(1) {
                if offset == from {
                    place = i;
                    break;
                }
                if !p.deleted {
                    offset += p.length;
                }
            }
            let clock = writers.take(me, 0, inserted)?;
            let stamp = writers.take(me, 1, 1)?;
            self.pieces.insert(
                place,
                Piece {
                    id: (me, clock),
                    length: inserted,
                    stamp: (me, stamp),
                    deleted: false,
                },
            );
        }
        self.runs = edit_runs(&self.runs, from, removed, inserted, editor)?;
        Some(())
    }
}

/// `run` with field `tag` set to `value` (a varint), in its place or appended.
fn with_run_int(run: &[u8], tag: u64, value: u64) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(run.len() + 6);
    let mut set = false;
    for (t, raw) in raw_fields(run)? {
        if t == tag {
            if !set {
                put_int(tag, value, &mut out);
                set = true;
            }
        } else {
            out.extend_from_slice(raw);
        }
    }
    if !set {
        put_int(tag, value, &mut out);
    }
    Some(out)
}

/// A cell's attribute runs after an edit, as a phone leaves them: what was removed cut out
/// of the runs it fell in, and what was typed a run of its own at its place, patterned on
/// the run before it (else the one after), so the paragraph style and its uuid carry on, and
/// stamped with the time (field 13). With no run to pattern on (a cell never typed in) it is
/// `{1: n, 2: {9: paragraph uuid}, 13: t}`.
fn edit_runs(
    runs: &[Vec<u8>],
    from: u64,
    removed: u64,
    inserted: u64,
    editor: &CellEditor,
) -> Option<Vec<Vec<u8>>> {
    let end = from + removed;
    // (pattern, length), and where the new run goes with what it is patterned on.
    let mut out: Vec<(Vec<u8>, u64)> = Vec::with_capacity(runs.len() + 2);
    let mut slot: Option<(usize, Vec<u8>)> = None;
    let mut cursor = 0u64;
    for run in runs {
        let len = run_units(run)?;
        let (start, stop) = (cursor, cursor + len);
        cursor = stop;
        let head = from.saturating_sub(start).min(len);
        let tail = stop.saturating_sub(end).min(len);
        if head + tail > len {
            return None;
        }
        if head > 0 {
            out.push((run.clone(), head));
            if slot.is_none() && from <= stop {
                slot = Some((out.len(), run.clone()));
            }
        } else if slot.is_none() && from <= start {
            slot = Some((out.len(), run.clone()));
        }
        if tail > 0 {
            out.push((run.clone(), tail));
        }
    }
    if inserted > 0 {
        let (at, pattern) = slot.unwrap_or_else(|| {
            (
                out.len(),
                out.last().map(|(r, _)| r.clone()).unwrap_or_default(),
            )
        });
        let pattern = if pattern.is_empty() {
            let mut style = Vec::new();
            put_len(9, &editor.paragraph, &mut style);
            let mut run = Vec::new();
            put_int(1, inserted, &mut run);
            put_len(2, &style, &mut run);
            run
        } else {
            pattern
        };
        let run = with_run_int(&pattern, 13, editor.unix_seconds)?;
        out.insert(at.min(out.len()), (run, inserted));
    }
    out.into_iter()
        .filter(|(_, n)| *n > 0)
        .map(|(r, n)| with_run_int(&r, 1, n))
        .collect()
}

/// A table's text (a cell or an ordered set's) edited by `edit`, against the writer list
/// the table keeps once for all its texts (document field 7, `ttTimestamp`): a phone writes
/// none in the text itself, and ids in a table's texts count from the document's list. The
/// list is lent to the text for the edit and taken back out, so this installation joins it
/// as the next writer instead of claiming the phone's number. Answers the text and the
/// document's list as they now are. A text that already carries a list of its own is
/// refused: no device writes one, so its ids cannot be read against the document's.
fn edit_table_text(
    text: &[u8],
    writers: &[u8],
    edit: impl FnOnce(&[u8]) -> Option<Vec<u8>>,
) -> Option<(Vec<u8>, Vec<u8>)> {
    if bytes(&fields(text)?, 4).next().is_some() {
        return None;
    }
    let mut lent = text.to_vec();
    put_len(4, writers, &mut lent);
    let edited = edit(&lent)?;
    let new_writers = bytes(&fields(&edited)?, 4).next()?.to_vec();
    // A writer new to the table must come last: this text alone is renumbered by the edit,
    // and the table's other texts would keep the old numbers.
    let (before, after) = (
        TableWriters::read(writers),
        TableWriters::read(&new_writers),
    );
    if let (Some(b), Some(a)) = (&before, &after) {
        if a.0.len() != b.0.len() && a.0.get(..b.0.len()) != Some(&b.0[..]) {
            return None;
        }
    }
    // Apple keeps this list sorted by uuid and reads a writer number as a place in that
    // order; a writer appended out of order would hand the phone's characters to another
    // writer (docs/notes-tables-crdt.md). Row and column writes do not renumber, so they are
    // refused when the list would come out unsorted.
    TableWriters::read(&new_writers)?;
    let mut back = Vec::with_capacity(edited.len());
    for (tag, raw) in raw_fields(&edited)? {
        if tag != 4 {
            back.extend_from_slice(raw);
        }
    }
    Some((back, new_writers))
}

/// Each field of a message with its whole encoding (key, length and payload), in order.
fn raw_fields(b: &[u8]) -> Option<Vec<(u64, &[u8])>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < b.len() {
        let start = at;
        let (key, next) = varint(b, at)?;
        at = next;
        match key & 7 {
            0 => at = varint(b, at)?.1,
            2 => {
                let (n, next) = varint(b, at)?;
                at = next.checked_add(usize::try_from(n).ok()?)?;
                b.get(..at)?;
            }
            1 => at = at.checked_add(8).filter(|&e| e <= b.len())?,
            5 => at = at.checked_add(4).filter(|&e| e <= b.len())?,
            _ => return None,
        }
        out.push((key >> 3, b.get(start..at)?));
    }
    Some(out)
}

fn put_len(tag: u64, payload: &[u8], out: &mut Vec<u8>) {
    put_varint((tag << 3) | 2, out);
    put_varint(payload.len() as u64, out);
    out.extend_from_slice(payload);
}

fn put_int(tag: u64, v: u64, out: &mut Vec<u8>) {
    put_varint(tag << 3, out);
    put_varint(v, out);
}

fn object_id(index: usize) -> Vec<u8> {
    let mut id = Vec::new();
    put_int(6, index as u64, &mut id);
    id
}

/// A dictionary element as Apple writes one: key and value objects, stamped by one replica.
fn dictionary_element(key: usize, value: usize, replica: u64, clock: u64) -> Vec<u8> {
    let mut stamp_one = Vec::new();
    put_int(1, replica, &mut stamp_one);
    put_int(2, clock, &mut stamp_one);
    let mut stamp = Vec::new();
    put_len(1, &stamp_one, &mut stamp);
    let mut e = Vec::new();
    put_len(1, &object_id(key), &mut e);
    put_len(2, &object_id(value), &mut e);
    put_len(3, &stamp, &mut e);
    e
}

/// An `NSUUID` custom object naming uuid number `uuid_index`, in the shape `template` (an
/// existing one) has.
fn nsuuid_object(template: &[u8], uuid_index: usize) -> Option<Vec<u8>> {
    let custom = bytes(&fields(template)?, 13).next()?;
    let cf = fields(custom)?;
    let entry = bytes(&cf, 3).next()?;
    let ef = fields(entry)?;
    let mut value = Vec::new();
    put_int(2, uuid_index as u64, &mut value);
    let mut new_entry = Vec::new();
    put_int(1, int(&ef, 1)?, &mut new_entry);
    put_len(2, &value, &mut new_entry);
    let mut new_custom = Vec::new();
    put_int(1, int(&cf, 1)?, &mut new_custom);
    put_len(3, &new_entry, &mut new_custom);
    let mut object = Vec::new();
    put_len(13, &new_custom, &mut object);
    Some(object)
}

/// The table's document with an empty row added at `at` (0 is the top), as Notes adds one:
/// a new member of the ordered row set, written as this installation's replica, and an
/// empty text in every column so its cells can be typed in at once.
///
/// Nothing is returned unless it is proven: the pieces this writes are first rebuilt from
/// the table's own and must come out byte for byte as Apple wrote them, and the result must
/// read back as the same grid with one empty row at `at`.
#[must_use]
pub fn with_row(document: &[u8], at: usize) -> Option<Vec<u8>> {
    with_member(document, false, at)
}

/// The table's document with an empty column added at `at` (0 is the leading edge), as Notes
/// adds one: a new member of the ordered column set and an empty text in it for every row,
/// proven as [`with_row`] is.
#[must_use]
pub fn with_column(document: &[u8], at: usize) -> Option<Vec<u8>> {
    with_member(document, true, at)
}

fn with_member(document: &[u8], column: bool, at: usize) -> Option<Vec<u8>> {
    let before = cells(document)?;
    let width = before.first().map_or(0, Vec::len);
    let count = if column { width } else { before.len() };
    if at > count {
        return None;
    }
    let (version_at, version_bytes) = live_version(document)?;
    let version = fields(version_bytes)?;
    let data_bytes = bytes(&version, 3).next()?;
    let data = fields(data_bytes)?;
    let objects: Vec<&[u8]> = bytes(&data, 3).collect();
    let uuids: Vec<&[u8]> = bytes(&data, 6).collect();
    let doc = Doc {
        objects: objects.clone(),
        keys: bytes(&data, 4)
            .map(|k| String::from_utf8_lossy(k).into_owned())
            .collect(),
        uuids: uuids.clone(),
    };
    let root = doc.custom(0)?;
    let at_key = |name: &str| root.get(name).and_then(|id| object_index(id));
    let rows_object = at_key(if column { "crColumns" } else { "crRows" })?;
    let cells_object = at_key("cellColumns")?;

    // This installation as a replica of the document, and the clock its writes carry.
    let me = crate::topotext::writer_bytes();
    let mut new_uuids: Vec<Vec<u8>> = Vec::new();
    let replica = match uuids.iter().position(|u| *u == me) {
        Some(i) => i,
        None => {
            new_uuids.push(me.to_vec());
            uuids.len()
        }
    };
    let versions: Vec<(u64, u64)> = bytes(&fields(bytes(&data, 1).next()?)?, 1)
        .filter_map(|e| {
            let f = fields(e)?;
            Some((int(&f, 1).unwrap_or(0), int(&f, 2).unwrap_or(0)))
        })
        .collect();
    let clock = versions
        .iter()
        .find(|(r, _)| *r == replica as u64)
        .map_or(1, |(_, c)| c + 1);

    // The new row's element and identity: two uuids and an NSUUID object for each.
    let template = (0..objects.len()).find_map(|i| {
        let entries = doc.custom(i)?;
        entries.contains_key("UUIDIndex").then_some(objects[i])
    })?;
    // Proof the NSUUID shape is Apple's: an existing one rebuilt comes out the same.
    let template_index = usize::try_from(int(
        &fields(
            bytes(
                &fields(bytes(&fields(bytes(&fields(template)?, 13).next()?)?, 3).next()?)?,
                2,
            )
            .next()?,
        )?,
        2,
    )?)
    .ok()?;
    if nsuuid_object(template, template_index)? != template {
        return None;
    }
    let element_uuid = *uuid::Uuid::new_v4().as_bytes();
    let identity_uuid = *uuid::Uuid::new_v4().as_bytes();
    let element_uuid_index = uuids.len() + new_uuids.len();
    new_uuids.push(element_uuid.to_vec());
    new_uuids.push(identity_uuid.to_vec());
    let element_object = objects.len();
    let identity_object = objects.len() + 1;
    let mut new_objects = vec![
        nsuuid_object(template, element_uuid_index)?,
        nsuuid_object(template, element_uuid_index + 1)?,
    ];

    // The ordered row set: the text gains a character at `at`, the attachments make room for
    // it, and the element joins both dictionaries.
    let (kind, set_body) = doc.object(rows_object)?;
    if kind != 16 {
        return None;
    }
    let set_fields = raw_fields(set_body)?;
    let sf = fields(set_body)?;
    let array_body = bytes(&sf, 1).next()?;
    let af = fields(array_body)?;
    let string_array = bytes(&af, 1).next()?;
    let saf = fields(string_array)?;
    let text = bytes(&saf, 1).next()?;
    let live = count;
    // In at `at` itself: the text is all one character, so only the place says which.
    let old_text: String = std::iter::repeat_n('\u{FFFC}', live).collect();
    let (new_string, new_writers) = edit_table_text(text, bytes(&data, 7).next()?, |lent| {
        crate::ckdocument::rewrite_text_object_at(lent, &old_text, at, 0, "\u{FFFC}")
    })?;
    let mut attachments: Vec<(u64, Vec<u8>)> = Vec::new();
    for a in bytes(&saf, 2) {
        let f = fields(a)?;
        let index = int(&f, 1).unwrap_or(0);
        attachments.push((
            if index >= at as u64 { index + 1 } else { index },
            bytes(&f, 2).next()?.to_vec(),
        ));
    }
    attachments.push((at as u64, element_uuid.to_vec()));
    attachments.sort_by_key(|(i, _)| *i);
    let mut new_string_array = Vec::new();
    for (tag, raw) in raw_fields(string_array)? {
        match tag {
            1 => put_len(1, &new_string, &mut new_string_array),
            2 => {}
            _ => new_string_array.extend_from_slice(raw),
        }
    }
    for (index, uuid) in &attachments {
        let mut a = Vec::new();
        put_int(1, *index, &mut a);
        put_len(2, uuid, &mut a);
        put_len(2, &a, &mut new_string_array);
    }
    let with_element = |dictionary: &[u8], value: usize| -> Option<Vec<u8>> {
        // Proof the element shape is Apple's: the dictionary's last element, rebuilt from its
        // key and value with its own stamp copied, is the same.
        if let Some(last) = bytes(&fields(dictionary)?, 1).last() {
            let f = fields(last)?;
            let mut rebuilt = Vec::new();
            put_len(
                1,
                &object_id(object_index(bytes(&f, 1).next()?)?),
                &mut rebuilt,
            );
            put_len(
                2,
                &object_id(object_index(bytes(&f, 2).next()?)?),
                &mut rebuilt,
            );
            put_len(3, bytes(&f, 3).next()?, &mut rebuilt);
            if rebuilt != last {
                return None;
            }
        }
        let mut out = dictionary.to_vec();
        put_len(
            1,
            &dictionary_element(element_object, value, replica as u64, clock),
            &mut out,
        );
        Some(out)
    };
    let mut new_array = Vec::new();
    for (tag, raw) in raw_fields(array_body)? {
        match tag {
            1 => put_len(1, &new_string_array, &mut new_array),
            2 => put_len(
                2,
                &with_element(bytes(&fields(raw)?, 2).next()?, identity_object)?,
                &mut new_array,
            ),
            _ => new_array.extend_from_slice(raw),
        }
    }
    let mut new_set = Vec::new();
    for (tag, raw) in set_fields {
        match tag {
            1 => put_len(1, &new_array, &mut new_set),
            2 => put_len(
                2,
                &with_element(bytes(&fields(raw)?, 2).next()?, element_object)?,
                &mut new_set,
            ),
            _ => new_set.extend_from_slice(raw),
        }
    }
    let mut new_rows_object = Vec::new();
    put_len(16, &new_set, &mut new_rows_object);

    // An empty text in each new cell, in the shape a phone gives an empty cell.
    let empty_cell = (0..objects.len())
        .find(|&i| doc.object(i).is_some_and(|(k, _)| k == 10) && doc.text(i).is_empty())?;
    let (cells_kind, cells_body) = doc.object(cells_object)?;
    let by_column = Doc::dictionary(cells_body)?;
    let mut changed_objects: HashMap<usize, Vec<u8>> = HashMap::new();
    if column {
        // A dictionary for the new column, a cell in it for every row, and the column in
        // `cellColumns`.
        let row_identities = doc.ordered(at_key("crRows")?, &doc.uuid_objects())?;
        let column_dictionary = objects.len() + new_objects.len();
        new_objects.push(Vec::new()); // filled once its cells have places
        let mut dictionary = Vec::new();
        for row in row_identities {
            let cell = objects.len() + new_objects.len();
            new_objects.push(objects[empty_cell].to_vec());
            put_len(
                1,
                &dictionary_element(row, cell, replica as u64, clock),
                &mut dictionary,
            );
        }
        let mut object = Vec::new();
        put_len(6, &dictionary, &mut object);
        new_objects[column_dictionary - objects.len()] = object;
        let mut grown = with_element(cells_body, column_dictionary)?;
        // `with_element` keys the element by the new member's element object; `cellColumns`
        // is keyed by its identity.
        grown.truncate(cells_body.len());
        put_len(
            1,
            &dictionary_element(identity_object, column_dictionary, replica as u64, clock),
            &mut grown,
        );
        let mut object = Vec::new();
        put_len(cells_kind, &grown, &mut object);
        changed_objects.insert(cells_object, object);
    } else {
        for &column_dictionary in by_column.values() {
            let (k, body) = doc.object(column_dictionary)?;
            if k != 6 {
                return None;
            }
            let cell = objects.len() + new_objects.len();
            new_objects.push(objects[empty_cell].to_vec());
            let mut dictionary = body.to_vec();
            put_len(
                1,
                &dictionary_element(identity_object, cell, replica as u64, clock),
                &mut dictionary,
            );
            let mut object = Vec::new();
            put_len(6, &dictionary, &mut object);
            changed_objects.insert(column_dictionary, object);
        }
    }

    // The document: changed objects in place, new ones and new uuids after the last of their
    // kind, and this replica's clock raised.
    // Every other replica's entry copied as it is (some carry a subclock); this one's set.
    let mut new_version_vector = Vec::new();
    let mut seen_me = false;
    for (_, raw) in raw_fields(bytes(&data, 1).next()?)? {
        let f = fields(bytes(&fields(raw)?, 1).next()?)?;
        if int(&f, 1).unwrap_or(0) == replica as u64 {
            let mut e = Vec::new();
            put_int(1, replica as u64, &mut e);
            put_int(2, clock, &mut e);
            put_len(1, &e, &mut new_version_vector);
            seen_me = true;
        } else {
            new_version_vector.extend_from_slice(raw);
        }
    }
    if !seen_me {
        let mut e = Vec::new();
        put_int(1, replica as u64, &mut e);
        put_int(2, clock, &mut e);
        put_len(1, &e, &mut new_version_vector);
    }
    let raw = raw_fields(data_bytes)?;
    let last_object = raw.iter().rposition(|(t, _)| *t == 3)?;
    let last_uuid = raw.iter().rposition(|(t, _)| *t == 6)?;
    let mut new_data = Vec::with_capacity(data_bytes.len() + 512);
    let mut object_n = 0usize;
    for (i, (tag, field)) in raw.iter().enumerate() {
        match *tag {
            1 => put_len(1, &new_version_vector, &mut new_data),
            // The table's writer list, with this installation in it.
            7 => put_len(7, &new_writers, &mut new_data),
            3 => {
                if object_n == rows_object {
                    put_len(3, &new_rows_object, &mut new_data);
                } else if let Some(o) = changed_objects.get(&object_n) {
                    put_len(3, o, &mut new_data);
                } else {
                    new_data.extend_from_slice(field);
                }
                object_n += 1;
            }
            _ => new_data.extend_from_slice(field),
        }
        if i == last_object {
            for o in &new_objects {
                put_len(3, o, &mut new_data);
            }
        }
        if i == last_uuid {
            for u in &new_uuids {
                put_len(6, u, &mut new_data);
            }
        }
    }
    let new_version = splice(version_bytes, 3, 0, &new_data)?;
    let out = splice(document, 2, version_at, &new_version)?;

    // And it reads back as the same table with one empty row or column at `at`.
    let mut expected = before;
    if column {
        for row in &mut expected {
            row.insert(at, String::new());
        }
    } else {
        expected.insert(at, vec![String::new(); width]);
    }
    (cells(&out)? == expected).then_some(out)
}

/// A dictionary message without the one element keyed by object `key`. `None` unless exactly
/// one element has that key.
fn without_element(dictionary: &[u8], key: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(dictionary.len());
    let mut removed = 0;
    for (tag, raw) in raw_fields(dictionary)? {
        if tag == 1 {
            let element = bytes(&fields(raw)?, 1).next()?;
            if object_index(bytes(&fields(element)?, 1).next()?)? == key {
                removed += 1;
                continue;
            }
        }
        out.extend_from_slice(raw);
    }
    (removed == 1).then_some(out)
}

/// The table's document with the row (or column) at `at` deleted, as a phone deletes one:
/// its character tombstoned in the ordered set's text, its attachment gone and the later
/// ones moved up, it out of the set, and its cells out of every column (or the column out of
/// `cellColumns`). The order dictionary keeps it, as Apple's own deletes do. Proven as
/// [`with_row`] is: the result must read back as the grid without that line.
#[must_use]
pub fn without_row(document: &[u8], at: usize) -> Option<Vec<u8>> {
    without_member(document, false, at)
}

/// [`without_row`] for a column.
#[must_use]
pub fn without_column(document: &[u8], at: usize) -> Option<Vec<u8>> {
    without_member(document, true, at)
}

fn without_member(document: &[u8], column: bool, at: usize) -> Option<Vec<u8>> {
    let before = cells(document)?;
    let width = before.first().map_or(0, Vec::len);
    let count = if column { width } else { before.len() };
    // A table keeps at least one row and one column, as Notes does.
    if at >= count || count < 2 {
        return None;
    }
    let (version_at, version_bytes) = live_version(document)?;
    let version = fields(version_bytes)?;
    let data_bytes = bytes(&version, 3).next()?;
    let data = fields(data_bytes)?;
    let objects: Vec<&[u8]> = bytes(&data, 3).collect();
    let uuids: Vec<&[u8]> = bytes(&data, 6).collect();
    let doc = Doc {
        objects: objects.clone(),
        keys: bytes(&data, 4)
            .map(|k| String::from_utf8_lossy(k).into_owned())
            .collect(),
        uuids: uuids.clone(),
    };
    let root = doc.custom(0)?;
    let at_key = |name: &str| root.get(name).and_then(|id| object_index(id));
    let set_object = at_key(if column { "crColumns" } else { "crRows" })?;
    let cells_object = at_key("cellColumns")?;
    let by_uuid = doc.uuid_objects();

    // The line being deleted: its element (by attachment position) and its identity.
    let (kind, set_body) = doc.object(set_object)?;
    if kind != 16 {
        return None;
    }
    let sf = fields(set_body)?;
    let array_body = bytes(&sf, 1).next()?;
    let af = fields(array_body)?;
    let string_array = bytes(&af, 1).next()?;
    let saf = fields(string_array)?;
    let mut attachments: Vec<(u64, Vec<u8>)> = Vec::new();
    for a in bytes(&saf, 2) {
        let f = fields(a)?;
        attachments.push((int(&f, 1).unwrap_or(0), bytes(&f, 2).next()?.to_vec()));
    }
    let gone_uuid = attachments.iter().find(|(i, _)| *i == at as u64)?.1.clone();
    let element = *by_uuid.get(gone_uuid.as_slice())?;
    let identity = *Doc::dictionary(bytes(&af, 2).next()?)?.get(&element)?;

    let me = crate::topotext::writer_bytes();
    let mut new_uuids: Vec<Vec<u8>> = Vec::new();
    let replica = match uuids.iter().position(|u| *u == me) {
        Some(i) => i,
        None => {
            new_uuids.push(me.to_vec());
            uuids.len()
        }
    };
    let versions: Vec<(u64, u64)> = bytes(&fields(bytes(&data, 1).next()?)?, 1)
        .filter_map(|e| {
            let f = fields(e)?;
            Some((int(&f, 1).unwrap_or(0), int(&f, 2).unwrap_or(0)))
        })
        .collect();
    let clock = versions
        .iter()
        .find(|(r, _)| *r == replica as u64)
        .map_or(1, |(_, c)| c + 1);

    // The ordered set: the character tombstoned, the attachments closed up, the set without it.
    let text = bytes(&saf, 1).next()?;
    // Out at `at` itself: the text is all one character, so only the place says which.
    let old_text: String = std::iter::repeat_n('\u{FFFC}', count).collect();
    let (new_string, new_writers) = edit_table_text(text, bytes(&data, 7).next()?, |lent| {
        crate::ckdocument::rewrite_text_object_at(lent, &old_text, at, 1, "")
    })?;
    let mut new_string_array = Vec::new();
    for (tag, raw) in raw_fields(string_array)? {
        match tag {
            1 => put_len(1, &new_string, &mut new_string_array),
            2 => {}
            _ => new_string_array.extend_from_slice(raw),
        }
    }
    for (index, uuid) in &attachments {
        if *index == at as u64 {
            continue;
        }
        let mut a = Vec::new();
        put_int(
            1,
            if *index > at as u64 {
                index - 1
            } else {
                *index
            },
            &mut a,
        );
        put_len(2, uuid, &mut a);
        put_len(2, &a, &mut new_string_array);
    }
    let mut new_array = Vec::new();
    for (tag, raw) in raw_fields(array_body)? {
        match tag {
            1 => put_len(1, &new_string_array, &mut new_array),
            _ => new_array.extend_from_slice(raw),
        }
    }
    let mut new_set = Vec::new();
    for (tag, raw) in raw_fields(set_body)? {
        match tag {
            1 => put_len(1, &new_array, &mut new_set),
            2 => put_len(
                2,
                &without_element(bytes(&fields(raw)?, 2).next()?, element)?,
                &mut new_set,
            ),
            _ => new_set.extend_from_slice(raw),
        }
    }
    let mut changed_objects: HashMap<usize, Vec<u8>> = HashMap::new();
    let mut object = Vec::new();
    put_len(16, &new_set, &mut object);
    changed_objects.insert(set_object, object);

    // The cells: a row out of every column's dictionary, or a column out of `cellColumns`.
    let (cells_kind, cells_body) = doc.object(cells_object)?;
    if column {
        let mut object = Vec::new();
        put_len(
            cells_kind,
            &without_element(cells_body, identity)?,
            &mut object,
        );
        changed_objects.insert(cells_object, object);
    } else {
        for &column_dictionary in Doc::dictionary(cells_body)?.values() {
            let (k, body) = doc.object(column_dictionary)?;
            if k != 6 {
                return None;
            }
            // A column with no cell for this row has nothing to drop.
            if let Some(trimmed) = without_element(body, identity) {
                let mut object = Vec::new();
                put_len(6, &trimmed, &mut object);
                changed_objects.insert(column_dictionary, object);
            }
        }
    }

    // This replica's clock raised; every other replica's entry copied as it is.
    let mut new_version_vector = Vec::new();
    let mut seen_me = false;
    for (_, raw) in raw_fields(bytes(&data, 1).next()?)? {
        let f = fields(bytes(&fields(raw)?, 1).next()?)?;
        if int(&f, 1).unwrap_or(0) == replica as u64 {
            let mut e = Vec::new();
            put_int(1, replica as u64, &mut e);
            put_int(2, clock, &mut e);
            put_len(1, &e, &mut new_version_vector);
            seen_me = true;
        } else {
            new_version_vector.extend_from_slice(raw);
        }
    }
    if !seen_me {
        let mut e = Vec::new();
        put_int(1, replica as u64, &mut e);
        put_int(2, clock, &mut e);
        put_len(1, &e, &mut new_version_vector);
    }
    let raw = raw_fields(data_bytes)?;
    let last_uuid = raw.iter().rposition(|(t, _)| *t == 6)?;
    let mut new_data = Vec::with_capacity(data_bytes.len());
    let mut object_n = 0usize;
    for (i, (tag, field)) in raw.iter().enumerate() {
        match *tag {
            1 => put_len(1, &new_version_vector, &mut new_data),
            // The table's writer list, with this installation in it.
            7 => put_len(7, &new_writers, &mut new_data),
            3 => {
                match changed_objects.get(&object_n) {
                    Some(o) => put_len(3, o, &mut new_data),
                    None => new_data.extend_from_slice(field),
                }
                object_n += 1;
            }
            _ => new_data.extend_from_slice(field),
        }
        if i == last_uuid {
            for u in &new_uuids {
                put_len(6, u, &mut new_data);
            }
        }
    }
    let new_version = splice(version_bytes, 3, 0, &new_data)?;
    let out = splice(document, 2, version_at, &new_version)?;

    let mut expected = before;
    if column {
        for row in &mut expected {
            row.remove(at);
        }
    } else {
        expected.remove(at);
    }
    (cells(&out)? == expected).then_some(out)
}

/// [`with_cell`] over the attachment's `MergeableDataEncrypted` as Apple sends it (base64,
/// compressed), answered in the same compression it came in.
#[must_use]
pub fn with_cell_in_base64(value: &str, row: usize, column: usize, text: &str) -> Option<String> {
    edit_in_base64(value, |plain| with_cell(plain, row, column, text))
}

/// [`with_row`] over the attachment's `MergeableDataEncrypted` as Apple sends it, answered in
/// the same compression it came in.
#[must_use]
pub fn with_row_in_base64(value: &str, at: usize) -> Option<String> {
    edit_in_base64(value, |plain| with_row(plain, at))
}

/// [`with_column`] over the attachment's `MergeableDataEncrypted` as Apple sends it.
#[must_use]
pub fn with_column_in_base64(value: &str, at: usize) -> Option<String> {
    edit_in_base64(value, |plain| with_column(plain, at))
}

/// [`without_row`] or [`without_column`] over the attachment's `MergeableDataEncrypted`.
#[must_use]
pub fn without_line_in_base64(value: &str, column: bool, at: usize) -> Option<String> {
    edit_in_base64(value, |plain| {
        if column {
            without_column(plain, at)
        } else {
            without_row(plain, at)
        }
    })
}

/// A table edit made to the inflated document, packed back as it came: gzip, zlib or plain.
fn edit_in_base64(value: &str, edit: impl FnOnce(&[u8]) -> Option<Vec<u8>>) -> Option<String> {
    use base64::Engine;
    use std::io::Write;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .ok()?;
    let gz = raw.starts_with(&[0x1f, 0x8b]);
    let plain = inflate(&raw)?;
    let edited = edit(&plain)?;
    let packed = if gz {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(&edited).ok()?;
        e.finish().ok()?
    } else if plain != raw {
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(&edited).ok()?;
        e.finish().ok()?
    } else {
        edited
    };
    Some(base64::engine::general_purpose::STANDARD.encode(packed))
}

/// The table's cells, row by row, from the attachment's `MergeableDataEncrypted` (base64).
/// `None` for data that is not a table this understands.
#[must_use]
pub fn cells_from_base64(value: &str) -> Option<Vec<Vec<String>>> {
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .ok()?;
    cells(&inflate(&raw)?)
}

/// [`cells_from_base64`] over the inflated document.
#[must_use]
pub fn cells(document: &[u8]) -> Option<Vec<Vec<String>>> {
    let version = fields(live_version(document)?.1)?;
    let data = fields(bytes(&version, 3).next()?)?;
    let doc = Doc {
        objects: bytes(&data, 3).collect(),
        keys: bytes(&data, 4)
            .map(|k| String::from_utf8_lossy(k).into_owned())
            .collect(),
        uuids: bytes(&data, 6).collect(),
    };
    let grid = doc.grid()?;
    Some(
        (0..grid.rows)
            .map(|r| {
                (0..grid.columns)
                    .map(|c| {
                        grid.cells
                            .get(&(r, c))
                            .map(|&cell| doc.text(cell))
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shared note's table that inflates past the ceiling is refused, never read whole.
    #[test]
    fn a_table_bomb_is_refused() {
        use std::io::Write;
        let zeros = vec![0u8; 1024 * 1024];
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        for _ in 0..=(crate::bounded::MOST_INFLATED / (1024 * 1024)) {
            gz.write_all(&zeros).unwrap();
            z.write_all(&zeros).unwrap();
        }
        let (gz, z) = (gz.finish().unwrap(), z.finish().unwrap());
        assert!(gz.len() < 1024 * 1024 && z.len() < 1024 * 1024);
        assert_eq!(inflate(&gz), None);
        assert_eq!(inflate(&z), None);
        assert_eq!(inflate(b"plain"), Some(b"plain".to_vec()));
    }

    /// A 2×2 table a phone made and nobody typed in (iOS, 2026-10-21), gzip + base64.
    const PHONE_BLANK: &[&str] = &[
        "H4sIAAAAAAAC/+NgEJrCx8EgwCDVy8fFxsUCZDJKBWT5cQhIaXEwCKkpqRhAgS4WAgak2DgYhZgMGIE0M5BmBtKs",
        "QJoDSLMBaV4pHi4uiNlAHpOUbpY2B6OUOgeTkLKSonNQSGJSTqpzfk5pbp5LZlFqcklmfp5PalpJSH5QZnpGidR5",
        "xibG04xcixi5qrh8hNje798DRFICYANBLgfTGoxgEUagCJMUmNZgkhLj4gDK/QcCfqA6OFuLiYMRhIVEgF4UcN8/",
        "Zdc0O58NW/8ompg/bY4GijIKCXRslzZpf+khtX1CwmqWOrGjQipcAlxMBiBvsUhBAwoswg4UYYOJIKlixVDFDhOR",
        "4sni4mACBg6LEBMajwmFx4zCY6F1SFy+5dZXeNKjYYXps9Rqi3hhSEiULeB1iFVzXbrz8ZdVur7zYqB+5AL6iBPF",
        "jzxAEW60kACp4sJQxYM1JFhReGwoPHYUHoeUmhHEfE6gaXwo5nMDRQTh5sPUAXUZ8KOoAyVMAbg6vSAdIQYsIYkj",
        "3EhWj+wOIQx3CNPLHUocmSmpeSWZJZVKYslF2DKeEktxak6aEltyUVB+ebESZ2iop4tnXkpqhRJnchFEbbESd3Jq",
        "Tg6UoyWenJ+rl1hQkJOq5xzkEqLnF+xXmpuUWoRFIrikKDMvXUsUQwJki5YYmjCwZCgFcrQ0MMSDUtMzi0tSi3xL",
        "c0oywxJzSlN9UhOLS7SUiVCJYTvQmqLUVC1JrMJ++SmpWNzrkliSiF04VcsQIZyXX5JarOfp7JyTX5oSXJmXDPS9",
        "f1IWMKQdgYFdBowG1zJgdCD7EKbFDegfkI7g1BxIxASXgAxXwlQZEuJYmpKZH5SanF+UAgpebcJq9NyKEtNzQVar",
        "YlMcUpSYV5xclFlQEpwKUSaOrgxabGNKAPWDJIwEHnPIZbx94qQ1MeuT0uaqqWuNBNq5omP9WF339HemNm5q7ftn",
        "hFHKGglcUoqI7r3svah/1rrP+k5J+4wwymcjgVOTnsYvMnO+Nl2xUaJ21UQzI4ySy0jgMMe39l/s/mH7/LPf5135",
        "8tQIoxyzkuGS4sJwpRATsFpi4mAEAHKl+GwZBwAA",
    ];

    /// A table a phone made and then typed into, cell by cell, with a row added and one
    /// removed along the way (iOS, 2026-10-21). Its cells' text is replaced by synthetic text
    /// of the same lengths; every merge record, run and clock is the phone's.
    const PHONE_TYPED: &[&str] = &[
        "H4sIAAAAAAAC/5WXfUwTdxjHe20p16PAcSDMU6CUt7NAxbo450SEIorWoqXo3IvSl4OWlRbKVcAti69TTJyZmc6J",
        "0+iWLYvbPzPuTadzW5YsZm/J4v7YjMl02aIyEufcH4ruud7TjnJH5y7A736f5/t7e+55fj9+pIo5XEyqaBW7v5jS",
        "UVp41TAqdlW3g6RZM6liKkxltfjUKPyJP6yOJBh1LQGlBkoNlGlQUlDqoMxmDRQV611Uqdma7iqSYCtJNVNqKrE5",
        "XW5PkLeFg9GeUFMgwnuFQDhk5zsFV9gZ6PIL7G1iK3GToL4iqBcJqoPRjZ07Az8sHetRnHys5IgYIYCo2VjJqVkm",
        "RtQ0ESOkkeA0bD5Fgv4+PNnQNvFuVpOE+Mvkwbrp1Rvn/zY81vrGiZIKZt3XxzKAEgxtOFZ3/uOTKzefqLGf/fHX",
        "ez8w8yiaUteKa9Wykv/UMZIORIeEiBE9EDJOmLKEKl2m0scJa+imSDX4UMuoJ9XUSTVNUk2bVEtLqunYb8GZFwjq",
        "OEFtouz/y5cP5LcFLTfWOi7ZRg7MO/5e48i8IslvH3158hH3KXvhzvq5dcSbjgJcvwFWm5G0/iwgmZO8JKoMMlWW",
        "opfSk2pkUk2fVKPYCqvUfyb0Rif1nwEkN9F/XCdGck6SjgTCJHRntc7TWqa0bSgk+Hkh4DV6+WDQKPCDQrVRGOrl",
        "fcZwyOg29vrDU7pbD58rOXSzMHQzYqErsWxkFDAtMhqZHlgashz8eGJbHY6QC+EhjZCOqgIICrE0gIpEVSG4TlLp",
        "UVWOfWWDikJWiWNmActAxiHLBGZANgtHENtm4gizaQZHyJoysGZC6DDL6SPnG/vmd7buff/o3pbvlub0+a/d/eWi",
        "DqxaRev1lNZRtJKK1jG06hWtf6JVo2i9hdY0RetfaFVe0W3Rylqc1bD5ymNjCv8kBWaeLDCnJQLT5/QwOiksp4w8",
        "DX6jVIk+EzZzmP03qufNRZ/ZF50b+aDn1uhpj//32Oy3G5xbDEz9A4R/iH/MCBpjXzTgfcboiYQHQsbO8KCxO9rT",
        "O+UEGzCwaJigRBqR5EBiSMSGhIG0kEgTktxEUixBYowlhcSakRUDi6fFUmRFsbSQ2DJkBcD0OEILshJIC4ksR2KC",
        "pJDICiSlkBISsSMpSyTESiTlkBAScSCp4LKRtCKp5Ggkq5BwXA6S1UhmcQwSJxIzl4ukDUkVl4fEhaSam4akHUkN",
        "l49kDRILV4BkLZLZ3ENIHkdSy01Hsg7JHI5F8gQSKzcDyZNI5nIzkTyF5GGuEH3/NJI68H0RsvXIFgErRrYBWT0w",
        "I7IOZIuBlSBzI3sUmAlH9cCpICVBKaqCuCE3gKoMVX2JVClH1QD2ZQNVBbJBZI3AKrHlEJzUUksOVc/BiRWLU1DN",
        "YnOoNHjfRmBTFWdmc2NoexwtB10Vwh1x2AKwGuELcbgMYA3CnXG4FKAF4a44bAY4G+FwHC4BWIsT2g1HCE5oDur2",
        "x3UrQGdF3QEicWzMTbGJaBU3kb8nbp8y652Jm7rMei9l2/toVStaN4+n6nkLWilF6za0Km+L28cnHBcy687xCQeC",
        "zDo8nmpFr6Sc88HxVOt9NeW4h1L2PJJyRYdFq4kM+PiQEBCGTPneiNJ/9iZtPx/sNOm8EWd4oN+kb29vaWoJ+fhB",
        "k94bkbT9pgzx8MCKucAb7rG4e3uDvMXmbHJZHG2OaI+HjygY2oRIINRlniYziKOY8ydhuHpEoWLmZNzJdwX6BT6y",
        "MhoUAmvcwShv5939grn0AZSy0WGYCM+bpytiR9jHK8y3yS24lTFvnvMvDoUFvt/SYrMFw1EfHL1eWH2rpxs83QDO",
        "3gifYclG+BwTVxhv0gzrEVu08UHpw7QJYucmudLlaoj6AmEn7w1HfKJ7q/5bY2mOuLt6xKHLlcSuiDvU740EeoU2",
        "XpIVTJbhvVBugPaiwUpfIYv8o1cbzS913zSd3LT/XSvd137twpqqVueZZwf/yOtYyFrps3s+vHjm58YTw0MvDwwM",
        "f3HISr/16cG6oHbx6NFPOiLrxywuq+yuZ6UXlmXssVau2HDknd2rr3iMn1tlNz8rPey/dGdfiUP19vWrpnv7Xrts",
        "ld19rLTvxo7y+ruLf9p1uX3P6/5T31tlN6EFhdQMSrYSRkMegasyaf8Hd6TLFqEPAAA=",
    ];

    fn phone(parts: &[&str]) -> Vec<u8> {
        use base64::Engine;
        let raw = base64::engine::general_purpose::STANDARD
            .decode(parts.concat())
            .unwrap();
        inflate(&raw).unwrap()
    }

    fn editor(first: u8) -> CellEditor {
        CellEditor {
            replica: [first; 16],
            unix_seconds: 1_792_600_000,
            paragraph: [0x5a; 16],
        }
    }

    fn data(document: &[u8]) -> Vec<u8> {
        let version = fields(live_version(document).unwrap().1).unwrap();
        let data = bytes(&version, 3).next().unwrap().to_vec();
        data
    }

    fn writers(document: &[u8]) -> TableWriters {
        TableWriters::read(bytes(&fields(&data(document)).unwrap(), 7).next().unwrap()).unwrap()
    }

    /// Data fields 1 (version vector) and 6 (uuids), as written.
    fn structure_clocks(document: &[u8]) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        let d = data(document);
        let f = fields(&d).unwrap();
        (
            bytes(&f, 1).map(<[u8]>::to_vec).collect(),
            bytes(&f, 6).map(<[u8]>::to_vec).collect(),
        )
    }

    /// Each object's texts as pieces whose writer is a uuid, not a number: who wrote what.
    /// A piece with its writer as a uuid (none for the markers): writer, clock, length, deleted.
    type Owned = (Option<[u8; 16]>, u64, u64, bool);

    fn attributed(document: &[u8]) -> Vec<Vec<Owned>> {
        let list = writers(document);
        let d = data(document);
        bytes(&fields(&d).unwrap(), 3)
            .map(|o| {
                let (kind, body) = match fields(o).unwrap().first().unwrap() {
                    (k, Wire::Bytes(b)) => (*k, *b),
                    _ => unreachable!(),
                };
                let text = match kind {
                    10 => body.to_vec(),
                    16 => {
                        let a = bytes(&fields(body).unwrap(), 1).next().unwrap();
                        let s = bytes(&fields(a).unwrap(), 1).next().unwrap();
                        bytes(&fields(s).unwrap(), 1).next().unwrap().to_vec()
                    }
                    _ => return Vec::new(),
                };
                read_pieces(&text)
                    .unwrap()
                    .iter()
                    .map(|p| {
                        let who = (p.id.0 > 0).then(|| list.0[p.id.0 as usize - 1].0);
                        (who, p.id.1, p.length, p.deleted)
                    })
                    .collect()
            })
            .collect()
    }

    /// Field numbers in order, a repeat folded into one.
    fn tags(message: &[u8]) -> Vec<u64> {
        let mut out: Vec<u64> = Vec::new();
        for (t, _) in fields(message).unwrap() {
            if out.last() != Some(&t) {
                out.push(t);
            }
        }
        out
    }

    fn cell_text(document: &[u8], row: usize, column: usize) -> Vec<u8> {
        let at = cell_object(document, row, column).unwrap();
        let d = data(document);
        let object = bytes(&fields(&d).unwrap(), 3).nth(at).unwrap().to_vec();
        let text = bytes(&fields(&object).unwrap(), 10)
            .next()
            .unwrap()
            .to_vec();
        text
    }

    fn dominates(after: &TableWriters, before: &TableWriters) -> bool {
        let ge =
            before.0.iter().all(|(u, c)| {
                after.0.iter().find(|(v, _)| v == u).is_some_and(|(_, d)| {
                    d.len() >= c.len() && d.iter().zip(c).all(|(x, y)| x >= y)
                })
            });
        ge && after != before
    }

    #[test]
    fn the_phone_documents_read_as_phone_tables() {
        assert_eq!(
            cells(&phone(PHONE_BLANK)).unwrap(),
            vec![vec![String::new(); 2]; 2]
        );
        let typed = cells(&phone(PHONE_TYPED)).unwrap();
        assert_eq!(typed.len(), 2);
        assert_eq!(typed[1][1], "");
        assert_eq!(typed[0][0].encode_utf16().count(), 6);
        // One writer, the phone, its list sorted as every list Apple writes is.
        assert_eq!(writers(&phone(PHONE_TYPED)).0.len(), 1);
    }

    /// Writers that sort before the phone's (as this installation's 6b… does) and after it.
    /// `document` with the cell at (`row`, `column`) keyed in its column by the row's
    /// element object instead of its identity: one of the shapes `apple_cloud_notes_parser` takes, and
    /// one that read as #289's empty cells did.
    fn keyed_by_element(document: &[u8], row: usize, column: usize) -> Vec<u8> {
        let version = fields(live_version(document).unwrap().1).unwrap();
        let data = fields(bytes(&version, 3).next().unwrap()).unwrap();
        let doc = Doc {
            objects: bytes(&data, 3).collect(),
            keys: bytes(&data, 4)
                .map(|k| String::from_utf8_lossy(k).into_owned())
                .collect(),
            uuids: bytes(&data, 6).collect(),
        };
        let root = doc.custom(0).unwrap();
        let at = |name: &str| object_index(root[name]).unwrap();
        let by_uuid = doc.uuid_objects();
        let (element, identity) = doc.ordered_pairs(at("crRows"), &by_uuid).unwrap()[row];
        let column_identity = doc.ordered(at("crColumns"), &by_uuid).unwrap()[column];
        let inner =
            Doc::dictionary(doc.object(at("cellColumns")).unwrap().1).unwrap()[&column_identity];
        let (_, body) = doc.object(inner).unwrap();
        assert!(
            element < 128 && identity < 128,
            "one-byte indexes keep every length"
        );
        // An entry's key: field 1, an object id of two bytes, its field 6 the index.
        let key = [0x0a, 0x02, 0x30, u8::try_from(identity).unwrap()];
        let found = body.windows(4).position(|w| w == key).unwrap();
        let start = body.as_ptr() as usize - document.as_ptr() as usize + found + 3;
        let mut out = document.to_vec();
        out[start] = u8::try_from(element).unwrap();
        out
    }

    /// #289: a cell keyed by its row's element, not the row's identity, reads and edits as the
    /// same cell — it was read as empty.
    #[test]
    fn a_cell_keyed_by_its_rows_element_is_still_that_cell() {
        let base = phone(PHONE_TYPED);
        let shown = cells(&base).unwrap();
        assert!(!shown[0][0].is_empty());
        let rekeyed = keyed_by_element(&base, 0, 0);
        assert_ne!(rekeyed, base);
        assert_eq!(cells(&rekeyed).unwrap(), shown);
        let edited = with_cell_by(&rekeyed, 0, 0, "Synthetic", &editor(0x10)).unwrap();
        let mut expected = shown.clone();
        expected[0][0] = "Synthetic".into();
        assert_eq!(cells(&edited).unwrap(), expected);
    }

    #[test]
    fn an_edit_to_a_phone_typed_table_reads_back_and_its_clocks_dominate() {
        let base = phone(PHONE_TYPED);
        for first in [0x10, 0x6b, 0xf0] {
            let out = with_cell_by(&base, 1, 1, "Synthetic", &editor(first)).unwrap();
            let mut expected = cells(&base).unwrap();
            expected[1][1] = "Synthetic".into();
            assert_eq!(cells(&out).unwrap(), expected);
            assert!(dominates(&writers(&out), &writers(&base)));
            // And an edit on top of that one dominates it in turn, as the same writer.
            let again = with_cell_by(&out, 1, 1, "Synth", &editor(first)).unwrap();
            assert_eq!(cells(&again).unwrap()[1][1], "Synth");
            assert!(dominates(&writers(&again), &writers(&out)));
            assert_eq!(writers(&again).0.len(), 2);
            // The table's structure clock is a phone's to move, not a text edit's.
            assert_eq!(structure_clocks(&again), structure_clocks(&base));
        }
    }

    #[test]
    fn a_round_trip_keeps_every_phone_replica_and_who_wrote_what() {
        let base = phone(PHONE_TYPED);
        let before = attributed(&base);
        // Into the middle of a typed cell, by a writer that sorts first.
        let old = cells(&base).unwrap()[0][1].clone();
        let mut text = old.clone();
        text.insert_str(4, "XY");
        let out = with_cell_by(&base, 0, 1, &text, &editor(0x01)).unwrap();
        assert_eq!(cells(&out).unwrap()[0][1], text);
        let after = attributed(&out);
        let edited = cell_object(&base, 0, 1).unwrap();
        for (i, (b, a)) in before.iter().zip(&after).enumerate() {
            if i == edited {
                // Every character the phone wrote is still the phone's, live or deleted.
                let live = |v: &[Owned]| {
                    v.iter()
                        .filter(|p| p.0.is_some())
                        .flat_map(|&(w, c, n, d)| (c..c + n).map(move |k| (w, k, d)))
                        .collect::<Vec<_>>()
                };
                let mine = Some([0x01; 16]);
                let theirs: Vec<_> = live(a).into_iter().filter(|p| p.0 != mine).collect();
                assert_eq!(theirs, live(b));
            } else {
                assert_eq!(a, b, "object {i} attributed as the phone wrote it");
            }
        }
        // Every replica entry the phone wrote is kept: its clocks, the structure's clock and
        // its uuids.
        let w = writers(&out);
        for (u, c) in &writers(&base).0 {
            assert_eq!(w.0.iter().find(|(v, _)| v == u).map(|x| &x.1), Some(c));
        }
        assert_eq!(structure_clocks(&out), structure_clocks(&base));
        assert_eq!(w.0[0].0, [0x01; 16], "the new writer at its sorted place");
    }

    /// What a write changes, against what a phone's own edits changed (blank → typed): the
    /// same fields in the same order, at every level, and nothing else.
    #[test]
    fn a_write_mirrors_a_phone_write_field_for_field() {
        let blank = phone(PHONE_BLANK);
        let typed = phone(PHONE_TYPED);
        let ours = with_cell_by(&blank, 0, 0, "Hi there", &editor(0xf0)).unwrap();

        // The document: version vector, the empty field 2 a phone adds, objects, keys, types,
        // uuids, writers.
        assert_eq!(tags(&data(&ours)), tags(&data(&typed)));
        assert_eq!(tags(&data(&ours)), vec![1, 2, 3, 4, 5, 6, 7]);
        assert!(!fields(&data(&blank)).unwrap().iter().any(|(t, _)| *t == 2));

        // The cell: text, records, runs.
        let phone_cell = cell_text(&typed, 0, 0);
        let our_cell = cell_text(&ours, 0, 0);
        assert_eq!(tags(&our_cell), tags(&phone_cell));
        assert_eq!(tags(&our_cell), vec![2, 3, 5]);
        let pieces = |c: &[u8]| -> Vec<Vec<u8>> {
            bytes(&fields(c).unwrap(), 3).map(<[u8]>::to_vec).collect()
        };
        let (p, o) = (pieces(&phone_cell), pieces(&our_cell));
        assert_eq!(p.len(), 3, "start, the typed stretch, end");
        assert_eq!(o.len(), 3);
        for (a, b) in p.iter().zip(&o) {
            assert_eq!(tags(a), tags(b));
            for t in [1, 3] {
                assert_eq!(
                    tags(bytes(&fields(a).unwrap(), t).next().unwrap()),
                    vec![1, 2]
                );
                assert_eq!(
                    tags(bytes(&fields(b).unwrap(), t).next().unwrap()),
                    vec![1, 2]
                );
            }
        }
        // The run: length, paragraph style {9: uuid}, time.
        let runs = |c: &[u8]| -> Vec<Vec<u8>> {
            bytes(&fields(c).unwrap(), 5).map(<[u8]>::to_vec).collect()
        };
        let (pr, or) = (runs(&phone_cell), runs(&our_cell));
        assert_eq!(or.len(), 1);
        assert_eq!(tags(&pr[0]), tags(&or[0]));
        assert_eq!(tags(&or[0]), vec![1, 2, 13]);
        let style = |r: &[u8]| bytes(&fields(r).unwrap(), 2).next().unwrap().to_vec();
        assert_eq!(tags(&style(&pr[0])), vec![9]);
        assert_eq!(style(&or[0]), [&[0x4a, 16][..], &[0x5a; 16]].concat());
        assert_eq!(int(&fields(&or[0]).unwrap(), 13), Some(1_792_600_000));

        // The writers: each entry {1: uuid, 2: {1: chars}, 2: {1: stamps}}, sorted.
        let w = writers(&ours);
        assert_eq!(w.0.len(), 2);
        assert!(w.0.iter().all(|(_, c)| c.len() == 2));
        assert_eq!(w.0[1], ([0xf0; 16], vec![8, 1]));
        assert_eq!(w.0[0], writers(&blank).0[0], "the phone's entry as it was");

        // Nothing else moved: every other object byte for byte, the structure clock too.
        let edited = cell_object(&blank, 0, 0).unwrap();
        let (b, o) = (data(&blank), data(&ours));
        let objects = |d: &[u8]| -> Vec<Vec<u8>> {
            bytes(&fields(d).unwrap(), 3).map(<[u8]>::to_vec).collect()
        };
        for (i, (x, y)) in objects(&b).iter().zip(objects(&o).iter()).enumerate() {
            if i != edited {
                assert_eq!(x, y, "object {i}");
            }
        }
        assert_eq!(structure_clocks(&ours), structure_clocks(&blank));
    }

    /// A deletion is stamped when it is made, as each of the phone's are; the characters stay.
    #[test]
    fn a_deletion_is_stamped_and_kept_as_a_phone_keeps_one() {
        let blank = phone(PHONE_BLANK);
        let one = with_cell_by(&blank, 1, 0, "Hello", &editor(0xf0)).unwrap();
        let two = with_cell_by(&one, 1, 0, "Help", &editor(0xf0)).unwrap();
        assert_eq!(cells(&two).unwrap()[1][0], "Help");
        let pieces = read_pieces(&cell_text(&two, 1, 0)).unwrap();
        let me = 2;
        let deleted: Vec<_> = pieces.iter().filter(|p| p.deleted).collect();
        assert_eq!(deleted.len(), 1);
        assert_eq!(
            (deleted[0].id, deleted[0].length),
            ((me, 3), 2),
            "\"lo\" kept, deleted"
        );
        assert_eq!(deleted[0].stamp, (me, 1), "stamped with the next timestamp");
        let typed = pieces
            .iter()
            .find(|p| !p.deleted && p.id == (me, 5))
            .unwrap();
        assert_eq!((typed.length, typed.stamp), (1, (me, 2)));
        // The runs still cover the text exactly.
        let runs: u64 = bytes(&fields(&cell_text(&two, 1, 0)).unwrap(), 5)
            .map(|r| run_units(r).unwrap())
            .sum();
        assert_eq!(runs, 4);
        assert_eq!(writers(&two).0[1].1, vec![6, 3]);
    }

    /// A list this app once wrote out of order cannot be read the way Apple reads it, so it is
    /// not written to.
    #[test]
    fn an_unsorted_writer_list_is_refused() {
        let mut w = writers(&phone(PHONE_BLANK));
        w.0.insert(0, ([0xff; 16], vec![1, 1]));
        assert!(TableWriters::read(&w.encode()).is_none());
    }
}
