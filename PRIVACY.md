# Privacy policy

Skykeep is a free, open-source app that shows your own iCloud data on Windows, Linux and
Android. It is made by Yashwanth Kamireddi, an independent developer, and is not affiliated
with Apple.

## In short

Skykeep has no servers and no Skykeep account. Your iCloud goes between your device and Apple
only. Its developer never receives your data, your password or anything about how you use the
app.

## What Skykeep handles, and where it goes

To show your iCloud, Skykeep signs in to your Apple Account and reads and changes your iCloud
Notes, Reminders, Calendar, Contacts, iCloud Drive and Photos as you use it.

- **Your Apple Account password** is never sent, not even to Apple: sign-in uses Apple's SRP
  sign-in and Apple's two-factor codes. So that Skykeep can sign back in by itself, it keeps
  your password and session in your system's secure store: Credential Manager on Windows,
  GNOME Keyring or KWallet on Linux, and on Android sealed with a key held in the Android
  Keystore. Signing out erases them.
- **Your library**, a copy of your iCloud so the apps open instantly and work offline, is kept
  on your device: in Skykeep's data folder on a computer (readable by you alone on Linux), and
  in the app's own private storage on Android. Signing out asks whether to keep it or delete
  it.
- **Back up iCloud** (desktop) copies your iCloud Photos and iCloud Drive, as you choose, into
  a folder or drive you pick on your computer. Nothing is sent anywhere by it.
- **Send to iPhone** uploads the files you choose to your own iCloud Photos or iCloud Drive,
  and nowhere else.
- **Photo backup** (Android) is off until you turn it on. Then the photos and videos you take
  on the phone from that moment go to your own iCloud Photos, with the place each was taken,
  over Wi-Fi only unless you allow mobile data.
- **iCloud Drive and Photos folders** (Windows and Linux) show your iCloud files in your file
  manager, for you alone. On Android, iCloud Drive appears in the Files app and in other apps'
  file pickers, and an app gets a file only when you pick it there.
- **Android's own Contacts and Calendar.** On Android, once you allow it, your iCloud contacts
  and calendars appear in the phone's Contacts and Calendar apps under a Skykeep account. Other
  apps you have allowed to read contacts or calendars can then see them, as they can for any
  account on the phone. When you sign out, you choose: Keep a Copy leaves them on the phone as
  its own, no longer synced; Remove from Phone removes them with the account.
- **Search on Linux.** GNOME's search and KDE's KRunner can find items from your library. That
  search runs on your computer.

## What Skykeep doesn't do

No analytics, no telemetry, no crash reports, no advertising, no tracking. Skykeep doesn't
sell or share your data, and sends it nowhere except your own iCloud and the calendar servers
you add yourself. The Android app contains no Google Play Services or other third-party
analytics or advertising code.

Nothing of Skykeep's is copied into Android's cloud backup or a phone-to-phone transfer: the
app's data, its sealed password included, is left out of both, and is read again from iCloud
when you sign in on a new phone.

## Other connections (desktop only)

The Android app connects to Apple only. Links you tap in it, such as the source code or
Apple's password reset page, open in your browser. On Windows and Linux:

- **Calendar subscriptions:** only the calendars you subscribe to, to fetch them.
- **Calendar accounts:** only the Nextcloud or CalDAV servers you add, to keep those calendars
  in step. Their passwords stay in your system's secure store. Nothing of your iCloud is sent
  to them, except an event you put in one of their calendars yourself.
- **Updates:** a copy installed with the Windows installer (Chocolatey installs that one too),
  the `.msi` or the AppImage asks GitHub for the latest version about once a day, sending
  nothing about you. You can turn this off in Settings ▸ Software Update. Copies from the
  Microsoft Store, the Snap Store, Scoop, COPR, the AUR, Flatpak, a `.deb` or an `.rpm` are
  updated by those and never contact GitHub themselves. GitHub, like any website, sees the
  request come from your network address.

Settings ▸ What Skykeep Talks To lists every address your copy can reach, and
[SECURITY.md](SECURITY.md) shows the code that holds it to that.

## Logs and diagnostics

Skykeep sends no logs anywhere. On a computer, a diagnostics log is written only when you
start Skykeep with `SKYKEEP_LOG=1`, and Settings ▸ Save Diagnostic Report writes a short
report to your Downloads folder only when you press it. On Android, errors go to the phone's
own system log. They hold technical details, such as the addresses Skykeep asked, Apple's
answer codes and the app's errors, with account numbers and session identifiers taken out,
and never passwords, tokens or anything from your library. They leave your device only if you attach them to a bug report yourself; read them
before you do.

## Your controls

- Sign out at any time: it erases your password and session from the device and asks whether
  to keep or delete the library (on Android, also your contacts and calendars there).
- Turn off update checks in Settings ▸ Software Update.
- Remove a calendar account or unsubscribe from a calendar, and Skykeep stops contacting that
  server. Removing a Nextcloud account also revokes the app password Skykeep was given.
- On Android, turn Photo backup or its use of mobile data off in Skykeep's Settings, and take
  back access to contacts, calendars, photos or notifications in Android's settings at any
  time.
- Your data in iCloud is managed by Apple: see Apple's privacy policy and privacy.apple.com,
  where you can also see and manage the devices and apps signed in to your Apple Account.

## If you contact the developer

A bug report, or a diagnostics log you choose to attach to one, is published on GitHub and
handled under GitHub's privacy statement.

## Children

Skykeep collects nothing from anyone, children included. Whether a child may use an Apple
Account is set by Apple and the child's family.

## Changes

This page changes when Skykeep's handling of data changes, and the date below changes with it.
Every change is visible in the repository's history.

## Contact

Privacy questions: open an issue at https://github.com/YashwanthKamireddi/Skykeep/issues. For
anything you would rather not post in public, use a private report:
https://github.com/YashwanthKamireddi/Skykeep/security/advisories/new

Last updated: 11 November 2026
