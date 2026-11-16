//! Diagnostics, for a problem that only shows on one person's account — in a release build too,
//! and only when they turn it on: start Skykeep with `SKYKEEP_LOG=1`.
//!
//! It writes `diagnostics.log` in Skykeep's data folder: each request's host and path (ids put
//! as `…`, never the query), each answer's status and Apple's error codes, which cookies went
//! and came back by name, whether the jar kept each, and the account flags that explain a
//! session that will not work (new terms waiting, web access off). Never a cookie's value, a
//! token, a password, a name, an address or anything from the library — see
//! `quince_auth::diag`, which every line passes through. Nothing is sent anywhere: the person
//! reads it, and attaches it to an issue if they choose.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// The log's name in the data folder.
pub const FILE_NAME: &str = "diagnostics.log";

/// Past this, the log starts over: a run left on for days never fills the disk.
const MOST_BYTES: u64 = 4 * 1024 * 1024;

static LOG: OnceLock<Mutex<Log>> = OnceLock::new();

struct Log {
    path: PathBuf,
    file: Option<std::fs::File>,
}

/// Whether the person asked for diagnostics this run.
fn asked() -> bool {
    std::env::var("SKYKEEP_LOG").is_ok_and(|v| {
        let v = v.trim().to_ascii_lowercase();
        v == "1" || v == "true" || v == "yes" || v == "on"
    })
}

/// A line in the log, when it is on; nothing otherwise.
pub fn note(text: &str) {
    if LOG.get().is_some() {
        line(text);
    }
}

/// Turn diagnostics on for this run, when the person asked, writing into `dir`.
pub fn start(dir: &Path) {
    if !asked() {
        return;
    }
    let path = dir.join(FILE_NAME);
    if LOG
        .set(Mutex::new(Log {
            path: path.clone(),
            file: open(&path),
        }))
        .is_err()
    {
        return;
    }
    quince_auth::diag::set_sink(line);
    quince_vfs::set_notes(line);
    quince_icloud::set_notes(line);
    quince_icloud::set_diagnostics(refused);
    line(&format!(
        "diagnostics on: Skykeep {} on {} {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    eprintln!("Skykeep diagnostics: {}", path.display());
}

fn open(path: &Path) -> Option<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Readable by this user alone, like everything else in the data folder.
        options.mode(0o600);
    }
    options.open(path).ok()
}

/// Write one line, with the time.
fn line(text: &str) {
    let Some(log) = LOG.get() else { return };
    let mut log = log
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let too_big = log
        .file
        .as_ref()
        .and_then(|f| f.metadata().ok())
        .is_some_and(|m| m.len() > MOST_BYTES);
    if too_big {
        let _ = std::fs::write(&log.path, b"");
        log.file = open(&log.path);
    }
    let at = jiff::Timestamp::now()
        .round(jiff::Unit::Second)
        .unwrap_or_else(|_| jiff::Timestamp::now());
    if let Some(file) = log.file.as_mut() {
        let _ = writeln!(file, "{at} {text}");
    }
}

/// A refusal from one of the iCloud services: its status and Apple's codes, nothing more.
fn refused(context: &str, status: u16, body: &str) {
    line(&format!(
        "  {context} {status}: {}",
        quince_auth::diag::codes(body)
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_off_unless_asked_for() {
        // The test runner never sets it; a build that logged unasked would write here.
        if std::env::var_os("SKYKEEP_LOG").is_none() {
            assert!(!asked());
        }
    }
}
