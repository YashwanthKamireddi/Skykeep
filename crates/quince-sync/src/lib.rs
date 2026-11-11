//! A service's sync, shared by the desktop app and the phone's engine (see Cargo.toml).

pub mod calendar;
pub mod contacts;
pub mod library_lock;
pub mod notes;
pub mod pass;
pub mod photos;
pub mod queue;
pub mod reminders;

/// What went wrong, said on the terminal in a development build only, and never with anything
/// of the person's in it — no names, record ids, addresses or Apple's answers. A release build
/// says nothing at all: Skykeep keeps no diagnostics and sends none anywhere.
#[macro_export]
macro_rules! quiet {
    ($($t:tt)*) => {
        if cfg!(debug_assertions) {
            eprintln!($($t)*);
        }
    };
}
