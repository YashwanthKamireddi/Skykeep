//! Which programs make Skykeep download, and how much: said in the diagnostics log.
//!
//! A file in the iCloud folders is downloaded whenever any program on the computer reads it —
//! not only the person opening it, but a search indexer, an antivirus scan or a photo app
//! cataloguing the Pictures library. Read one after another, a library of 57,000 photos was
//! downloaded whole, again and again as the cache let older ones go, at the full speed of the
//! line, while the app said everything was up to date (discussion #167, 23 GB in a day). The log
//! said nothing of it: it noted only failures. Now the first request from each program is noted
//! at once, and every ten minutes how many files and bytes each one asked for. Only the
//! program's file name is noted (`SearchProtocolHost.exe`), never a file's name or path.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How often the totals are said while downloads go on.
pub const EVERY: Duration = Duration::from_secs(10 * 60);

/// Requests and bytes, by program, since the totals were last said.
#[derive(Default)]
pub struct Tally {
    since: Option<Instant>,
    by: HashMap<String, (u64, u64)>,
    seen: std::collections::HashSet<String>,
}

impl Tally {
    /// One request from `program` for `bytes`, at `now`: the notes it makes due, if any.
    pub fn add(&mut self, program: &str, bytes: u64, now: Instant) -> Vec<String> {
        let mut notes = Vec::new();
        if self.seen.insert(program.to_string()) {
            notes.push(format!("{program} is downloading files from the folder"));
        }
        let since = *self.since.get_or_insert(now);
        let entry = self.by.entry(program.to_string()).or_default();
        entry.0 += 1;
        entry.1 += bytes;
        if now.duration_since(since) >= EVERY {
            notes.push(self.said(now.duration_since(since)));
            self.by.clear();
            self.since = Some(now);
        }
        notes
    }

    fn said(&self, over: Duration) -> String {
        let mut rows: Vec<_> = self.by.iter().collect();
        rows.sort_by(|a, b| b.1 .1.cmp(&a.1 .1).then_with(|| a.0.cmp(b.0)));
        let list = rows
            .iter()
            .map(|(program, (files, bytes))| {
                format!("{program} {files} requests ({})", size(*bytes))
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "downloaded in the last {} min, by program: {list}",
            over.as_secs() / 60
        )
    }
}

/// "1.6 GB", "12 MB", "900 KB".
pub fn size(bytes: u64) -> String {
    const KB: u64 = 1_000;
    const MB: u64 = 1_000 * KB;
    const GB: u64 = 1_000 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{} MB", bytes / MB)
    } else {
        format!("{} KB", bytes.div_ceil(KB))
    }
}

/// The program's own file name from its full path, as the log may say it.
pub fn program(image_path: &str) -> String {
    let name = image_path.rsplit(['\\', '/']).next().unwrap_or("");
    if name.is_empty() {
        "an unknown program".to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first request from a program is said at once; the totals every ten minutes, the
    /// biggest first; a program's name only, never its folder.
    #[test]
    fn who_downloads_is_said() {
        let mut t = Tally::default();
        let start = Instant::now();
        let first = t.add(
            &program(r"C:\Windows\System32\SearchProtocolHost.exe"),
            4_000_000,
            start,
        );
        assert_eq!(
            first,
            vec!["SearchProtocolHost.exe is downloading files from the folder"]
        );
        assert!(t
            .add(
                "SearchProtocolHost.exe",
                4_000_000,
                start + Duration::from_secs(5)
            )
            .is_empty());
        let explorer = t.add("explorer.exe", 300_000, start + Duration::from_secs(6));
        assert_eq!(
            explorer,
            vec!["explorer.exe is downloading files from the folder"]
        );
        let due = t.add("SearchProtocolHost.exe", 4_000_000, start + EVERY);
        assert_eq!(
            due,
            vec!["downloaded in the last 10 min, by program: SearchProtocolHost.exe 3 requests (12 MB), explorer.exe 1 requests (300 KB)"]
        );
        // Counted afresh after that, and a known program isn't announced again.
        assert!(t
            .add("explorer.exe", 1, start + EVERY + Duration::from_secs(1))
            .is_empty());
    }

    #[test]
    fn a_missing_path_is_an_unknown_program() {
        assert_eq!(program(""), "an unknown program");
        assert_eq!(program("/usr/bin/nautilus"), "nautilus");
        assert_eq!(size(23_100_000_000), "23.1 GB");
    }
}
