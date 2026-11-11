# Security

Skykeep signs in to iCloud with your Apple Account. This page says exactly what it does with
your password and your data: what it talks to, what it keeps, and where. Every line here is
true of the code in this repository; if one ever stops being true, that is a bug, and a
serious one.

## What Skykeep talks to

Apple for your iCloud, the calendars you subscribe to, the calendar accounts you add yourself,
and GitHub's release servers for Skykeep's own updates:

- `idmsa.apple.com` — signing in (Apple's own SRP sign-in and two-factor codes).
- `setup.icloud.com`, `www.icloud.com` — opening the session and finding your services.
- `*.icloud.com` service addresses Apple gives the session (CloudKit, Contacts, Calendar,
  iCloud Drive, Photos), `gateway.icloud.com` and `cvws.icloud-content.com` (downloads).
- `calendars.icloud.com` — Apple's public holiday calendars.
- The host of each calendar you subscribe to by address (Calendar ▸ + ▸ New Subscription),
  and only once you have: a `webcal://` or `https://` address of a public calendar file.
- The server of each calendar account you add (Calendar ▸ + ▸ Add CalDAV Account, with
  Nextcloud Server under it for a Nextcloud), and only once you have.
- `github.com`, then `release-assets.githubusercontent.com` (or
  `objects.githubusercontent.com`), where GitHub serves release files — only in a copy
  installed from the AppImage, the Windows installer or the `.msi`, to check for a new version
  of Skykeep and, when you choose **Update and Restart**, to download it. A `.deb` or `.rpm`
  copy never contacts GitHub: you install new versions from the Releases page yourself.

Each of these is reached by a connection that can reach its own hosts and nothing else,
enforced in code rather than promised:

- **Apple.** Every iCloud client is built by `quince_auth::apple_only()`, whose resolver
  refuses to look up any host outside `apple.com`, `icloud.com` and `icloud-content.com` — a
  request anywhere else fails before a connection exists. A URL that names an address instead
  of a host (`https://1.2.3.4/`) is never looked up, so each request's URL is checked too, a
  redirect's included: anything that is not Apple's by name (or this computer, `127.0.0.1`)
  is refused the same way. It cannot reach GitHub. `crates/quince-auth/tests/apple_only.rs`
  holds it to that.
- **Calendar subscriptions.** A subscribed calendar is fetched by `quince_auth::listed_only()`,
  built for each fetch with exactly the hosts of the addresses you subscribed to — the same two
  locks — so it can reach those hosts and nothing else (`listed_tests` in
  `crates/quince-auth/src/lib.rs`). A subscription is read-only: nothing is sent to it but the
  request for its file, and no Apple data goes with that request.
- **Calendar accounts.** A Nextcloud or CalDAV account is reached by
  `quince_auth::account_only()`, built with exactly that account's hosts and the same two locks,
  so it can reach that server and nothing else (`crates/quince-auth/src/lib.rs`). Unlike a
  subscription, whose address anyone can hand you, it is your own server, so one at home or on
  a tailnet is reached. The password travels only over HTTPS, or in the clear to an address
  named as a private one on your own network (`192.168.…`, `10.…`). It is kept in the system keyring, never in
  a file. A Nextcloud is signed in to on its own page in your browser (Login Flow v2): Skykeep
  never sees your password, only an app password made for it, which you can revoke in
  Nextcloud's settings and which **Remove Account** revokes itself. A password the server
  refuses is not tried again until you sign in again, so Skykeep cannot trip a server's
  protection against guessing. Nothing of your iCloud is sent to these servers, except an event
  you put in one of their calendars yourself.
- **Updates.** The updater's client is locked by `quince_auth::releases::releases_only()` to
  the three GitHub hosts above, exactly (not `api.github.com`, not any other host under
  them), over HTTPS only, by the same two locks (`crates/quince-auth/src/releases.rs`). It
  cannot reach Apple. It asks for one file, `latest.json` from the latest release, and sends
  nothing about you: no account, no identifier, nothing from your library. GitHub, like any
  website, sees the request come from your network address, and counts downloads of release
  files.

An update is installed only if it passes its signature check: the updater verifies the
download against Skykeep's public key (in `src-tauri/tauri.conf.json`) with minisign, and the
version is signed into the signature, so an older release cannot be passed off as a newer
one. Skykeep never installs an older version or a prerelease, and never restarts to install
while a change is still on its way to iCloud or while you are typing. Checking automatically
can be turned off in **Settings ▸ Software Update**.

There are no Skykeep servers and no Skykeep account. There is no telemetry, analytics or crash
reporting. The window itself cannot reach the network at all: its content security policy
(`src-tauri/tauri.conf.json`) lets it talk only to the app's own Rust side, which does all the
talking, and it is granted none of the updater plugin's commands.

Links you click in a note or an event open in your own browser, mail or map app; a photo's
place opens in this computer's own map app (a `geo:` link), never a website.

## What Skykeep keeps, and where

Everything is in your user's data folder: `~/.local/share/io.github.yashwanthkamireddi.Skykeep`
on Linux, readable by you alone (mode 700), and `%APPDATA%\io.github.yashwanthkamireddi.Skykeep`
on Windows.

- **Your password, Apple Account and session:** to keep you signed in, Skykeep keeps your
  Apple Account password together with the session in the system keyring: the Secret Service
  (GNOME Keyring or KWallet) on Linux, Credential Manager on Windows. Never in a file. With it
  Skykeep signs back in by itself when a session lapses, so you sign in once; Apple asks for a
  code only when it stops trusting this computer. Signing out erases the password, the session
  and the cookies' key from the keyring. If the keyring is locked or not running when you sign
  out, the cookies (and the library, if you chose not to keep it) are deleted all the same,
  Skykeep writes down in its data folder that the sign-out is still owed, reads nothing from
  the keyring until it is done, and erases those entries as soon as the keyring answers.
- **Session cookies:** `cookies.json`, encrypted (AES-256-GCM) with a random 256-bit key
  kept in the same keyring, one key per data folder, and readable by you alone (mode 600) as
  well. The file on its own opens nothing: copied into a backup or another folder, it is of no
  use without the keyring entry. With no keyring running, the cookies are kept in memory only
  and never written. If the key is gone (the keyring was reset), the file is deleted and
  Skykeep signs in afresh from what the keyring still holds, or asks you to.
- **What that protects, and what it does not.** The keyring is locked when you are logged out
  and encrypted on disk, but while you are logged in **any program running as you can read
  Skykeep's keyring entries**, the password included. The keyring does not tell one of your
  programs from another. A program that runs as you can already read your files too; the
  keyring is the system's own place for a secret like this, which is why it is kept there
  rather than in a file of Skykeep's own.
- **Your library:** `library.db` (SQLite) — notes, reminders, events, contacts, the Drive
  and Photos listings — and caches of pictures and files you opened. These are not encrypted
  by Skykeep; use full-disk encryption, as you would for the same data on a Mac.

Signing out asks whether to keep a copy on this computer. Removing it deletes the library's
files themselves — `library.db`, its `-wal` and `-shm`, and any damaged copy Skykeep set aside
beside them (`library.corrupt-*`) — not only the rows in them, so nothing of the account is
left in the file's free space; then its caches and Skykeep's file-manager thumbnails. Another
Apple Account signing in never sees the last one's data: each account's library is set aside
whole and brought back only for that account, the iCloud Drive and Photos folders are let go
of before it is, and a library that holds records but cannot say whose is set aside too,
never handed to the account signing in.

## What runs on this computer

- **Videos** are played from a server on `127.0.0.1` only (WebKitGTK's player reads only
  http). It answers only a request that carries a secret made fresh at every start and given
  to Skykeep's window alone, is addressed to `127.0.0.1:<its port>` (so a web page that
  rebinds its own name to this computer is refused), and carries no web page's `Origin`. It
  sends no CORS headers. Another program or user on this computer, or a page in your browser,
  cannot read your videos from it.
