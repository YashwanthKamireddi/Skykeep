<div align="center">

<img src="packaging/icons/skykeep-1024.png" alt="Skykeep" width="104">

# Skykeep

Your iPhone's Notes, Reminders, Calendar, Contacts, iCloud Drive and Photos, as desktop apps on Linux and Windows.

<p><a href="https://apps.microsoft.com/detail/9P6B042JNK54"><img src="https://get.microsoft.com/images/en-us%20dark.svg" alt="Get it from Microsoft" height="52"></a>&nbsp;&nbsp;<a href="https://snapcraft.io/skykeep"><img src="https://snapcraft.io/static/images/badges/en/snap-store-black.svg" alt="Get it from the Snap Store" height="52"></a></p>

<a href="#install">All downloads: Fedora, Arch, Debian, AppImage, Chocolatey, Scoop</a>

<br><br>

<img src="docs/media/hero.webp" alt="Skykeep ticking off a reminder, switching to the week in Calendar, opening a note, then a contact card, an iCloud Drive folder and the photo library" width="880">

<p><a href="https://github.com/YashwanthKamireddi/Skykeep/releases/latest"><img src="https://img.shields.io/github/v/release/YashwanthKamireddi/Skykeep?label=release&color=2f6fd6" alt="Latest release"></a> <a href="https://github.com/YashwanthKamireddi/Skykeep/releases"><img src="https://img.shields.io/github/downloads/YashwanthKamireddi/Skykeep/total?label=downloads&color=2f6fd6" alt="Downloads"></a></p>

</div>

Skykeep keeps a copy of your iCloud on your computer and keeps it in sync with your iPhone. Every app opens straight from that copy, so nothing waits on the network: it works on a train, and a reminder you tick off on your phone usually disappears on your desktop about a second later.

I'm [Yashwanth](https://github.com/YashwanthKamireddi). I built Skykeep because icloud.com kept signing me out on my Linux PC and stopped working whenever the Wi-Fi did, and I use it every day.

<table>
  <tr>
    <td width="50%" align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/media/notes-dark.webp"><img src="docs/media/notes-light.webp" alt="Notes with folders, pinned notes and a trip note with a checklist and a table"></picture><b>Notes</b></td>
    <td width="50%" align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/media/reminders-dark.webp"><img src="docs/media/reminders-light.webp" alt="Reminders with lists, sections, subtasks, flags and tags"></picture><b>Reminders</b></td>
  </tr>
  <tr>
    <td width="50%" align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/media/calendar-dark.webp"><img src="docs/media/calendar-light.webp" alt="Calendar in week view with events from four calendars"></picture><b>Calendar</b></td>
    <td width="50%" align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/media/contacts-dark.webp"><img src="docs/media/contacts-light.webp" alt="Contacts with photos and a full contact card"></picture><b>Contacts</b></td>
  </tr>
  <tr>
    <td width="50%" align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/media/drive-dark.webp"><img src="docs/media/drive-light.webp" alt="An iCloud Drive folder of pictures and documents in grid view"></picture><b>iCloud Drive</b></td>
    <td width="50%" align="center"><picture><source media="(prefers-color-scheme: dark)" srcset="docs/media/photos-dark.webp"><img src="docs/media/photos-light.webp" alt="The photo library by month, with albums"></picture><b>Photos</b></td>
  </tr>
</table>

Each one is its own app with its own launcher or Start-menu entry, and they share one sign-in. Reminders ring on your desktop with Complete and Snooze even when no window is open. On Linux, iCloud Drive and Photos also show up as folders in Files or Dolphin, so any app can open what's in them. Your family's Shared Library sits beside your own photos, as it does on iPhone.

## Your Apple password, and what leaves your computer

You sign in with your Apple Account, so here is what happens to it, with where to check.

