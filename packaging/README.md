# Packaging Skykeep

Everything that turns a build into something people install.

| Path | What it is |
|---|---|
| `arch/` | A PKGBUILD that builds the checkout it sits in. |
| `aur/` | The AUR packages, each in its own folder with its `.SRCINFO`: `skykeep` builds the release's source tarball, `skykeep-bin` installs the release's `.deb`, and `skykeep-git` builds the latest commit. |
| `flatpak/` | The Flatpak manifest for Flathub, with its own [README](flatpak/README.md). |
| `chocolatey/` | The Chocolatey package (the release's installer, checked and run silently), with its own [README](chocolatey/README.md). |
| `scoop/` | The Scoop manifest for a bucket of our own, with its own [README](scoop/README.md). |
| `copr/` | The Fedora COPR spec, built from source with `../.copr/Makefile`, with its own [README](copr/README.md). |
| `apps/` | The six per-app launcher entries (Notes, Reminders, Calendar, Contacts, Drive, Photos). |
| `*.desktop.in`, `*.metainfo.xml` | The main launcher entry and the AppStream data software centres show. |
| `*.search-provider.ini`, `*.SearchProvider.service`, `*.krunner.desktop` | Results in GNOME Shell and KRunner search. |
| `icons/` | The app, tray and notification icons. |
| `windows/msix/` | The Microsoft Store's MSIX package: its manifest and `pack.ps1`. |
| `linux-compat/` | `check.sh`: installs a package in a clean distribution container and checks it runs. |

## From a release to every store

`release.yml` builds a tag into a draft release. Publishing it (`gh release edit vX.Y.Z
--draft=false`) starts `.github/workflows/stores.yml`, which skips prereleases and sends the
release to:

- **Snap Store** (stable): `snap.yml`, built from the `.deb`, started, uploaded.
- **Chocolatey**: `packages.yml`, packed, installed, uninstalled, then pushed.
- **Scoop**: `bucket/skykeep.json` in `scoop-bucket` gets the new version, URL and hash,
  checked against the installer itself, and is pushed.
- **winget**: a pull request to `microsoft/winget-pkgs` from the owner's fork, made by Komac
  (pinned release, checksum checked) from the NSIS `-setup.exe` only.
- **Microsoft Store**: `msix.yml` builds the unsigned `.msix` as the `skykeep-msix` artifact of
  the run; upload it to Partner Center by hand. It fails unless the `MSIX_*` repository
  variables are set.
- **COPR** builds each new tag from its own webhook.

Re-run one release: Actions → Stores → Run workflow, with its tag. The secrets it reads:

| Secret | What it is |
|---|---|
| `SNAPCRAFT_STORE_CREDENTIALS` | `snapcraft export-login` output. |
| `CHOCOLATEY_API_KEY` | The community.chocolatey.org API key. |
| `SCOOP_BUCKET_TOKEN` | A fine-grained token: repository access to `scoop-bucket` only, Contents read and write. |
| `WINGET_TOKEN` | A classic token with the `public_repo` scope (Komac cannot open the pull request with a fine-grained one). |

winget only updates a package it already has, so the first version's pull request must be
merged before the winget job can succeed.

## Arch, CachyOS, EndeavourOS, Manjaro

```sh
cd packaging/arch
makepkg -si
```

This installs Skykeep like any other program: `skykeep` on the path, launcher entries for the
main app and each of the six apps, the icon, search providers and the licence.

### The AUR

`aur/skykeep` is the one to recommend: the tagged release, built with the same steps as
`arch/`. `aur/skykeep-bin` unpacks the release's `.deb` (the build CI installs and starts on
Ubuntu), so nothing is compiled and pacman owns every file. `aur/skykeep-git` follows `main`.
`skykeep-bin` and `skykeep-git` provide and conflict with `skykeep`.

At each release, in `aur/skykeep` and `aur/skykeep-bin`:

```sh
# set pkgver to the new tag (without the v) and pkgrel=1, then:
updpkgsums                          # fills sha256sums from the published files
makepkg --printsrcinfo > .SRCINFO
```

Until `updpkgsums` has run, their `sha256sums` hold a `TODO` that makepkg refuses, so nothing
is built from a file nobody checked. Each folder is pushed to its own AUR repository
(`ssh://aur@aur.archlinux.org/<name>.git`) with just its `PKGBUILD` and `.SRCINFO`.

## Debian, Ubuntu, Fedora, AppImage, Windows

```sh
npx tauri build
```

builds every bundle for the platform you're on (`.deb`, `.rpm` and `.AppImage` on Linux;
`.msi` and the `-setup.exe` on Windows) under `target/release/bundle/`; `--bundles deb` (or
`rpm`, `appimage`) builds one. Releases are built the same way by
`.github/workflows/release.yml`, which installs and starts each Linux package before keeping
it.

