<p align="center">
  <img src="packaging/icons/skykeep.svg" alt="Skykeep logo" width="96" height="96">
</p>

<h1 align="center">Skykeep</h1>

<p align="center"><strong>iCloud, set free.</strong><br>Notes, Reminders, Calendar, Contacts, iCloud Drive and Photos as native apps on Windows, Linux and Android. Free and open source, in sync with your iPhone, iPad and Mac.</p>

<p align="center">
  <a href="https://github.com/YashwanthKamireddi/Skykeep/releases/latest"><strong>Download</strong></a> ·
  <a href="#install">Install</a> ·
  <a href="#engineering">How it works</a> ·
  <a href="#private-by-design">Privacy</a> ·
  <a href=".github/CONTRIBUTING.md">Contribute</a>
</p>

<p align="center">
  <a href="ARCHITECTURE.md"><img src="https://img.shields.io/badge/Built_with-Rust-2f6fd6" alt="Built with Rust"></a>
  <a href="#install"><img src="https://img.shields.io/badge/Linux_%C2%B7_Windows_%C2%B7_Android-native-2f6fd6" alt="Linux, Windows and Android, native"></a>
  <a href="https://github.com/YashwanthKamireddi/Skykeep/releases"><img src="https://img.shields.io/github/downloads/YashwanthKamireddi/Skykeep/total?label=GitHub%20Downloads&color=2f6fd6" alt="GitHub downloads"></a>
  <a href="https://github.com/YashwanthKamireddi/Skykeep/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/YashwanthKamireddi/Skykeep/ci.yml?branch=main&label=CI" alt="CI"></a>
  <a href="https://github.com/YashwanthKamireddi/Skykeep/releases/latest"><img src="https://img.shields.io/github/v/release/YashwanthKamireddi/Skykeep?label=release&color=2f6fd6" alt="Latest release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-GPL--3.0-2f6fd6" alt="GPL-3.0 license"></a>
</p>

<p align="center"><a href="https://apps.microsoft.com/detail/9P6B042JNK54"><img src="https://get.microsoft.com/images/en-us%20dark.svg" alt="Get it from Microsoft" height="48"></a>&nbsp;&nbsp;<a href="https://snapcraft.io/skykeep"><img src="https://snapcraft.io/static/images/badges/en/snap-store-black.svg" alt="Get it from the Snap Store" height="48"></a></p>

<p align="center"><b>Android:</b> <a href="https://github.com/YashwanthKamireddi/Skykeep/releases/latest/download/Skykeep-android.apk">download the APK</a> · <b>Linux:</b> <a href="#install">all packages</a></p>

Skykeep keeps your iCloud on your computer and phone, in sync with your iPhone, iPad and Mac. Every app opens instantly and works offline. Underneath is a small Rust engine that idles at under 0.1% of a CPU core.

<p align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/media/hero-dark.webp"><img src="docs/media/hero-light.webp" width="880" alt="Skykeep's window with its six app tiles; Notes, Calendar and Photos each open from their tile in a window of their own"></picture></p>

## Your iCloud, as apps

Six apps, each with its own icon in your launcher and Start menu and a window of its own. Click a picture to see it full size.

<table>
  <tr>
    <td width="50%" valign="top"><p align="center"><img src="packaging/icons/apps/notes-256.png" width="64" alt=""><br><b>Skykeep Notes</b><br><sub>Checklists, tables, drawings and #tags.</sub></p><a href="docs/media/notes-light.webp"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/media/notes-dark.webp"><img src="docs/media/notes-light.webp" alt="Notes with folders, pinned notes and a trip note"></picture></a></td>
    <td width="50%" valign="top"><p align="center"><img src="packaging/icons/apps/reminders-256.png" width="64" alt=""><br><b>Skykeep Reminders</b><br><sub>Smart lists, subtasks, tags and alerts.</sub></p><a href="docs/media/reminders-light.webp"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/media/reminders-dark.webp"><img src="docs/media/reminders-light.webp" alt="Reminders with lists, sections and subtasks"></picture></a></td>
  </tr>
  <tr>
    <td width="50%" valign="top"><p align="center"><img src="packaging/icons/apps/calendar-256.png" width="64" alt=""><br><b>Skykeep Calendar</b><br><sub>Day to year views, drag to edit, time zones. Nextcloud and CalDAV calendars beside iCloud’s.</sub></p><a href="docs/media/calendar-light.webp"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/media/calendar-dark.webp"><img src="docs/media/calendar-light.webp" alt="Calendar in week view"></picture></a></td>
    <td width="50%" valign="top"><p align="center"><img src="packaging/icons/apps/contacts-256.png" width="64" alt=""><br><b>Skykeep Contacts</b><br><sub>Every field, lists, merging, vCard and CSV.</sub></p><a href="docs/media/contacts-light.webp"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/media/contacts-dark.webp"><img src="docs/media/contacts-light.webp" alt="A full contact card"></picture></a></td>
  </tr>
  <tr>
    <td width="50%" valign="top"><p align="center"><img src="packaging/icons/apps/drive-256.png" width="64" alt=""><br><b>Skykeep Drive</b><br><sub>Previews, resumable downloads, a real folder.</sub></p><a href="docs/media/drive-light.webp"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/media/drive-dark.webp"><img src="docs/media/drive-light.webp" alt="An iCloud Drive folder in grid view"></picture></a></td>
    <td width="50%" valign="top"><p align="center"><img src="packaging/icons/apps/photos-256.png" width="64" alt=""><br><b>Skykeep Photos</b><br><sub>Albums, Shared Library, Live Photos, videos.</sub></p><a href="docs/media/photos-light.webp"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/media/photos-dark.webp"><img src="docs/media/photos-light.webp" alt="The photo library by month"></picture></a></td>
  </tr>
