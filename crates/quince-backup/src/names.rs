//! File names made safe: a name from Apple or another person is one plain name in a folder,
//! never a path. Moved from the desktop app (main.rs, photos.rs) to be shared with the Mac app.

/// A note attachment's file name as it is saved: Apple's name for it with anything that could
/// make it a path (`/`, `\\`, `:`, `..`), hide it (a leading dot), or show it as something it
/// is not (direction marks that turn `exe.pdf` around) taken out; a name Windows keeps for a
/// device made harmless. Nothing left, and it is named after the attachment.
pub fn attachment_name(given: Option<&str>, id: &str) -> String {
    let cleaned: String = given
        .unwrap_or_default()
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(
                    c,
                    '/' | '\\'
                        | ':'
                        | '*'
                        | '?'
                        | '"'
                        | '<'
                        | '>'
                        | '|'
                        | '\u{200E}'
                        | '\u{200F}'
                        | '\u{202A}'..='\u{202E}'
                        | '\u{2066}'..='\u{2069}'
                        | '\u{FEFF}'
                )
        })
        .collect();
    let trimmed = cleaned
        .trim_start_matches(['.', ' '])
        .trim_end_matches(['.', ' ']);
    // Long enough for any real name, short enough for every file system — and, cut short, it
    // still ends in neither a dot nor a space, which Windows drops (turning `a.pdf.` into
    // another file than the one checked).
    //
    // Counted in bytes, as Linux counts a name (255 at most, with room left for " 2" and
    // ".part"), and cut from the stem so the extension stays: 120 characters dropped ".pdf",
    // and 120 CJK characters or emoji were over the limit and could not be saved at all.
    const LONGEST: usize = 120;
    let cut: String = if trimmed.len() <= LONGEST {
        trimmed.to_string()
    } else {
        let (stem, ext) = name_parts(trimmed);
        let room = LONGEST.saturating_sub(ext.len()).max(1);
        let mut at = room.min(stem.len());
        while !stem.is_char_boundary(at) {
            at -= 1;
        }
        format!("{}{ext}", stem[..at].trim_end_matches(['.', ' ']))
    };
    let mut name = cut.trim_end_matches(['.', ' ']).to_string();
    // Windows reads a device in the stem whatever follows it (`CON.txt`, `CON .txt`), and
    // counts superscript digits as digits (`COM¹`).
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_uppercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix).is_some_and(|n| {
            let mut n = n.chars();
            matches!(
                (n.next(), n.next()),
                (Some('0'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'), None)
            )
        })
    };
    let device = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
    ) || numbered("COM")
        || numbered("LPT");
    if device {
        name.insert(0, '_');
    }
    if name.is_empty() {
        name = format!("Attachment {}", id.chars().take(8).collect::<String>());
    }
    name
}

/// A note attachment's id as part of a file name in `note-media`: letters, digits and dashes,
/// so an id from a request is never a path.
pub fn media_file_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// A path in `dir` for `name` that holds nothing yet: a number goes on the end rather than
/// writing over a file already there — the rule a browser follows.
pub fn free_path(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let (stem, ext) = name_parts(name);
    let mut path = dir.join(name);
    // "Report 2.pdf" for the second copy, as Finder and a folder download (folder_download's
    // `Taken`) number it, so every save names a second copy the same way.
    let mut suffix = 2;
    // Anything at all under the name counts as taken — a link too, even one that leads
    // nowhere: written through, it would put the file wherever the link points.
    while path.symlink_metadata().is_ok() {
        path = dir.join(format!("{stem} {suffix}{ext}"));
        suffix += 1;
    }
    path
}

/// A file name as its stem and its extension (with the dot, or empty): the whole of a
/// compressed tar's (".tar.gz"), and only something that reads as one — short, with no space —
/// so "v1.2 notes" has none.
pub fn name_parts(name: &str) -> (&str, &str) {
    let lower = name.to_ascii_lowercase();
    for double in [".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst"] {
        if lower.len() > double.len() && lower.ends_with(double) {
            let at = name.len() - double.len();
            if name.is_char_boundary(at) {
                return (&name[..at], &name[at..]);
            }
        }
    }
    match name.rfind('.') {
        Some(at)
            if at > 0
                && name.len() - at - 1 <= 16
                && !name[at..].contains(' ')
                && at + 1 < name.len() =>
        {
            (&name[..at], &name[at..])
        }
        _ => (name, ""),
    }
}

/// A record id as one name in a folder. Ids come from CloudKit and the window, and are checked
/// where a request comes in (`requested`) — and here too, where one becomes a path, so no other
/// way in can make one reach elsewhere. A record name is letters, digits, `-` and `_`, kept as
/// it is; anything else is spelled out in hex behind a `~` no record name has.
pub fn cache_name(id: &str) -> std::borrow::Cow<'_, str> {
    if !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return id.into();
    }
    let mut out = String::from("~");
    for b in id.bytes() {
        out.push_str(&format!("{b:02x}"));
    }
    out.into()
}

/// A name made safe to be one file name on every system (see [`attachment_name`]): no
/// path in it, nothing hidden, nothing reversed; `fallback` when nothing is left.
pub fn file_name(given: &str, fallback: &str) -> String {
    let cleaned = attachment_name(Some(given), "");
    // `attachment_name` names an empty one after the id, and the id given was empty.
    if cleaned == "Attachment " {
        return fallback.to_string();
    }
    // Cut short at its length limit, it can end in a space, which Windows will not keep.
    cleaned.trim_end_matches(['.', ' ']).to_string()
}