- Skykeep signs in the way icloud.com does, with Apple's SRP protocol and two-factor: it proves to Apple that it knows your password without ever sending it ([`srp.rs`](crates/quince-auth/src/srp.rs)).
- The code that talks to iCloud refuses to even look up an address outside apple.com, icloud.com and icloud-content.com (plus Apple's own regional domains, for accounts Apple keeps there). A test in this repo holds it to that ([`apple_only.rs`](crates/quince-auth/tests/apple_only.rs)).
- The app window itself can't reach the internet at all. Its content policy allows only Skykeep's own engine ([`tauri.conf.json`](src-tauri/tauri.conf.json)).
- There are no Skykeep servers, accounts, analytics or crash reports. The only other address it contacts is GitHub, to check for updates, and you can turn that off. Updates install only after their signature checks out.
- Your password and session are kept in the system keyring, the saved connection is encrypted (AES-256-GCM, its key in the keyring), and your library sits in a folder only you can read.

One thing to know: like any app that stays signed in, Skykeep keeps your password in the keyring, and other programs running as you can read it. Signing out erases it. Your library isn't encrypted by Skykeep, so use your system's disk encryption (BitLocker, LUKS) if your computer could be lost. [SECURITY.md](SECURITY.md) is the line-by-line version.

## Your iCloud stays intact

Every change you make is shown at once and queued, then sent to Apple in the order you made it. With no connection it waits, and sending it twice is harmless. A note typed on your iPhone and in Skykeep at the same moment comes out with both edits, because Skykeep merges notes the way Apple's own apps do. Deleting a note, a file or a photo moves it to Recently Deleted, as it does on your iPhone. You can export everything to Markdown, iCalendar and vCard whenever you like.

## Before you install

Signing in asks you to approve on your iPhone, iPad or Mac, as icloud.com does. If you use Advanced Data Protection, turn on **Access iCloud Data on the Web** (iPhone: Settings, your name, iCloud): Calendar and Contacts then open right away, and Notes, Reminders, Photos and iCloud Drive sync once you approve Skykeep on your iPhone. Skykeep runs on Windows 10 and 11, and on Linux with glibc 2.39 or newer (Ubuntu 24.04, Mint 22, Debian 13, Fedora 41 and later), on x86 or ARM.

## Install

| | |
|---|---|
| **Windows** | [Microsoft Store](https://apps.microsoft.com/detail/9P6B042JNK54), or the [installer](https://github.com/YashwanthKamireddi/Skykeep/releases/latest/download/Skykeep-windows-x64-setup.exe). Also `choco install skykeep` and [Scoop](https://github.com/YashwanthKamireddi/scoop-bucket) |
| **Linux stores** | [Snap Store](https://snapcraft.io/skykeep)<br>Fedora and Fedora Asahi: `sudo dnf copr enable yashonfed/skykeep && sudo dnf install skykeep`<br>Arch: `yay -S skykeep` |
| **Linux files** | [`.deb`](https://github.com/YashwanthKamireddi/Skykeep/releases/latest/download/Skykeep-linux-amd64.deb) ([ARM](https://github.com/YashwanthKamireddi/Skykeep/releases/latest/download/Skykeep-linux-arm64.deb)), [`.rpm`](https://github.com/YashwanthKamireddi/Skykeep/releases/latest/download/Skykeep-linux-x86_64.rpm) ([ARM](https://github.com/YashwanthKamireddi/Skykeep/releases/latest/download/Skykeep-linux-aarch64.rpm)), or the [AppImage](https://github.com/YashwanthKamireddi/Skykeep/releases/latest/download/Skykeep-linux-x86_64.AppImage) for any other Linux |

Every file, with checksums, is on the [latest release](https://github.com/YashwanthKamireddi/Skykeep/releases/latest). The Windows installer, the AppImage and every store keep Skykeep up to date; for a `.deb` or `.rpm`, install the new one over the old.

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

<details>
<summary>"Windows protected your PC"</summary>

The Microsoft Store copy is checked by Microsoft and installs straight away. The installer on this page is new to SmartScreen, so Windows asks once: **More info**, then **Run anyway**.
</details>

## What works, and what doesn't yet

| | Linux | Windows |
|---|:-:|:-:|
| Notes, Reminders, Calendar, Contacts, iCloud Drive and Photos | ✅ | ✅ |
| Reminder alerts with Complete and Snooze | ✅ | ✅ |
| Shared Library, shared albums, a locked Hidden album | ✅ | ✅ |
| iCloud Drive and Photos as folders in your file manager | ✅ | In progress |
| Results in your desktop's search | ✅ GNOME, KDE | No |
| Apple Silicon Macs running Linux | ✅ | |
| Replying to calendar invitations | Not yet | Not yet |
| Shared notes and shared reminder lists | Not yet | Not yet |
| Mail and Find My | Not planned | Not planned |
| Passwords and Messages | Can't be built: Apple keeps them end-to-end encrypted, out of reach of anything but its own devices | |

<details>
<summary>Everything each app does</summary>

- **Notes:** styles, lists and checklists; pictures, files and tables; #tags, scans and drawings shown in place; folders, pinned notes, search, Recently Deleted, Markdown export.
- **Reminders:** lists with colours and sections; Today, Scheduled, Flagged and smart lists; subtasks, repeats, flags, priorities, tags, early alerts. "Call Alex tomorrow at 5" sets the date.
- **Calendar:** day, week, month and year; drag to create, move and resize; repeating events, time zones, birthdays and holidays; a Join button for video calls; `webcal://` subscriptions and `.ics` files.
- **Contacts:** nearly every field with Apple's labels, photos, lists, My Card, merging duplicates; vCard and Google or Outlook CSV import.
- **iCloud Drive:** list or grid with thumbnails; upload, rename, move, duplicate, Space to preview; whole folders download and pick up where they stopped.
- **Photos:** your library by month, albums, shared albums, Favourites, Videos; Live Photos and videos play; slideshow, copy, original-quality download.
</details>

## How it's built

A Rust engine under a Tauri 2 window. Every screen reads a local SQLite library and never the network, which is why it's instant and offline. Notes and Reminders are CloudKit records, merged through Apple's own CRDT; Calendar, Contacts and Drive go through Apple's web services; changes arrive over Apple's push channel. On Linux a FUSE mount puts Drive and Photos in your file manager. Over 1,300 Rust tests and interface checks in two browser engines run before every release. [ARCHITECTURE.md](ARCHITECTURE.md) has the full tour.

## Questions

<details>
<summary>Is Skykeep made by Apple?</summary>

No. It's an independent, open-source app that works with iCloud. Apple, iCloud and iPhone are trademarks of Apple Inc.
</details>

<details>
<summary>How is this different from iCloud for Windows?</summary>

iCloud for Windows brings Photos and iCloud Drive to File Explorer, and Calendar and Contacts through Outlook. It has no Notes or Reminders. Skykeep has all six as apps of their own, works offline, and runs on Linux too.
</details>

<details>
<summary>Something's wrong. How do I send a log?</summary>

Quit Skykeep and start it with `SKYKEEP_LOG=1 skykeep` (Windows PowerShell: `$env:SKYKEEP_LOG=1`, then `skykeep.exe`). It writes `diagnostics.log` in its data folder with the addresses it asked and Apple's answer codes, never passwords, tokens or anything from your library. Attach it to a [bug report](https://github.com/YashwanthKamireddi/Skykeep/issues/new?template=bug_report.yml).
</details>

## Contributing

Bug reports, packaging for your distro, design ideas and code are all welcome. [CONTRIBUTING.md](.github/CONTRIBUTING.md) has the setup and a map of the code, and [Discussions](https://github.com/YashwanthKamireddi/Skykeep/discussions) is the place for questions and ideas. Security problems go to a [private report](https://github.com/YashwanthKamireddi/Skykeep/security/advisories/new).

Copyright (C) 2026 Yashwanth Kamireddi. [GPL-3.0](LICENSE): if you share a copy or a changed version, keep this notice and the same licence. Not affiliated with or endorsed by Apple Inc.
