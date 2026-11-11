# Contributing to Skykeep

Bug fixes, features, packaging, design and docs are all welcome. Contributions are licensed
under [GPL-3.0-only](../LICENSE), and the [code of conduct](CODE_OF_CONDUCT.md) applies.

Read [ARCHITECTURE.md](../ARCHITECTURE.md) first. It covers how the engine is laid out, how a
read and a write move through it, and the invariants a change has to keep. This page covers
building, testing and getting a change merged.

For anything bigger than a small fix, open an issue first so the approach is agreed before you
spend the time.

**Contents:** [Set up](#set-up) · [Build and run](#build-and-run) · [Tests](#tests) ·
[Without an Apple Account](#testing-without-an-apple-account) ·
[With one](#testing-with-an-apple-account) · [CI](#ci) · [What is published](#what-is-published) ·
[Code style](#code-style) · [Commits and pull requests](#commits-and-pull-requests) ·
[UI and design](#ui-and-design) · [Where to start](#where-to-start)

## Set up

Everyone needs **Rust** through [rustup](https://rustup.rs) (it installs the version
`rust-toolchain.toml` pins) and **Node.js 22**. Node 20 works too; the Playwright suites
need 20 or newer.

### Linux desktop

A C compiler and the libraries Tauri, libheif, GStreamer, FUSE and the keyring need:

```sh
# Debian 13 / Ubuntu 24.04
sudo apt install build-essential curl nodejs npm \
  libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libsecret-1-dev \
  libheif-dev libfuse3-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev pkg-config \
  libayatana-appindicator3-dev

# Arch
sudo pacman -S --needed base-devel webkit2gtk-4.1 gtk3 libsecret libheif gstreamer \
  gst-plugins-base-libs fuse3 rust nodejs npm pkgconf

# Fedora
sudo dnf install gcc gcc-c++ make nodejs npm \
  webkit2gtk4.1-devel gtk3-devel librsvg2-devel libsecret-devel \
  libheif-devel fuse3-devel gstreamer1-devel gstreamer1-plugins-base-devel pkgconf-pkg-config \
  libayatana-appindicator-gtk3-devel

# openSUSE Tumbleweed
sudo zypper install gcc gcc-c++ make nodejs-default npm-default \
  webkitgtk3-devel gtk3-devel librsvg-devel libsecret-devel \
  libheif-devel fuse3-devel gstreamer-devel gstreamer-plugins-base-devel pkgconf-pkg-config \
  libayatana-appindicator3-devel
```

The appindicator package is only needed for `npx tauri build`. The Linux tray uses ksni, but
tauri-cli sees the `tray-icon` feature the Windows build asks for and stops with "Can't detect
any appindicator library" without it. `./run.sh`, `cargo build` and `./check.sh` don't need it.

A few Rust tests decode real media, so they also need the codecs, not only the headers. On
Debian and Ubuntu that is `gstreamer1.0-plugins-good gstreamer1.0-libav libheif-plugin-libde265`
(the CI list in [`ci.yml`](workflows/ci.yml) is the reference).

Then:

```sh
git clone https://github.com/YashwanthKamireddi/Skykeep.git
cd Skykeep
npm ci
npx playwright install chromium webkit   # for the interface suites
```

When you change dependencies (`Cargo.lock` or `package-lock.json`), run `npm run licences`
(it needs `cargo install --locked cargo-about@0.9.3`) and commit `src/licences.json`: a release
refuses a licence list that is out of date.

### Windows

Install the [Tauri prerequisites](https://tauri.app/start/prerequisites/) (MSVC build tools,
WebView2). HEIC decoding links libheif statically from vcpkg, with the static C runtime to
match, exactly as [`windows.yml`](workflows/windows.yml) does:

```powershell
vcpkg install libheif:x64-windows-static
$env:VCPKG_ROOT = "<your vcpkg folder>"
$env:VCPKGRS_TRIPLET = "x64-windows-static"
$env:RUSTFLAGS = "-C target-feature=+crt-static"
npm ci
npm run build
cargo build -p quince-app
```

The Explorer integration (`crates/quince-vfs/src/cloud.rs`, `crates/quince-thumbs`) only
builds and runs here. Its test registers real sync roots, so it runs on its own:
`cargo test -p quince-vfs --test cloud_windows -- --test-threads=1`.

### Android

- JDK 17, and the Android SDK with platform 37 and the **NDK version pinned in
  `android/app/build.gradle.kts`** (`ndkVersion`, currently `30.0.16248370`):
  `sdkmanager "platforms;android-37" "ndk;30.0.16248370"`. Point `ANDROID_NDK_HOME` at it.
- The phone targets for the pinned Rust:
  `rustup target add aarch64-linux-android x86_64-linux-android`.
- The two build tools Gradle calls, at the versions CI uses:
  `cargo install cargo-ndk --version 4.1.2 --locked` and
  `cargo install cargo-about --version 0.9.3 --locked`.

Gradle builds the engine before every build (`buildEngine`, `engineBindings`,
`engineLicences`; see [ARCHITECTURE](../ARCHITECTURE.md#android-shell)). Engine output goes to
`target-android/`, separate from the desktop's `target/`. The generated `jniLibs/`, Kotlin
bindings and licence file are ignored by git; never commit them.

## Build and run

```sh
./run.sh
```

builds the interface (`npm run build`) and the app (`cargo build -p quince-app`), then opens
`target/debug/quince-app`. There is no dev server: the app always serves its own built bundle,
as a release does. `cargo build` embeds whatever is in `dist/`, so if you build by hand, run
`npm run build` first or you will be looking at a stale interface.

Useful environment variables:

| Variable | Effect |
|---|---|
| `SKYKEEP_LOG=1` | Writes a redacted `diagnostics.log` in the data folder (`src-tauri/src/diagnostics.rs`) |
| `SKYKEEP_TIMING=1` | Prints the timing of each launch step to stderr (`timing.rs`) |

Android:

```sh
cd android
./gradlew assembleDebug        # app, with the engine built first
./gradlew assembleVerify       # the release build (R8) signed with the debug key, installed beside the real app
```

## Tests

### The gate

```sh
./check.sh
```

runs everything a change has to pass, in parallel: the Rust lane beside the web lane, browser
suites in a pool (`CHECK_JOBS` sets its size), and timing-sensitive suites alone at the end.
Besides the setup above it needs `actionlint`, `cargo-nextest`, `python3`, the
`x86_64-pc-windows-gnu` Rust target with a MinGW-w64 C compiler (for the "windows compiles"
step), and optionally `gnome-keyring` with `dbus-run-session` (for the real-keyring test).

What it runs, and how to run each part alone:

| Step | Command |
|---|---|
| Only published paths tracked | see [What is published](#what-is-published) |
| Workflows well formed | `actionlint -shellcheck=` |
| Flatpak offline sources match the lockfiles | `python3 packaging/flatpak/sources-current.py` |
| Rust format | `cargo fmt --all -- --check` |
| Rust lints | `cargo clippy --workspace --all-targets -- -D warnings` |
| Rust tests | `cargo nextest run --workspace --lib --bins --tests --profile ci` |
| Windows code compiles | `cargo check --workspace --tests --target x86_64-pc-windows-gnu` |
| Keyring against a throwaway gnome-keyring | `cargo test -p quince-auth --lib secret_service -- --ignored` (inside the private bus `check.sh` sets up) |
| TypeScript | `npx tsc --noEmit` |
| Model tests | `node --test .uiverify/*.test.mjs` |
| Frontend build | `npm run build` |
| Interface suites | `node .uiverify/<suite>.mjs` against `npx vite preview --port 4173 --strictPort` |

The nextest profiles live in [`.config/nextest.toml`](../.config/nextest.toml): `default` for
local runs, `ci` (no fail-fast, slow tests listed at the end) and `ci-windows` (leaves out the
scale tests and the Explorer test). Tests slower than 30 s are reported and stopped at two
minutes. The scale and crash-stress tests run in limited test groups.

To narrow a run, use `cargo nextest run -p quince-core` or a filter such as
`cargo nextest run -p quince-app outbox`.

### Interface suites

[`.uiverify/`](../.uiverify) holds Playwright scripts that drive the real built interface:
screenshots, keyboard paths, accessibility (axe), narrow windows, other locales and time zones,
and frame-by-frame motion checks. Skykeep draws in WebKitGTK on Linux and WebView2 (Edge) on
Windows, so many suites run in more than one engine:

```sh
npx vite preview --port 4173 --strictPort &
node .uiverify/notes-compose.mjs                   # Chromium
ENGINE=webkit node .uiverify/notes-table.mjs       # WebKit (run .uiverify/webkit-libs.sh once on non-Ubuntu distros)
WEBVIEW2=1 node .uiverify/photos-drag.mjs          # Chromium as the Windows window behaves
ENGINE=msedge node .uiverify/menus.mjs             # Edge, on Windows
```

When you change a screen, run its suites in Chromium and WebKit. When you add behaviour, add
or extend a suite and list it in `check.sh`. CI runs a smaller list (`suites=` in `ci.yml`),
and `check.sh` fails if CI names a suite it doesn't run itself.

**A test must fail without its fix.** Before calling a bug fixed, check that the new test fails
on the old code.

### Android tests

```sh
cd android
./gradlew testDebugUnitTest                         # JVM unit tests
./gradlew assembleDebug assembleDebugAndroidTest    # app and device-test APKs
```

Device tests run with `adb shell am instrument -w io.github.yashwanthkamireddi.skykeep.test/androidx.test.runner.AndroidJUnitRunner`
after installing both APKs. Don't use Gradle's `connected*` tasks: they uninstall the app, and
with it the signed-in account. `SyncScaleTest` uses a throwaway Android account that never
syncs with iCloud. The live tests (`DriveProviderLiveTest` and others) need an emulator signed
in to a test Apple Account.

## Testing without an Apple Account

Most work can be done and tested without signing in:

- **Apple's formats:** `quince-icloud` unit tests feed trimmed, anonymised replies to the same
  decoders the app uses. Add one for any field you parse or write.
- **Sign-in and session:** `crates/quince-auth/tests/` run the SRP and two-factor flows against
  `wiremock` servers. The crate's `loopback` feature lets clients reach `127.0.0.1` and exists
  for tests only.
- **The library:** `quince-core` tests open throwaway databases (`tempfile`), including the
  scale tests in `crates/quince-core/tests/scale.rs`.
- **The queue:** `quince_sync::queue::drain` is `drain_as` without the owner check, for tests
  against a pretend Apple (see the tests in `crates/quince-sync/src/contacts/tests.rs` and
  `src-tauri/src/outbox.rs`).
- **Folders:** `crates/quince-vfs/tests/` mount and write through the engine with a stand-in
  remote.
- **The interface:** every `.uiverify` suite runs against an IPC stub (`.uiverify/stub.mjs`
  replaces `window.__TAURI_INTERNALS__` with sample data shaped like the Rust commands' replies).
  `.uiverify/readme-fixture.mjs` is an invented demo library for screenshots.

## Testing with an Apple Account

Some changes can only be proven against Apple: a new field, a new kind of write, a protocol
change.

- **Use a separate test Apple Account**, never your own data, for anything that writes. Name
  every item you make `Skykeep test …` and delete it afterwards.
  `probe/examples/find_test_leftovers.rs` lists leftovers; there are cleanup probes per service
  (`drive_cleanup_tests`, `reminders_cleanup_tests`, `photos_test_cleanup`).
- **The probe** keeps its own session, apart from the app's: `cargo run -p quince-probe` signs
  in and stores it under `~/.local/share/quince-probe` (`QUINCE_STATE` moves it). The examples
  reuse that session: `cargo run -p quince-probe --example notes_audit`. Many examples write.
  Read one before you run it. They are tools, not tests, and never run in CI.
- **Nothing real is committed.** No names, emails, phone numbers, note text, account ids,
  record names or tokens in code, tests, fixtures, logs or screenshots. Use `example.invalid`
  or `example.com` addresses and 555-01xx numbers. CI runs gitleaks over every commit.
- **Writes not yet proven on real devices** sit behind a `const …: bool = false` (today
  `TABLE_LINE_WRITES` in `src-tauri/src/main.rs`). Don't turn one on without showing the change
  appearing correctly on an iPhone and a second device.

## CI

Pull requests to `main` run [`ci.yml`](workflows/ci.yml). A change to Markdown or
`docs/media/` alone skips it.

| Job | Gates |
|---|---|
| `published` | actionlint; only paths in `published-paths` tracked; Flatpak sources current; gitleaks over the history |
| `rust` (x86_64 and arm64) | `cargo fmt`; Tauri npm package and crate on the same minor version; frontend build; clippy with warnings as errors; `cargo nextest --profile ci` in `TZ=America/Chicago` (a zone with DST, west of UTC); the real-keyring test; a download onto a disk that really fills up |
| `android` | `cargo ndk` engine for both ABIs, bindings, `assembleDebug assembleDebugAndroidTest testDebugUnitTest`; every engine `.so` aligned for 16 KB pages |
| `web` | `tsc`, production build, model tests, the key interface suites in Chromium |

Other workflows run on `main` or for releases, not on pull requests:

| Workflow | When | What |
|---|---|---|
| `windows-checks.yml` | every push to `main` (not for Android, docs or Linux-only packaging) | the full Windows pipeline and the MSIX |
| `windows.yml` | by hand, or called by the above and by releases | tests with `--profile ci-windows`, the Explorer test, `.msi` and `.exe`, a silent install and launch; Edge suites on request |
| `linux-compat.yml` | by hand and on tags | each package installed and started in clean containers of the main distributions |
| `tag.yml`, `release.yml` | maintainers | tags only a commit whose CI and Windows checks passed, then builds and drafts the release |
| `appimage.yml`, `flatpak.yml`, `snap.yml`, `packages.yml`, `msix.yml`, `stores.yml` | releases and by hand | packaging |

Every third-party action is pinned to a commit SHA, and `.uiverify/workflows-pinned.test.mjs`
fails otherwise.

## What is published

[`published-paths`](published-paths) lists, as extended regular expressions, every path this
repository may track. CI, `check.sh` and the release fail on any tracked file outside it. It
exists because a broad `git add` once published files that were never meant to be public.

- Stage explicit paths. Don't use `git add -A` or `git add .`.
- Check before you push:
  `git ls-files | grep -vxE -f <(grep -vE '^(#|$)' .github/published-paths)` should print
  nothing.
- A new file under an existing pattern (`crates/…`, `src/…`, `android/…`) needs no change. Add
  a pattern only for something genuinely new and meant to be public, and say why in the pull
  request.

## Code style

- **Match the code around you.** Rust is `rustfmt` and clippy clean with `-D warnings`.
  TypeScript is strict (`tsc --noEmit`). Kotlin follows the existing files.
- **Comments say why.** Write plain prose about the reason, the constraint or what went wrong
  before, not a restatement of the code. Module docs (`//!`) open each file with what it is
  for. When you fix a bug, the comment by the fix says what happened, so nobody undoes it.
- **No new network hosts, telemetry or analytics.** Every client comes from one of the four
  builders in `quince-auth` ([invariant 5](../ARCHITECTURE.md#invariants)).
- **Logs carry nothing personal.** Use `quince_sync::quiet!` for debug-only messages. Never log
  names, ids, URLs with queries or Apple's replies.
- **New Apple fields:** parse in `quince-icloud` with a test, store in `quince-core` (a new
  migration in `schema.rs` if it needs a column), expose through a command and
  `src/lib/ipc.ts`, show it, and cover it in a suite. If it can be edited, write it through the
  outbox with its own kind, `lay` and `send`.
- **Dependencies:** ask in the issue before adding one. The Android build fails on any licence
  `about.toml` doesn't accept.

## Commits and pull requests

Subjects start with the area, then say what is now true:

```text
Notes: tables keep their rows after an empty version
Calendar: a stalled window is asked again, each read limited to 20 seconds
Android Reminders: lists, Today, Scheduled, Flagged, All and Completed read from the phone's own library
CI: every commit on main keeps its own run; only a pull request's older run is cancelled
```

- The area is the app or part changed (`Notes`, `Reminders`, `Calendar`, `Contacts`, `Drive`,
  `Photos`, `Sync`, `quince-sync`, `Android`, `Windows build`, `CI`, `Snap`, `Packaging`,
  `Docs`).
- Describe the result, not the activity ("tables keep their rows", not "fix table bug").
- The body explains why, and what you verified and how.
- One logical change per commit.

Pull requests:

1. Branch from `main` and keep the change focused.
2. Run `./check.sh`, or at least the steps for what you touched, until it is green. Android
   changes: `./gradlew assembleDebug testDebugUnitTest` too.
3. Fill in the template: what and why, how you checked it, and before/after screenshots in
   light and dark for UI changes.

## UI and design

Skykeep follows the look and keys of Apple's own apps.

- Start with an issue: a sketch or screenshot and the problem it solves.
- Use the shared components in `src/components` (Toast, Confirm, EmptyState, PaneTitle,
  RowMenu and the rest) and the styles in `src/styles`, not one-off styles.
- Light and dark, always. Honour Reduce Transparency and Increase Contrast.
- No animation on large layers (whole panes, viewers, sheets). WebKitGTK on some Wayland
  compositors flickers, so these open and close instantly. Small, local motion is fine if it
  passes the frame-sampling suites in WebKit.
- Glass only on chrome: sidebars, toolbars, menus and sheets. Content is never translucent.
- Keyboard first: the keys of Apple's matching app, with Ctrl for ⌘.
- Copy is short and true, never implies Apple affiliation, and uses no fruit imagery.
- Formats (week start, 12/24-hour time, dates) follow the system. Translations aren't planned,
  so please don't open pull requests adding them.

## Where to start

Each item below has been checked against the code as it is now. Comment on the issue (or open
one) before starting so two people don't take the same thing.

**Rust**

- **`skykeep --help` and `--version`.** Today `--help` starts the app. Parse both first thing in
  `main()` (`src-tauri/src/main.rs`), before the updater and the single-instance plugin, and
  list the flags the app reads: `--app` and `--new` (`apps.rs`), `--background` and
  `--search-provider` (`login.rs`), `--send-to-iphone` (`send.rs`). Take the version from
  `CARGO_PKG_VERSION`. Unit test the parsing in `apps.rs`.
- **Flatpak: send waiting changes as soon as the network returns.** `watch_network()` in
  `src-tauri/src/outbox.rs` listens to NetworkManager and connman on the system bus, which the
  Flatpak sandbox doesn't expose. Add a watcher on the `org.freedesktop.portal.NetworkMonitor`
  portal (session bus), routed through `came_online()` so one reconnection wakes the outbox
  once, with a unit test for the mapping.
- **Contacts CSV: phone and email types.** `csv_to_vcards()` in
  `crates/quince-icloud/src/contacts.rs` writes every number as `TYPE=CELL`. Use the column
  heading (Home, Business/Work, Mobile; Google's `… - Label` / `… - Type` columns). Test with
  synthetic rows only.
- **Contacts import: other encodings and `;` separators.** `contacts_import` in `main.rs`
  reads with `read_to_string` (fails on anything not UTF-8) and `intake.rs` `load()` uses
  `from_utf8_lossy`. One shared decoder for UTF-8 with or without BOM, UTF-16 with BOM and a
  Windows-1252 fallback; `csv_rows()` splitting on `;` when the header uses it.
- **Mounted Drive: no synchronous re-list per change.** In `crates/quince-vfs/src/fuse.rs`,
  `change()` waits for Apple and then `relist()`s every touched folder on the FUSE thread, so
  `rm -rf` of a large folder costs one full listing per file. Apply the change to the library
  locally and reconcile later, with a test that counts listings.
- **CloudKit walks past 100,000 records.** `ZoneClient::all_records` in
  `crates/quince-icloud/src/cloudkit.rs` buffers every page in memory and stops at
  `MAX_PAGES` × `PAGE_SIZE`. Streaming pages into the library, one transaction per page,
  removes both limits. Large design change: discuss it in an issue first.

**Interface**

- **Drive: Ctrl+Shift+N makes a new folder**, as Finder does (Notes already has it). In
  `src/screens/DriveView.tsx`, open the New Folder field in the folder shown; do nothing in
  Recents or Recently Deleted, while renaming, or with a sheet open (`overlayOpen()`). Add it to
  `ShortcutsSheet.tsx` and `.uiverify/drive-keys.mjs`.
- **Contacts: ask before discarding an edited card.** `startNew()` and Ctrl+N in
  `src/screens/ContactsView.tsx` replace the draft without checking it. Compare against
  `draftBase` and ask through `useConfirm()` only when something changed.
- **A screen-reader walkthrough** with Orca or NVDA, using a test account: sign in, switch apps,
  write a note, tick a reminder, add an event, open a contact, browse Drive. Each problem
  reported with steps becomes its own issue.

**Packaging**

- **A man page**, `packaging/skykeep.1`, matching the `--help` text, installed by the Arch and
  AUR PKGBUILDs and the deb/rpm file lists in `src-tauri/tauri.conf.json`.
- **A Nix flake** that builds with nixpkgs' Tauri tooling and installs what
  `packaging/arch/PKGBUILD` installs. A copy built without Tauri's bundler already leaves
  updates to the package manager (`install_kind` in `src-tauri/src/updates.rs`).