- **Pictures** reach the window through Skykeep's own `quince://` scheme, which only
  Skykeep's windows can load. Every id in it is checked to be an id — letters, digits,
  dashes — before it names a file, so no request reads anything outside Skykeep's caches.
- **The window** loads only Skykeep's own bundled page, never a website, and may ask
  Tauri for no more than it uses: events, its version and a window title.
- **Files on this computer** are read or written only when you chose them: in an open or save
  dialog Skykeep shows from its Rust side, by dropping them on the window, by pasting them into
  Drive after copying them in your file manager (only Paste itself grants them; the window
  looking at what is on the clipboard grants nothing), or by opening them with Skykeep. Each choice is recorded there (`src-tauri/src/grants.rs`) for ten minutes, a
  save for one write only, and every command that reads or writes a file (uploads, imports,
  attachments, exports) refuses a path that is not one of them, compared with `..` and links
  resolved. So even script running in the window could not name a file of yours on its own.
- **Download a Copy** in iCloud Drive — a file, a folder, or several at once — saves into your
  Downloads folder only: the window says what to download, never where. Every name in it, a
  shared folder's included, is made one plain name first (no `..`, folder separator, drive,
  stream or Windows device name), a folder arrives as a new folder there ("Photos 2" beside a
  "Photos"), nothing that was in Downloads before is written over or into, and no link found
  on the way is followed. Each file is written under a name of its own and moved into place
  only once it is whole; on Windows it is marked as come from the internet, so SmartScreen and
  Office's Protected View still look at it. A download that does not finish is remembered in
  Skykeep's own data folder (what was chosen, and where it went) until it does; downloading
  the same again goes on inside the folder it made — only if that is still a folder in
  Downloads — keeping each file already there at iCloud's size and replacing any other copy of
  its own.
