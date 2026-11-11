# quince-thumbs

Windows only. Two things in one crate:

- **A DLL that File Explorer loads** (`cdylib`): a thumbnail provider (`provider.rs`) for photos
  in the iCloud Photos folder that aren't on disk yet, and the Send to iPhone command in Windows
  11's context menu (`menu.rs`) for the MSIX package.
- **The pipe protocol** (`protocol`), shared with the app (`src-tauri/src/thumbs.rs`) so the
  two sides can't disagree. A request carries the size and the file path, and the reply is BGRA
  pixels. There is one pipe per Windows session (`pipe_name`).

The DLL runs inside Explorer, so it does as little as possible: no decoding, no network, every
entry point catches panics and every wait is bounded. With Skykeep closed the pipe is gone and
Explorer draws its usual icon.

Used by `quince-vfs` and the desktop app. Built with `cargo build --release -p quince-thumbs`
and shipped in the installers.

## Tests

```sh
cargo nextest run -p quince-thumbs
```

The protocol tests run anywhere. The end-to-end check (register the provider, ask for a
thumbnail as Explorer does) is part of `quince-vfs`'s `cloud_windows` test on Windows.
