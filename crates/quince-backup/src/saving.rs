//! Files from iCloud saved where the person keeps their own: Downloads, or a folder they chose.
//! A name is whoever shared the file's to choose — a shared folder, a shared album — so it is
//! made one plain file name before it touches a path, the path is checked to be in that folder
//! before anything is written, and on Windows the file is marked as come from the internet, so
//! SmartScreen and Office's Protected View still look at it.

use std::path::{Component, Path, PathBuf};

/// Where `given` is saved in `dir`: a plain name (see [`crate::names::attachment_name`]), `fallback`
/// when nothing is left of it, with a number on the end rather than over a file already there.
/// Refused — never a path elsewhere — when the result would not sit directly in `dir`.
pub fn save_path(dir: &Path, given: &str, fallback: &str) -> Result<PathBuf, String> {
    let name = crate::names::file_name(given, fallback);
    let path = crate::names::free_path(dir, &name);
    if !directly_in(dir, &path) {
        return Err("That file's name can't be saved here.".into());
    }
    Ok(path)
}

/// Whether `path` is one plain name directly inside `dir`: no folder on the way, no `..`, no
/// drive or root that would set `dir` aside, no stream (`name:stream`) on Windows.
pub fn directly_in(dir: &Path, path: &Path) -> bool {
    let Ok(rest) = path.strip_prefix(dir) else {
        return false;
    };
    let mut parts = rest.components();
    let (Some(Component::Normal(name)), None) = (parts.next(), parts.next()) else {
        return false;
    };
    let name = name.to_string_lossy();
    !name.contains(['/', '\\', ':']) && name != "." && name != ".."
}

/// The stream Windows reads to know a file came from the internet: zone 3, as a browser writes.
// Read only where Windows writes it (and in tests).
#[cfg_attr(not(windows), allow(dead_code))]
pub const ZONE_IDENTIFIER: &str = "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=about:internet\r\n";

/// Mark a file saved from iCloud as come from the internet (Mark of the Web). Without it a
/// document from a stranger's shared folder opened straight into Office with its macros, and
/// a program ran without SmartScreen asking. Nothing to do elsewhere: no other system reads it.
/// Best effort — a drive that keeps no streams (FAT, a network share) simply has none.
pub fn mark_from_internet(path: &Path) {
    #[cfg(windows)]
    {
        // Writing the stream counts as writing the file: Windows moved its last-modified date to
        // now, and a file downloaded with its iCloud date (drive::download_with_progress) lost
        // it. The date is read first and put back after, so marking a file changes nothing else.
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        let _ = std::fs::write(PathBuf::from(stream), ZONE_IDENTIFIER);
        if let Some(when) = modified {
            let _ = std::fs::File::options()
                .write(true)
                .open(path)
                .and_then(|f| f.set_modified(when));
        }
    }
    #[cfg(not(windows))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shared file's name is saved as a name in Downloads, whatever its sender called it.
    #[test]
    fn a_shared_name_stays_in_its_folder() {
        let dir = std::env::temp_dir().join(format!("skykeep-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (given, want) in [
            ("Report.pdf", "Report.pdf"),
            (
                "..\\..\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Startup\\update.bat",
                "AppDataRoamingMicrosoftWindowsStart MenuProgramsStartupupdate.bat",
            ),
            ("../../.config/autostart/x.desktop", "configautostartx.desktop"),
            ("/etc/passwd", "etcpasswd"),
            ("C:\\Windows\\evil.dll", "CWindowsevil.dll"),
            ("C:evil.txt", "Cevil.txt"),
            ("report.pdf:evil.exe", "report.pdfevil.exe"),
            ("CON", "_CON"),
            ("nul.txt", "_nul.txt"),
            ("COM1.pdf", "_COM1.pdf"),
            ("LPT9", "_LPT9"),
            ("CONIN$", "_CONIN$"),
            ("conout$.log", "_conout$.log"),
            ("COM\u{b9}.txt", "_COM\u{b9}.txt"),
            ("CON .txt", "_CON .txt"),
            ("trailing. . ", "trailing"),
            ("..", "Download"),
            ("", "Download"),
            ("\u{202E}fdp.exe", "fdp.exe"),
        ] {
            let at = save_path(&dir, given, "Download").unwrap();
            assert_eq!(at.parent(), Some(dir.as_path()), "{given:?}");
            assert_eq!(at.file_name().unwrap().to_string_lossy(), want, "{given:?}");
            assert!(directly_in(&dir, &at), "{given:?}");
        }
        // A long name cut to length never ends in a dot or a space Windows would drop.
        let at = save_path(&dir, &format!("{}. x", "a".repeat(119)), "Download").unwrap();
        let name = at.file_name().unwrap().to_string_lossy().into_owned();
        assert!(!name.ends_with(['.', ' ']), "{name:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn only_a_plain_name_directly_inside_counts() {
        let dir = Path::new("/home/someone/Downloads");
        assert!(directly_in(dir, &dir.join("a.pdf")));
        assert!(!directly_in(dir, &dir.join("sub/a.pdf")));
        assert!(!directly_in(dir, &dir.join("../a.pdf")));
        assert!(!directly_in(dir, Path::new("/etc/a.pdf")));
        assert!(!directly_in(dir, dir));
        assert!(!directly_in(dir, &dir.join("a.pdf:stream")));
        assert!(!directly_in(dir, &dir.join("a\\b.pdf")));
    }

    /// A file already there, or a link left in its place, is never written through.
    #[cfg(unix)]
    #[test]
    fn a_link_in_the_way_is_never_followed() {
        let dir = std::env::temp_dir().join(format!("skykeep-link-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink("/nonexistent/target", dir.join("a.pdf")).unwrap();
        let at = save_path(&dir, "a.pdf", "Download").unwrap();
        assert_eq!(at, dir.join("a 2.pdf"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_mark_says_internet_zone() {
        assert!(ZONE_IDENTIFIER.starts_with("[ZoneTransfer]\r\n"));
        assert!(ZONE_IDENTIFIER.contains("ZoneId=3\r\n"));
    }

    #[cfg(windows)]
    #[test]
    fn a_saved_file_carries_the_mark_on_windows() {
        let dir = std::env::temp_dir().join(format!("skykeep-motw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.docx");
        std::fs::write(&file, b"PK").unwrap();
        mark_from_internet(&file);
        let mut stream = file.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        assert_eq!(
            std::fs::read_to_string(PathBuf::from(stream)).unwrap(),
            ZONE_IDENTIFIER
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