- **A note's attachment** you open is saved under a cleaned-up name (never a path, a hidden
  file, or a name that reverses itself to hide its type) in your private data folder, not
  executable, and handed to the app this computer uses for it. A program, script, launcher or
  installer — by its name or its first bytes — is shown in its folder instead of opened, since
  opening it would run it, and a shared note can carry one from anyone.
- **The iCloud Drive and Photos folders** (FUSE) are mounted for you alone (no
  `allow_other`), with no set-user-id or device files, and nothing in them is executable.

## Advanced Data Protection

With Advanced Data Protection on and "Access iCloud Data on the Web" off, Apple does not
let any app outside your devices read your iCloud data, Skykeep included. Skykeep says so
plainly and shows the switch; it does not work around it.

With "Access iCloud Data on the Web" on, Skykeep uses Apple's own approval, the one
icloud.com uses (`crates/quince-auth/src/pcs.rs`). Calendar and Contacts work as usual. For
Notes, Reminders, Photos and iCloud Drive, Skykeep asks Apple to put an approval request on
your trusted iPhone, iPad or Mac. It asks only when you act: when you sign in, or when you
press Sync Now. It never asks from a background sync, a window opening or the timer. One
approval covers all four services, which Skykeep asks for one after another.
Once you approve, Apple's servers have that device send each service's key to this session,
and your devices show that they did. The keys arrive as `X-APPLE-WEBAUTH-PCS-*` cookies and
are kept in Skykeep's sealed cookie jar with the rest of the session. Apple limits an
approval to about an hour. Skykeep stops syncing those four services when the approval or
its keys run out, whichever comes first. Your copy on this computer stays readable, and
changes you make wait in the queue until you approve again.

Skykeep never sees your device keys. It never approves anything for you, and it never turns
Advanced Data Protection or any other Apple setting on or off. Health data and passwords in
iCloud Keychain are outside what Apple lets the web reach at all.

## Reporting a vulnerability

Please use GitHub's private vulnerability reporting on this repository (Security →
Report a vulnerability). Do not open a public issue for a security problem. You will get an
answer, and credit if you want it.
