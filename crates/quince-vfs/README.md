# quince-vfs

iCloud Drive and Photos as folders other programs can use, fetched on demand.

- **The platform-free engine** (`lib.rs`): `Inodes` maps items to inode numbers and `Cache`
  keeps file bytes in a sparse on-disk cache, in chunks, each fetched once and only when read.
  Names, sizes and dates come from the library, so browsing never waits on the network.
- **`Remote`** and **`Store`** are the traits the app implements: `Remote` lists folders and
  reads byte ranges from Apple, `Store` answers from the library.
- **`fuse.rs`** (Linux, `fuser`) serves the folder as a FUSE mount on its own threads.
- **`cloud.rs`, `cloud_local.rs`, `transfer.rs`** (Windows) use the Cloud Files API, the one
  OneDrive is built on. Placeholders come from the library, and callbacks only queue work.
- **`writes.rs`**: a file saved through the folder is kept locally and sent once the last
  writer closes it, into the same file at Apple, or beside it as a conflicted copy if it changed
  elsewhere. The Android Files provider uses this too.

Depends on `quince-core`, `quince-media` (HEIC off here; the desktop app turns it on) and
`quince-thumbs` (the Explorer pipe protocol). Used by `quince-app` (`mount.rs`,
`mount_photos.rs`), `quince-mobile` and the probe.

## Tests

```sh
cargo nextest run -p quince-vfs
```

`tests/mount.rs` mounts a made-up Drive through real FUSE and is skipped where FUSE is missing.
`tests/writes.rs`, `changes.rs`, `keep.rs` and `packages.rs` cover write-back and the cache.
`tests/cloud_windows.rs` registers real sync roots, so it runs only on Windows, one test at a
time.
