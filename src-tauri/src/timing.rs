//! How quickly Skykeep opens, measured: with `SKYKEEP_TIMING=1` each step of a launch, of an
//! app's window and of an opened file is printed to stderr — milliseconds since the process
//! started, and the wall clock, to line up with the launcher's own. Silent otherwise.

use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static START: OnceLock<Instant> = OnceLock::new();

fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SKYKEEP_TIMING").is_some_and(|v| !v.is_empty()))
}

/// The process's start, as near as `main` can say it.
pub fn start() {
    START.get_or_init(Instant::now);
}

/// How long since the process started.
pub fn uptime() -> std::time::Duration {
    START.get_or_init(Instant::now).elapsed()
}

/// One step, printed when timing is asked for.
pub fn mark(what: impl std::fmt::Display) {
    if !on() {
        return;
    }
    let since = START.get_or_init(Instant::now).elapsed();
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    eprintln!(
        "skykeep-timing {wall} +{:.1}ms {what}",
        since.as_secs_f64() * 1000.0
    );
}