</table>

## Beyond the apps

**Send to iPhone.** Right-click any file and choose Send to iPhone: photos land in Photos, documents in Files, wherever the phone is. Nothing to install on the iPhone.

**Back up iCloud.** Every original photo and video, and all of iCloud Drive, copied to a folder or drive you choose and kept current. Changed files keep their earlier version, and a backup never deletes.

**iCloud in your file manager.** iCloud Drive and Photos are real folders in File Explorer and on Linux, with thumbnails before anything downloads.

## Engineering

A native Rust engine with a light Tauri window on top, one codebase for Linux and Windows on x86 and ARM, and the same engine inside the Android app.

- Every screen reads a local SQLite library, never the network, so apps open instantly and work offline.
- Changes from your iPhone arrive over Apple's own push channel as they happen.
- Edits show at once and go through an outbox: sent in order, held while offline, safe to send twice.
- Notes merge through Apple's own CRDT, so typing on two devices at once keeps both edits.
- Apple's SRP sign-in with two-factor is rebuilt in Rust. Your password is never sent.
- The engine's DNS resolver cannot look up a host that isn't Apple's.
- Each release passes over 1,300 Rust tests and is installed and launched on Ubuntu, Debian, Mint, Fedora, openSUSE, Arch and Windows before it ships.

[ARCHITECTURE.md](ARCHITECTURE.md) has the full tour.

## Private by design

- No Skykeep servers, accounts, analytics or crash reports. Your iCloud goes only between your device and Apple.
- Your session lives in the system keyring (the Android Keystore on a phone), and your library in a folder only you can read.
- Nextcloud and CalDAV calendars can sit beside iCloud's, on a connection locked to your own server.
- Open source, built in public, with checksums and signature-checked updates.

[SECURITY.md](SECURITY.md) shows the code behind each point.

## On Android

Reminders, Notes, Photos and iCloud Drive as apps, and your iCloud contacts and calendars inside Android's own Contacts and Calendar, synced both ways. iCloud Drive also appears in the Files app. Same engine as the desktop, offline-first, talking only to Apple.

<p align="center"><a href="docs/media/android/home-light.webp"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/media/android/home-dark.webp"><img src="docs/media/android/home-light.webp" width="280" alt="Home: the account and each service syncing"></picture></a></p>

