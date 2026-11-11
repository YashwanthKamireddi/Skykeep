# quince-mobile

Skykeep's engine for Android: the Kotlin app's only way into the Rust crates, through UniFFI.

Calls are coarse. Each one crosses the bridge once and does its work in Rust, so the app never
drives the engine step by step.

- **`start(secrets)`** hands the engine the Android Keystore (`SecretStore`, implemented in
  Kotlin by `KeystoreSecrets.kt`), which stands in for the desktop keyring through
  `quince_auth::PlatformSecrets`.
- **`Account`** (a `uniffi::Object`) wraps the desktop's own `quince_auth::Client`: sign-in,
  two-factor, `reopen`. It also has the Contacts and Calendar calls the sync adapters use
  (`contacts`, `save_contacts`, `save_lists`, `calendar_window`, `save_events`), and, from the
  submodules, Notes, Reminders, Photos and Drive.
- **Libraries:** one per account under `libraries/<account tag>/` in the app's files, opened
  through `quince_sync::library_lock::LibraryLock`. Queued changes go out through
  `quince_sync::queue::drain_as`, the same code the desktop runs.
- **Runtime:** one tokio runtime for the process, two workers. Exported calls block on it, and
  the Kotlin side calls them off the main thread.
- **Logging:** `log::redact` cuts every URL at its `?` before anything reaches logcat.

Built with `cargo ndk` by the Gradle task `buildEngine` (`android/app/build.gradle.kts`). The
Kotlin bindings are written by `crates/uniffi-bindgen`, which reads the built library. Neither
output is committed. `quince-media` is built without HEIC here.

## Tests

```sh
cargo nextest run -p quince-mobile
cd android && ./gradlew testDebugUnitTest
```

The host build runs the Rust unit tests (log redaction). Behaviour on a phone is covered by the
device tests in `android/app/src/androidTest`.
