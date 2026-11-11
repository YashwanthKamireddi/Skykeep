# Contributing to Skykeep

Thank you for helping make Skykeep better. Whether it's a typo, a bug fix, a packaging recipe, a design idea or a new feature, it's welcome here.

Contributions are licensed under the project's license, [GPL-3.0-only](../LICENSE). Please be kind to each other.

**Contents:** [Ways to help](#ways-to-help) · [Set up](#set-up) · [Run it](#run-it) · [The checks](#the-checks) · [Code map](#code-map) · [Testing with iCloud](#testing-with-icloud) · [Ground rules](#ground-rules) · [UI and design](#ui-and-design) · [Commit style](#commit-style) · [Pull requests](#pull-requests)

## Ways to help

- **Bugs:** reproduce an open issue, or confirm a fix on your distribution or desktop.
- **Packaging:** Nix, Fedora COPR, Debian, Snap, winget. Each one brings Skykeep to more people.
- **Desktop integration:** GNOME, KDE and Windows shell integration.
- **Design:** see [UI and design](#ui-and-design).
- **Docs:** clearer install steps, FAQ answers, screenshots made with test data.

For anything bigger than a small fix, please open an issue first so we can agree on the approach before you invest the time.

## Set up

You need **Rust** ([rustup](https://rustup.rs) installs the version `rust-toolchain.toml` pins), **Node.js 22** (20 or newer runs everything; Ubuntu 24.04's own Node 18 builds the app, but the Playwright suites need 20+), a C compiler and the system libraries Tauri uses.

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

The appindicator package is only for `npx tauri build`, the installable packages: it is not
linked (the Linux tray is ksni's), but tauri-cli sees the `tray-icon` feature the Windows and
macOS builds ask for and stops with "Can't detect any appindicator library" without it.
`./run.sh`, `cargo build`, `./check.sh` and the Arch `makepkg` don't need it (on Arch, `npx tauri
build` needs `libayatana-appindicator`).

Then:

```sh
git clone https://github.com/YashwanthKamireddi/Skykeep.git
cd Skykeep
npm ci
npx playwright install chromium webkit   # for the interface suites
```

When you change dependencies (Cargo.lock or package-lock.json), run `npm run licences` (it needs `cargo install --locked cargo-about@0.9.3`) and commit `src/licences.json`: a release refuses a licence list that is out of date.

On Windows, follow the [Tauri prerequisites](https://tauri.app/start/prerequisites/) and install libheif through vcpkg, exactly as [`.github/workflows/windows.yml`](workflows/windows.yml) does.

## Run it

```sh
./run.sh
```

builds the interface and the app and opens Skykeep. There's no dev server: the app always serves its own built bundle, the same way a release does.

## The checks

One command runs everything a change has to pass:

```sh
./check.sh
```

It runs, in order:

| Step | Command |
|---|---|
| Rust formatting | `cargo fmt --all -- --check` |
| Rust lints | `cargo clippy --workspace --all-targets -- -D warnings` |
| Rust tests | `cargo test --workspace` |
| TypeScript | `npx tsc --noEmit` |
| Model tests | `node --test .uiverify/*.test.mjs` |
| Frontend build | `npm run build` |
| Interface suites | `node .uiverify/<suite>.mjs`, against `npx vite preview --port 4173` |

The interface suites in [`.uiverify/`](../.uiverify) are Playwright scripts that drive the real built interface: screenshots, keyboard paths, accessibility (axe), narrow windows, other locales and time zones, and frame-by-frame motion checks (a list that flickers fails). Each runs in Chromium by default; Skykeep draws in WebKit on Linux, so many also run with `ENGINE=webkit`:

```sh
npx vite preview --port 4173 --strictPort &
node .uiverify/notes-compose.mjs
ENGINE=webkit node .uiverify/notes-table.mjs     # WebKit (run .uiverify/webkit-libs.sh once on non-Ubuntu distros)
ENGINE=msedge node .uiverify/menus.mjs           # Edge, on Windows (WebView2's engine)
```

When you change a screen, run its suites in both engines. When you add behaviour, add or extend a suite.

**A test must fail without its fix.** Before you call a bug fixed, check that the new test fails on the old code.

The Windows pipeline (the full test suite, every interface suite in Edge, the installers and an install-and-launch smoke test) runs from Actions → Windows build, and on every release.

## Code map

```text
crates/quince-auth/    Sign-in (Apple's SRP + two-factor), session, keyring, the network locks (apple_only, listed_only, releases_only)
crates/quince-icloud/  Each iCloud service: CloudKit, Notes (topotext CRDT, tables), Reminders,
                       Calendar, Contacts, Drive, Photos, push channel
crates/quince-core/    Local library (SQLite), sync, outbox, search, export
crates/quince-vfs/     iCloud Drive and Photos as folders (FUSE, Linux)
crates/quince-media/   Picture decoding and thumbnails (HEIC included)
src-tauri/src/         The app: windows, Tauri commands, alerts, tray, search provider, import
src/screens/           One screen per app, plus sign-in and Settings
src/components/        The shared design system
src/lib/               IPC, sync state, optimistic writes (outbox, pending), theming
probe/examples/        Live checks against a real iCloud account; never part of CI
packaging/             Arch, AUR, Flatpak, desktop entries, AppStream metadata
```

The crates keep the `quince-` prefix from the project's working name.

**Adding a field Apple sends**, roughly:
1. Parse it in the service module in `crates/quince-icloud`, with a unit test from a trimmed, anonymised sample.
2. Store it in `crates/quince-core` (add a migration in `schema.rs` if it needs a column).
3. Expose it through a command in `src-tauri/src` and its type in `src/lib/ipc.ts`.
4. Show it in the screen, and cover it in a `.uiverify` suite.
5. If it can be edited, write it through the outbox so it's instant, works offline, and rolls back with a reason if Apple refuses it.

## Testing with iCloud

- **Use a test Apple Account**, never your personal data, for anything that writes. Name test items clearly (for example "Skykeep test …") and clean them up afterwards.
- Live checks live in `probe/examples` and are run on purpose, for example:
  ```sh
  cargo run -p quince-probe --example notes_audit
  ```
  They ask whether the account and the local copy agree, right now, about real data.
- Features whose writes aren't proven on a real iPhone yet sit behind a constant that is `false` (for example `TABLE_CELL_WRITES`, `NOTE_ATTACH_WRITES`, `REMINDER_DETAIL_WRITES`). Don't turn one on in a pull request without evidence that the change appears correctly on an iPhone and a second device.

## Ground rules

- **Privacy is the product.** Never add a network host, telemetry, analytics or crash reporting. Every client is built by one of four locks in `quince-auth`: `apple_only()` for iCloud, `listed_only()` for calendar subscriptions, `account_only()` for a calendar account the person added, and `releases::releases_only()` for Skykeep's own updates (GitHub's release servers only). `crates/quince-auth/tests/apple_only.rs` must keep passing.
- **No personal data** in code, tests, fixtures, logs or screenshots: no real names, emails, phone numbers, note contents, account IDs or tokens. A pre-commit secret scan is a good idea; CI-safe test vectors use `example.invalid`.
- **Instant everywhere.** Every write shows immediately and rolls back with a reason on failure. Never block a redraw on the network.
- **Never lose data.** Merges keep both sides; deletes go to Recently Deleted where Apple has one.
- **Copy is short, positive and true.** No "…" on labels, no rows that add nothing, and never imply Apple affiliation.
- **Localisation:** formats (week start, 12/24-hour time, dates) follow the system. Translations aren't planned yet, so please don't open pull requests adding them.

## UI and design

Designers are very welcome. Skykeep aims to feel like Apple's own apps: calm, familiar and quietly premium.

- **Start with an issue**: a sketch, mockup or screenshot and the problem it solves. Agreeing on the direction first saves everyone time.
- **Use the design system** in `src/components` (Toast, Confirm, EmptyState, PaneTitle, RowMenu and the rest) rather than one-off styles. Styles live in `src/styles`.
- **Light and dark, always.** Honour Reduce Transparency and Increase Contrast.
- **Motion:** no animation on large layers (whole panes, viewers, sheets). WebKitGTK on some Wayland compositors flickers, so these open and close instantly. Small, local motion is fine if it passes the frame-sampling suites in WebKit.
- **Glass on chrome only.** Sidebars, toolbars, menus and sheets may be translucent; your content never is.
- **Keyboard first.** Follow the keys of Apple's matching app, with Ctrl for ⌘.
- **No fruit imagery** in icons or artwork.
- Include **before and after screenshots, light and dark**, in the pull request.

## Commit style

Short, plain-English subjects that start with the area changed, followed by what's now true:

```text
Notes: tables keep their rows after an empty version
Calendar: an event's time zone, set as Calendar sets one
Windows build: the launch check runs for a quick build too
```

- Area first (`Notes`, `Reminders`, `Calendar`, `Contacts`, `Drive`, `Photos`, `Sync`, `Interface`, `Windows build`, `Packaging`, `Docs`, …), then a colon.
- Describe the result, not the activity ("tables keep their rows", not "fix table bug").
- Use the body for the *why*, and for what you verified and how.
- One logical change per commit.

## Pull requests

1. Fork, branch from `main`, and keep the change focused.
2. Run `./check.sh` until it's green.
3. Fill in the pull request template: what, why, how it was tested, and screenshots for UI.
4. Expect a review within a few days. Small, well-tested pull requests are merged fastest.

Thank you for making Skykeep better.
