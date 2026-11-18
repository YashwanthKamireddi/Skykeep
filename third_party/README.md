# Dependencies kept here with a fix of ours

Each one is the crates.io release as published, with a single change. The change is named here,
along with when to drop the copy and go back to crates.io.

## wry 0.57.2 (`third_party/wry`)

**Change:** `src/webkitgtk/mod.rs`, `attach_ipc_handler`. The IPC signal handler holds the WebView
weakly instead of strongly.

Also, in its `Cargo.toml`: `[lints.rust] warnings = "allow"`. A path dependency's warnings are
shown in every build (crates.io ones are capped); these are upstream's, not ours to fix in a copy.

**Why:** the handler is connected on the WebView's own `UserContentManager`. Capturing the view
strongly makes a cycle (view → manager → handler → view), so on Linux a destroyed window's WebView
is never freed, and neither is its WebKit web process. Every app window opened and closed left one
web process (about 80–125 MB) running until Skykeep quit: seven of them, about 790 MB, measured on a
day's use with no window open. With the change, a closed window's web process exits once the window
is let go (`apps::KEPT`).

Upstream: [tauri-apps/wry#1871](https://github.com/tauri-apps/wry/issues/1871) (still open on 18 Nov
2026; 0.57.2 still captures the view strongly). The copy moved from 0.57.0 to 0.57.2 on 18 Nov for
upstream's own fixes: on Linux a custom-protocol request body cut short on a short read (#1911),
which Tauri's IPC goes through; on Windows long-path drops (#1922), empty drop events after a
refused drag, and a window move blurring the web view (#1910).

**Drop it when** a wry release holds the view weakly in `attach_ipc_handler`. Then:

1. delete `third_party/wry` and the `[patch.crates-io]` entry in `Cargo.toml`,
2. `cargo update -p wry`,
3. run `.uiverify/window-close-race.py`, which checks that closed windows' web processes exit.

`src-tauri/src/apps.rs`'s `wry_is_the_patched_release` test fails as soon as the locked wry is not
the version patched here. A wry upgrade can't silently drop or outlive the fix.
