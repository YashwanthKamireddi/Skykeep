# quince-media

Turns what Apple stores into what the interface draws.

- **Decoding:** `heic_to_jpeg`, `is_heif`, `to_png`, `fit`, `long_side`, `thumbnail_bgra`. HEIC
  uses libheif (the system's on Linux, a static vcpkg build on Windows) behind the `heic`
  feature, on by default. `quince-mobile` and `quince-vfs` build without it.
- **Contact pictures:** `Crop` (the square the owner chose) and `avatar`, which cuts it so a
  portrait shows a face, not the middle of the photo. `photo_for_apple` sizes a picture for
  upload.
- **Video:** `video_matrix` and `video_zoom` read rotation and zoom from the file. `poster.rs`
  (Linux, GStreamer) finds the first frame that isn't blank.
- **Desktop thumbnails:** `freedesktop_thumbnail` and its helpers write the shared thumbnail
  cache that file managers read.
- **`appimage.rs`** points libheif at the HEVC decoder the AppImage carries.

Depends on no other workspace crate. Used by `quince-vfs`, the desktop app, `quince-mobile` and
the probe.

## Tests

```sh
cargo nextest run -p quince-media
```

`fixtures/two.heic` is a synthetic test image, and `tests/video_matrix.rs` reads its own
fixtures. The HEIC and video tests need the system codecs (see CONTRIBUTING, Linux desktop).