[Download the APK](https://github.com/YashwanthKamireddi/Skykeep/releases/latest/download/Skykeep-android.apk) · Android 10 or newer, 64-bit ARM

## Install

| Platform | Installation |
|---|---|
| **Windows** | [Microsoft Store](https://apps.microsoft.com/detail/9P6B042JNK54), or the [installer](https://github.com/YashwanthKamireddi/Skykeep/releases/latest/download/Skykeep-windows-x64-setup.exe). Also `choco install skykeep` and [Scoop](https://github.com/YashwanthKamireddi/scoop-bucket) |
| **Linux stores** | [Snap Store](https://snapcraft.io/skykeep)<br>Fedora and Fedora Asahi: `sudo dnf copr enable yashonfed/skykeep && sudo dnf install skykeep`<br>Arch: `yay -S skykeep` |
| **Linux files** | [`.deb`](https://github.com/YashwanthKamireddi/Skykeep/releases/latest/download/Skykeep-linux-amd64.deb) ([ARM](https://github.com/YashwanthKamireddi/Skykeep/releases/latest/download/Skykeep-linux-arm64.deb)), [`.rpm`](https://github.com/YashwanthKamireddi/Skykeep/releases/latest/download/Skykeep-linux-x86_64.rpm) ([ARM](https://github.com/YashwanthKamireddi/Skykeep/releases/latest/download/Skykeep-linux-aarch64.rpm)), or the [AppImage](https://github.com/YashwanthKamireddi/Skykeep/releases/latest/download/Skykeep-linux-x86_64.AppImage) for any other Linux |
| **Android** | The [APK](https://github.com/YashwanthKamireddi/Skykeep/releases/latest/download/Skykeep-android.apk) from GitHub Releases |

Every file and its checksum is on the [latest release](https://github.com/YashwanthKamireddi/Skykeep/releases/latest). The stores, the Windows installer and the AppImage update themselves; for a `.deb` or `.rpm`, install the new one over the old.

<details>
<summary>Commands for each Linux file, and fixes for Fedora, openSUSE and the AppImage</summary>

```sh
sudo apt install ./Skykeep-linux-amd64.deb                          # Ubuntu, Debian, Mint
sudo dnf install ./Skykeep-linux-x86_64.rpm                         # Fedora
sudo zypper install --allow-unsigned-rpm ./Skykeep-linux-x86_64.rpm # openSUSE Tumbleweed
chmod +x Skykeep-linux-x86_64.AppImage && ./Skykeep-linux-x86_64.AppImage
sudo snap install skykeep && sudo snap connect skykeep:password-manager-service
```

The Snap's second command lets it keep you signed in through your keyring. On Ubuntu the AppImage needs `libfuse2t64` once (`libfuse2` on 22.04). Fedora and openSUSE leave out the decoder for iPhone photos (HEIC) for patent reasons: on Fedora, enable RPM Fusion and `sudo dnf install libheif-freeworld`; on openSUSE, add Packman and install `libheif1`. For Drive and Photos as folders on Arch, also `sudo pacman -S fuse3`.
</details>

## Official downloads

Skykeep comes only from the links above. The AUR packages are community-maintained builds of these releases. The Android APK is signed with this certificate (SHA-256):

```
61:0A:64:50:6D:95:DA:6C:CE:75:FF:0E:40:B8:59:02:FA:41:E4:0D:00:1B:BA:87:BF:40:70:FE:CB:F2:3A:FB
```

## Sign in

Sign in with your Apple Account and approve it on your iPhone, iPad or Mac, as for icloud.com. Each app fills in as your library arrives.

Using Advanced Data Protection? Turn on **Access iCloud Data on the Web** first (iPhone: Settings, your name, iCloud).

## Questions

<details>
<summary>How do I use iCloud and Apple Notes on Linux?</summary>

Install Skykeep (see [Install](#install)) and sign in with your Apple Account. Apple makes no iCloud app for Linux; Skykeep gives you Notes, Reminders, Calendar, Contacts, iCloud Drive and Photos as native apps there, kept on your computer and in sync with your iPhone. iCloud Drive and Photos also appear as folders in your file manager.
</details>

<details>
<summary>Can I use Apple Notes and Reminders on Windows?</summary>

Yes. Apple's own iCloud for Windows has no Notes or Reminders app; Skykeep has both, with checklists, tables, smart lists and alerts, alongside Calendar, Contacts, iCloud Drive and Photos. Get it from the Microsoft Store, Chocolatey, Scoop or the installer on the [releases page](https://github.com/YashwanthKamireddi/Skykeep/releases/latest).
</details>

<details>
<summary>How do I get iCloud Notes and Reminders on Android?</summary>

Install the [APK](https://github.com/YashwanthKamireddi/Skykeep/releases/latest/download/Skykeep-android.apk) and sign in with your Apple Account. Notes, Reminders, Photos and iCloud Drive open as apps; your iCloud contacts and calendars appear in Android's own Contacts and Calendar.
</details>

<details>
<summary>Is Skykeep made by Apple?</summary>

No. It's an independent, open-source app that works with iCloud. Apple, iCloud and iPhone are trademarks of Apple Inc.
</details>

<details>
<summary>How is this different from iCloud for Windows?</summary>

iCloud for Windows brings Photos and iCloud Drive to File Explorer, and Calendar and Contacts through Outlook. Skykeep has Notes and Reminders too, all six as apps of their own in your launcher, works offline, and runs on Linux as well.
</details>

<details>
<summary>Why does it need my Apple Account password?</summary>

Notes, Reminders, Drive and Photos open only to a full sign-in with two-factor, as on icloud.com. Skykeep proves your password with SRP instead of sending it, and keeps it in your system keyring; signing out erases it.
</details>

<details>
<summary>Is it safe to use with my Apple Account?</summary>

Skykeep signs in the way icloud.com does, with your password and two-factor code, and talks only to Apple (plus any calendars or servers you add yourself, and GitHub's release servers in builds that update themselves). Apple has no official API for Notes, Reminders, Drive or Photos, so Skykeep uses the same web services icloud.com uses, as icloudpd has for years. Apple can change them at any time and may occasionally ask you to sign in again.
</details>

<details>
<summary>How do I send a diagnostics log?</summary>

Quit Skykeep and start it with `SKYKEEP_LOG=1 skykeep` (Windows PowerShell: `$env:SKYKEEP_LOG=1`, then `skykeep.exe`). It writes `diagnostics.log` in its data folder with the addresses it asked and Apple's answer codes, never passwords, tokens or anything from your library. Attach it to a [bug report](https://github.com/YashwanthKamireddi/Skykeep/issues/new?template=bug_report.yml).
</details>

## Contributing

Bug reports, packaging for your distro, design ideas and code are all welcome. [CONTRIBUTING.md](.github/CONTRIBUTING.md) has the setup and a map of the code, and [Discussions](https://github.com/YashwanthKamireddi/Skykeep/discussions) is the place for questions and ideas. Security problems go to a [private report](https://github.com/YashwanthKamireddi/Skykeep/security/advisories/new).

## License

Skykeep is free and open source under the [GPL-3.0](LICENSE).

Copyright (C) 2026 Yashwanth Kamireddi. Not affiliated with Apple Inc. Apple, iCloud and iPhone are trademarks of Apple Inc.