The build dependencies are the ones in [CONTRIBUTING](../.github/CONTRIBUTING.md#set-up),
**including the libayatana-appindicator development package**: tauri-cli adds up the `tauri`
features of every target, sees the Windows/macOS `tray-icon`, and stops with "Can't detect any
appindicator library" without it. For the same reason the `.deb` depends on
`libayatana-appindicator3-1`, the `.rpm` on `libayatana-appindicator3.so.1()(64bit)` and the
AppImage carries `libayatana-appindicator3.so.1`, although the Linux tray is ksni's and never
loads it (`ldd` on the binary doesn't list it).

### What the AppImage carries for photos and videos

The AppImage is built in an Ubuntu 22.04 container (`.github/workflows/appimage.yml`): an
AppImage uses the glibc and libstdc++ of the computer it runs on, so it runs on the system it
was built on and newer ones, and 22.04 is the oldest Ubuntu LTS. The build fails if any ELF
file in it needs more than glibc 2.35, GLIBCXX 3.4.30 or CXXABI 1.3.13. It carries that
system's media stack, so HEIC photos and H.264/HEVC videos work on a computer that has none
of it:

- **GStreamer 1.20** (`bundle.linux.appimage.bundleMediaFramework`): Tauri's linuxdeploy
  GStreamer plugin copies every plugin installed on the build machine, with the libraries they
  link — the release builds install `gstreamer1.0-plugins-base` (playbin3, decodebin,
  videoconvert, appsink), `gstreamer1.0-plugins-good` (qtdemux for `.mov`/`.mp4`, videoflip)
  and `gstreamer1.0-libav` (avdec_h264, avdec_h265 — FFmpeg's libavcodec 58 from Ubuntu's
  universe), and its AppRun hook points GStreamer at them. Not `-bad` or `-ugly`, and no
  openh264.
- **libheif 1.19 with libde265's HEVC decoder**, built from their release tarballs (22.04's
  libheif is 1.12, older than the 1.17 the app needs) into Ubuntu's multiarch paths, and
  listed in `bundle.linux.appimage.files` (the plugin and `libde265.so.0`): libheif looks for its decoder plugins
  in the directory it was built with, which on another system is that system's (empty on a
  clean Debian or Fedora). At start-up, before libheif is used, Skykeep sets
  `LIBHEIF_PLUGIN_PATH` to the AppImage's own `usr/lib/libheif/plugins` — only when `APPDIR`
  is set, is absolute, contains the running program, and the directory really is inside it
  (`crates/quince-media/src/appimage.rs`); a `LIBHEIF_PLUGIN_PATH` you set yourself wins.

Licences: GStreamer, gst-libav and libde265 are LGPL; Ubuntu's libavcodec is built with
`--enable-gpl` (GPL-2.0-or-later), which Skykeep's GPL-3.0 allows. H.264 and HEVC are
patent-encumbered formats in some countries; the AppImage carries exactly what Ubuntu's
`gstreamer1.0-libav` installs — libavcodec with the libraries it links, x264 and x265 among
them, although Skykeep only decodes. The `.deb`, `.rpm` and
Arch package carry none of this — they use the system's own packages (Fedora and openSUSE
users add RPM Fusion or Packman for HEVC, as the README says).

