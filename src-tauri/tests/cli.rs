//! The read commands (src/cli.rs) as another program meets them: the built Skykeep run with its
//! output read through pipes, its exit code checked, on every platform CI tests. On Windows the
//! released Skykeep is a windowed program (the test build is a console one): its output reaches
//! a pipe the same way, through the handles the program that started it passed down.

use std::path::Path;
use std::process::{Command, Output};

const IDENTIFIER: &str = "io.github.yashwanthkamireddi.Skykeep";

/// Skykeep run with `args` and every folder it could find a library in pointed at `home`.
fn skykeep(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_quince-app"))
        .args(args)
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("APPDATA", home.join("roaming"))
        .env("LOCALAPPDATA", home.join("local"))
        .output()
        .expect("Skykeep runs")
}

/// Where this platform keeps the library, under `home` as [`skykeep`] sets it up.
fn library_dir(home: &Path) -> std::path::PathBuf {
    let base = if cfg!(windows) {
        home.join("local")
    } else if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else {
        home.join("data")
    };
    base.join(IDENTIFIER)
}

#[test]
fn another_program_reads_skykeep_through_a_pipe() {
    let home = tempfile::tempdir().unwrap();

    // No library yet: told so on stderr, exit 2, nothing on stdout, no window.
    let none = skykeep(home.path(), &["--calendars"]);
    assert_eq!(none.status.code(), Some(2), "{none:?}");
    assert!(none.stdout.is_empty());
    assert!(String::from_utf8_lossy(&none.stderr).contains("no library"));

    let dir = library_dir(home.path());
    std::fs::create_dir_all(&dir).unwrap();
    {
        let mut library = quince_core::Library::open(&dir).unwrap();
        library
            .put_reminder_lists(&[quince_core::reminders::ReminderList {
                guid: "L1".into(),
                title: "Home".into(),
                enabled: true,
                ..Default::default()
            }])
            .unwrap();
    }

    let lists = skykeep(home.path(), &["--reminder-lists"]);
    assert_eq!(lists.status.code(), Some(0), "{lists:?}");
    let json: serde_json::Value = serde_json::from_slice(&lists.stdout).unwrap();
    assert_eq!(json, serde_json::json!([{ "id": "L1", "title": "Home" }]));

    // Asked wrongly: exit 1, the reason on stderr.
    let wrong = skykeep(home.path(), &["--reminders", "nope"]);
    assert_eq!(wrong.status.code(), Some(1), "{wrong:?}");
    assert!(String::from_utf8_lossy(&wrong.stderr).contains("nope"));

    // A create asked for wrongly is refused before anything starts: exit 1, the reason said.
    for bad in [
        &["--create-reminder"][..],
        &["--create-note", " "],
        &["--create-event", "Dentist", "--start", "tomorrow"],
    ] {
        let refused = skykeep(home.path(), bad);
        assert_eq!(refused.status.code(), Some(1), "{bad:?}: {refused:?}");
        assert!(String::from_utf8_lossy(&refused.stderr).starts_with("skykeep: "));
    }
}
