//! Skykeep's backup and Export Everything, without a window.
//!
//! Moved out of the desktop app (src-tauri) so the macOS app can run the same code through
//! quince-mobile. The desktop app re-exports these modules under their old paths, so its own
//! code reads as it did.

pub mod backup;
pub mod export;
pub mod fetch;
pub mod folder_download;
pub mod names;
pub mod saving;