Source for what the AppImage bundles, as the LGPL and GPL ask: the GStreamer, gst-libav,
FFmpeg, WebKitGTK and other libraries it copies are Ubuntu 22.04's own packages, whose exact
sources are in Ubuntu's archive (`apt-get source <package>` on 22.04, or each package's page
on https://launchpad.net/ubuntu/jammy); libheif and libde265 are built from the upstream
release tarballs whose versions and SHA-256 sums are pinned in `.github/workflows/appimage.yml`
(https://github.com/strukturag/libheif/releases, https://github.com/strukturag/libde265/releases).
The Windows build's libheif, libde265 and x265, which vcpkg builds and links in statically,
are listed with their licences and sources in `packaging/licences/extra.json`, shown in
Settings ▸ Open-Source Licences.

The files map uses Ubuntu's multiarch paths, so `npx tauri build --bundles appimage` is meant
for an Ubuntu/Debian machine with `libheif-plugin-libde265` (or the one appimage.yml builds)
installed; elsewhere build the `.rpm` or the Arch package.

`linux-compat.yml` (Actions → Linux compatibility) builds the packages on Ubuntu 24.04 (the
AppImage on 22.04, and starts it there with no network), Debian 13, Fedora 42, openSUSE Tumbleweed and Arch from these instructions and installs each in a
clean container of its distribution; `linux-compat/check.sh` is what it runs there.

## Flatpak

See [`flatpak/README.md`](flatpak/README.md): how to build it, the permissions it asks for,
and what's left before Flathub.

## What Skykeep needs to run

For the `.deb`, `.rpm` and Arch package, the floor is glibc 2.39 (they are built on Ubuntu
24.04) and libheif 1.17
(`libheif-rs` with `v1_17`): Ubuntu 24.04, Mint 22, Debian 13, Fedora 41, Tumbleweed, or later.
The `.deb` and `.rpm` declare both as versioned dependencies in `src-tauri/tauri.conf.json`, so
an older system is refused by its package manager with a clear message instead of a loader
error at launch. The AppImage carries its own libheif and needs glibc 2.35: Ubuntu 22.04 or
later. It needs FUSE 2 to mount itself; without `libfuse2` it runs with
`--appimage-extract-and-run`.

- `webkit2gtk-4.1` for the window and `libsecret` for the keyring. Without a keyring the app
  starts but can't stay signed in.
- `libheif` for HEIC photos.
- GStreamer with the base, good and libav plugins for videos.
- Optional: `fuse3`, for iCloud Drive and Photos as folders in the file manager.

## Opening files and quick capture

Calendar (`.ics`) and contact (`.vcf`) files open in Skykeep's import sheet
(`src-tauri/src/intake.rs`): read on this computer, shown, and added only when confirmed.

- **Linux:** the main entry declares `MimeType=text/calendar;text/vcard;text/x-vcard;` and takes
  `%U`; the metainfo files list the same types under `<provides>`. The per-app entries do not
  claim them, so "Open With" offers one "Skykeep", not a second "Calendar" beside GNOME's.
- **Windows:** `bundle.fileAssociations` in `tauri.conf.json` registers `.ics`/`.ical` and
  `.vcf`/`.vcard` in both the NSIS and MSI installers; Explorer passes the path on the command
  line, and a second launch hands it to the running Skykeep (single instance).

`skykeep --new reminder` and `skykeep --new note` open the quick-capture window;
`skykeep --new event` opens Calendar with a new event started. The first two are `Actions=` on
the main entry and on the Reminders and Notes entries (right-click in a dock or launcher).

## The six apps on Windows

What the per-app `.desktop` files and `Actions=` are on Linux:

- **Start menu:** beside "Skykeep", "Skykeep Notes", "Skykeep Reminders" and so on, each running
  `skykeep.exe --app notes` (and so on). The `.exe` installer makes them in
  `windows/installer-hooks.nsh` (`bundle.windows.nsis.installerHooks`), and removes them, unpinned,
  on uninstall; the MSI makes the same six from `windows/app-shortcuts.wxs`
  (`bundle.windows.wix.fragmentPaths`), in one component that its uninstall removes.
- **Taskbar buttons:** each shortcut carries its app's AppUserModelID,
  `io.github.yashwanthkamireddi.Skykeep.Notes` and so on, and each app's window says the same ID
  at runtime (`src-tauri/src/taskbar.rs`), so Notes and Calendar get their own taskbar buttons
  and pin as their own shortcuts. The process says Skykeep's own ID, the one Tauri stamps on
  "Skykeep", so the main window, toasts and the jump list are Skykeep's.
- **Jump list:** Skykeep writes it as it starts (`ICustomDestinationList`; no installer can
  declare one): New Note, New Reminder, New Event, then the six apps. Notes', Reminders' and
  Calendar's own buttons offer their own new item. Each task is one of the command lines above,
  handed to the Skykeep already running.

`.github/workflows/windows.yml` checks all of this after a silent install: each entry's app ID
and command line, each app window's app ID, every jump-list task, and that uninstalling leaves no
entry behind.

## The Microsoft Store (MSIX)

`.github/workflows/msix.yml` (Actions → MSIX for the Microsoft Store → Run workflow) builds
Windows as a release does, takes the program out of the `.msi` and packs it with
`windows/msix/pack.ps1` into an unsigned `.msix` — the `skykeep-msix` artifact. The Store signs
it. Then a signed copy is installed on the runner, started from its Start-menu entry, seen still
running 15 s later, and removed.

The package's identity comes from Partner Center (the app → Product management → Product
identity): `Package/Identity/Name`, `Package/Identity/Publisher` and
`Package/Properties/PublisherDisplayName`, given as the run's inputs or kept as the repository
variables `MSIX_IDENTITY_NAME`, `MSIX_PUBLISHER` and `MSIX_PUBLISHER_DISPLAY_NAME`. The package's
display name is "Skykeep", the name reserved there. A tag input builds that release's code.

The package has the same seven Start-menu entries as the installers, each entry's ID the
installers' app ID, so Windows knows them as `<package family name>!io.github.yashwanthkamireddi.Skykeep`,
`…!io.github.yashwanthkamireddi.Skykeep.Notes` and so on, and the windows, toasts and jump lists
use those (`src-tauri/src/package.rs`, `taskbar.rs`). It opens `.ics` and `.vcf` files. Open at
Login is the package's startup task (`SkykeepStartup`, listed in Task Manager's Startup apps),
turned on and off through Windows' StartupTask API (`login.rs`), not the `Run` key. It never
updates itself: a copy running from a package is the Store's to update (`updates.rs`,
`Install::Store`), and the packed program's bundle mark says neither `.msi` nor `.exe`.
