# Architecture

Skykeep is one Rust engine with two shells: a Tauri app for Linux and Windows, and an Android
app in Kotlin that calls the same crates through UniFFI. This page is the map: what each layer
owns, how a read and a write move through it, and the rules the code depends on. Setup,
commands and CI are in [CONTRIBUTING](.github/CONTRIBUTING.md).

The crates still carry the project's working name, `quince-*`. That name also turns up in
event names (`quince://synced`) and some comments. Don't rename them.

## Layers

```text
 Desktop (Linux, Windows)                         Android
 src/         React window                        android/app   Compose screens, sync adapters,
   │ invoke, events                                            Files provider
 src-tauri/   commands, outbox worker,               │ UniFFI (JNA)
              refresh, push, folders, OS glue     crates/quince-mobile
   │                                                 │
   └──────────────────────┬──────────────────────────┘
                          ▼
 quince-sync    library lock, change queue; Notes, Reminders, Contacts, Calendar, Photos:
                record, lay, send; the sync pass
 quince-vfs     Drive and Photos as folders (FUSE, Cloud Files); Drive write-back
 quince-icloud  each Apple service: request, parse, encode, apply into the library
 quince-core    the library: SQLite schema and migrations, search, outbox table, export
 quince-auth    sign-in (SRP, two-factor), session and keyring, Advanced Data Protection,
                the only builders of HTTP clients
 quince-media   picture decoding (HEIC), crops, thumbnails
```

| Crate | Depends on (workspace) | Used by |
|---|---|---|
| `quince-auth` | none | app, mobile, probe |
| `quince-core` | none | everything above it |
| `quince-icloud` | core | sync, app, mobile, probe |
| `quince-sync` | core, icloud | app, mobile |
| `quince-media` | none | vfs, app, mobile (without HEIC), probe |
| `quince-vfs` | core, media, thumbs | app, mobile (write-back only), probe |
| `quince-thumbs` | none | vfs, app; also a DLL File Explorer loads |
| `quince-mobile` | auth, core, icloud, media, sync, vfs | the Android app |
| `uniffi-bindgen` | none | the Android build, to write the Kotlin bindings |
| `quince-app` (`src-tauri/`) | all but mobile | the desktop app |
| `quince-probe` (`probe/`) | auth, core, icloud, media, vfs | people, by hand |

`quince-icloud` does not depend on `quince-auth`. It takes a `reqwest::Client` (built by
`quince-auth`), a service root URL and the session's query parameters, which is what lets its
tests run against `wiremock`.

Each engine crate under `crates/` has a README with its main types and how to test it.

## Reading

**Screens read the library, never Apple.** The library is one SQLite file per account
(`library.db` in the data folder), owned by `quince_core::Library`. Every command that answers
a screen reads it.

**`quince_sync::library_lock::LibraryLock`** is how every caller shares it: one writing
connection behind a `tokio::sync::Mutex`, and read-only WAL connections beside it (up to four
kept open). A read goes to a reader only when nothing but a sync pass holds or waits for the
lock. While a change from the window holds or waits for it, reads queue behind it in turn, so a
read made after a save never shows the library from before it. A pass stores everything it
fetched in one `Library::atomically` transaction, so a reader sees all of it or none.

**A sync pass** fetches first and stores second: the network half never holds the lock. The
store takes `lock_for_pass`, writes Apple's records, and then replays every change still in
the outbox over them (`quince_sync::reminders::replay` and its kin), in the same transaction.
Without the replay, a pass landing while an edit waits would show Apple's older version. Each
service keeps its own sync token, and a full walk is forced once a day (`FULL_SYNC_EVERY`). One
service failing leaves the others alone: the failure is recorded in `quince_core::health` and
the last good data stays on screen.

**`quince_sync::pass`** is that pass, the same on both platforms: the desktop's `refresh.rs`
and the phone's `quince-mobile` both run Notes, Reminders and Photos through it (Calendar,
Contacts and Drive use its `store` and `token`). It checks the owner under the lock it stores
with, against the account the pass's own session parameters ask as; picks a full walk or a
delta; leaves a service Advanced Data Protection holds alone (`Paused`, Apple not asked); turns
a panic into that service's failure; records failures; and reads Photos' readiness, Shared
Albums and the Shared Library after the library. Before every pass both platforms call
`quince_auth::Client::ensure_session` (`keep.rs`): the heartbeat, then a resume when Apple says
the session lapsed, asking Apple at most once a minute and not at all once Apple has ended it.

**When a pass runs** is `src-tauri/src/push.rs`:

- Notes, Reminders, Photos: Apple's web push socket (`quince_icloud::courier`), the one
  icloud.com uses. Only the service that changed is refreshed. While the socket is down, each
  container's `changes/database` token is polled instead.
- Calendar and Contacts have no web push. They are polled with their own cheap change calls,
  less often when no window is focused or the machine is on battery (`power.rs`).
- Drive has no push either. `drive_watch.rs` follows its change feed and re-reads only the
  folders that changed.

After a pass the window hears `quince://synced` and the open screens read again.
`quince_core::changes` tracks which rows a write touched, so a screen can redraw those rows
instead of its whole list.

## Writing

Writes to Notes, Reminders, Contacts, Calendar and Photos' flags go through the **outbox**, a
queue in the same library as the records it changes.

1. **Record.** A command calls a `record_*` function (`quince_sync::notes::record_note_save`,
   `quince_sync::reminders::record_reminder_tick`, `quince_sync::contacts::record_card_save`,
   `quince_sync::calendar::record_event_save`). In one `atomically` transaction it changes the local rows and
   calls `Library::outbox_push`. The command returns and every view shows the change, online
   or not. `outbox_push` replaces an unsent change of the same kind on the same record, so only
   the last word on a field is sent.
2. **Drain.** The worker in `src-tauri/src/outbox.rs` wakes on `kick()`, on a backoff timer
   and when the network returns. For each service it calls `quince_sync::queue::drain_as`,
   which takes changes oldest first. Before each take it checks, under the library lock, that
   the library still belongs to the account it is sending as (see Accounts below). It also
   holds that service's pass mutex, so no sync pass can run between a send and its removal
   from the queue.
3. **Outcome.** `outbox_take` marks a change as tried before it is sent, so a change that may
   already be at Apple is never rewritten in place; later edits queue behind it instead.
   - Sent: `outbox_done`, and the next pass brings Apple's version.
   - Transient (`queue::transient`: network, reauth, busy, device approval, I/O; a protocol
     error or conflict for its first `PROTOCOL_TRIES`): `outbox_retry`, and the drain stops so
     order is kept.
   - Refused: `outbox_refuse` keeps the change, what was typed and Apple's reason. The window
     gets `quince://outbox-refused` and offers Try Again and Discard. A panic while sending is
     caught and treated as a refusal, so one bad change never blocks the queue.
4. **Idempotence.** Every change is safe to send twice. A create carries the record name it was
   given before it was queued, and a create that finds that name at Apple does nothing. Edits
   are merges against the version they were made from (a note's text against its base, a card's
   fields against the form's base).

Each kind of change has its own payload type, its own `lay` (how it shows locally, used by
both record and replay) and its own `send`. A Nextcloud or CalDAV account gets an outbox queue
of its own (`src-tauri/src/dav.rs`), so a server that is down holds up only its own changes.

**Photos** favourites, hiding, deletes, recovers and deletes for good go through the outbox
too (`quince_sync::photos::record_flags`, `send`), on the desktop's worker and at the start of
each Android Photos sync. One change carries up to `AT_ONCE` (200) photos, one CloudKit
request, so a large selection is a few changes, and a send cut off part way is narrowed
(`outbox_narrow`) to what Apple has not yet taken. A change undone before it is sent is taken
back off the queue. A photo deleted for good leaves every view at once but is kept whole in
`photos_aside` until Apple confirms (then forgotten) or refuses (then put back).

Other writes are shown at once but are not queued:

- **Photos** captions, ratings, keywords and dates (`src-tauri/src/photos.rs`) are sent
  directly and stored from Apple's answer.
- **Drive** operations from the app (rename, move, new folder, delete) are sent directly, then
  the folder is listed again. `DriveView.tsx` shows the change at once and puts the row back
  if the command fails.
- **Files saved in the mounted folders** are kept on disk first and sent once the writing app
  closes them (`quince-vfs` `writes.rs`). A file changed on another device meanwhile is saved
  beside it as a conflicted copy, never over it.

On the window side, `src/lib/outbox.ts` follows what is waiting and what was refused.
`src/lib/pending.ts` (used by Notes, Reminders and Calendar) lays changes still in flight over
each read, so a refresh that lands mid-write can't show the old state for a moment.

## Accounts

A library names its owner in its state table (`queue::ACCOUNT_KEY`) as `account_tag(dsid)`, a
SHA-256 of the account's Apple id. The id itself is never stored as the owner.

- **Desktop.** One library lives in the data folder. On sign-in, `claim_library` (in
  `main.rs`) runs while the auth lock is still held. Another account's library is moved whole
  to `accounts/<tag>/`, and this account's, if it was set aside before, is brought back with
  its caches and unsent changes. A library with records but no owner is set aside as
  unclaimed. Nothing is merged or deleted. The folders are released and the readers closed
  before the files move.
- **Every pass** checks `owned_by(library, asked_as(params))` before it stores, so a pass started
  under one account's session never writes into the next account's library. `drain_as` stops
  the same way.
- **Android** keeps one library per account under `libraries/<tag>/` in the app's files. Each
  Android account records the tag it syncs for, and the sync adapters refuse an account that
  isn't the signed-in one.
- **One Skykeep per library.** `library_lock::hold_or_mark` takes an exclusive lock on
  `skykeep.lock` beside the library (or `skykeep.pid` where the disk refuses locks), so two
  copies can never send the same queue.

## Desktop shell

One process, one library and one sync behind every window. The main window and each app's own
window (Notes, Reminders, Calendar, Contacts, Drive, Photos; `apps.rs`) are webviews over the
same commands. A screen once opened stays mounted while another is in front.

- **Commands** are in `main.rs` and the `*_cmds.rs` files. Their TypeScript types and wrappers
  are in `src/lib/ipc.ts`.
- **The window is untrusted.** It serves its own built bundle (no dev server; `check.sh`
  refuses a `devUrl`), its content policy lets it reach only the engine, a file path it names
  must have been handed over by the person (`grants.rs`), and links are filtered before the
  system opener sees them (`links.rs`).
- **Pictures** come through the `quince://` scheme with a disk cache (`photos.rs`,
  `photo_cache.rs`). Videos are served over loopback HTTP with a fresh secret per path
  (`video.rs`), because WebKitGTK's player can't read a custom scheme.
- **Folders.** `mount.rs` (Drive) and `mount_photos.rs` (Photos) serve `quince-vfs`: FUSE on
  Linux, the Cloud Files API on Windows. Explorer thumbnails come from the `quince-thumbs` DLL,
  which asks the app over a local pipe (`thumbs.rs`).
- **Calendar accounts other than iCloud** (`dav.rs`) are stored in the same `calendars` and
  `events` tables, with guids starting `quince:dav:`. `dav_resources` keeps each event's
  iCalendar text as the server has it, and edits are applied to that text line by line
  (`quince_icloud::ical_edit`) and sent with `If-Match`, so nothing Skykeep has no field for
  is lost.
- **System integration:** reminder alerts (`alerts.rs`, `notices.rs`), GNOME and KRunner search
  (`search_provider.rs`), open at login (`login.rs`), Windows taskbar and jump list
  (`taskbar.rs`), Send to iPhone (`send.rs`), backup (`backup.rs`), export (`export.rs`) and
  self-update for the AppImage, .exe and .msi (`updates.rs`).

## Android shell

`android/app` is a Compose app (package `io.github.yashwanthkamireddi.skykeep`, min SDK 29).
Gradle builds the engine before every build (`preBuild` in `android/app/build.gradle.kts`):

- `buildEngine`: `cargo ndk` builds `quince-mobile` for arm64-v8a and x86_64 into
  `src/main/jniLibs/`;
- `engineBindings`: `uniffi-bindgen` reads that library and writes the Kotlin bindings into
  `src/main/kotlin/uniffi/`;
- `engineLicences`: `cargo about` writes the engine's licences into the app.

None of those outputs is committed, so the library and its bindings always come from the same
code.

`quince-mobile` exposes coarse calls: each crosses the bridge once and does its work in Rust.
`Account` (a `uniffi::Object`) wraps the same `quince_auth::Client` the desktop uses.
`SecretStore` is a callback trait the Kotlin side implements over the Android Keystore
(`KeystoreSecrets.kt`), and it stands in for the desktop keyring through
`quince_auth::PlatformSecrets`.

How each service lands on the phone:

- **Contacts and Calendar** live in Android's own providers, written by sync adapters
  (`sync/ContactsSync.kt`, `GroupsSync.kt`, `CalendarSync.kt`), and are sent by the desktop's
  own code. Each sync hands every row Android marks changed to the engine as it is now, against
  its base (the record as last read from Apple, kept in the provider's sync columns);
  `quince-mobile` queues it in the engine's outbox (`quince_sync::contacts::queue_phone_card`
  and its kin, which queue nothing new for a row asked again unchanged) and drains it with
  `drain_as` and the shared `send`. The row is let go of only once its changes went or were
  refused; refusals are kept, as on the desktop, in Settings' Not Sent with Try Again and
  Discard (`Account::outbox_refused`). Contacts is then read only when Apple's tokens say the
  book changed; Calendar's span is read, and Apple's answer for each sent event is written
  into its row wherever it falls.
- **Notes, Reminders and Photos** live in the engine's library and use the desktop's own
  `quince-sync` code. Their adapters (`NotesSync.kt`, `RemindersSync.kt`, `PhotosSync.kt`)
  store nothing; they give Android a schedule and run an engine pass (`quince_sync::pass`, as
  the desktop runs it). Photos' collections list Shared Albums (looked at and shared, never
  changed) and, with a Shared Library, the desktop's library switcher.
- **Drive** appears in the Files app through a `DocumentsProvider` (`DriveProvider.kt`). A file
  is read in parts as the app reads it (a proxy file descriptor over `quince_vfs::Cache`, ranged
  reads from `quince_icloud::drive_parts`, shared with the desktop folder). A file an app writes
  is held in `quince_vfs::writes`' journal, with the version it was opened at, and sent by
  WorkManager work (`DriveWrites.kt`), so a save survives the process and never overwrites
  another device's change.
- **Background work** is WorkManager's (`SyncWork.kt`, `Work.kt`): the sync adapters only hand a
  sync to a worker and wait for it, periodic syncs are WorkManager's, photo backup has its own
  worker started by MediaStore, and Send to iPhone is a user-initiated job on Android 14+.
  Every piece of work is named for the engine's account tag.
- **The session** is kept alive before every sync (`Account::ensure_session`). When Apple ends
  it (it wants the password or a code), the account's work stops and one notification says to
  sign in again (`SessionEnded.kt`); signing in clears it.
- **Advanced Data Protection:** Home reads where the approval stands from what the phone holds
  (`data_access_standing`, no request) and asks for it only when the person presses Approve
  (`data_access_ask`, then `data_access_follow`), as the desktop's `data_access.rs` does. A
  service waiting on it is reported as waiting, nothing in its queue is sent, and it is not
  retried until the approval lands.

## Threads and async

- **Desktop:** Tauri's multi-threaded tokio runtime runs the commands and workers. Long stores
  run under `tokio::task::block_in_place` so other tasks on that worker keep going. Picture
  work goes through `cpu.rs`: work for the screen runs on blocking threads, work done ahead
  runs on one background worker so it never starves scrolling. FUSE serves on its own threads
  (four) and uses `LibraryLock::blocking_lock` / `blocking_read`. Cloud Files callbacks only
  copy what they are given and hand the work to the runtime; they never wait inside Windows'
  callback.
- **Per-service pass mutexes** (`NOTES_PASS`, `REMINDERS_PASS`, `CONTACTS_PASS`,
  `CALENDAR_PASS`, `PHOTOS_PASS` in `main.rs`, `dav::PASS`) put a service's sync pass and its
  writes in turn.
- **Android:** one tokio runtime for the process (two workers, `runtime()` in
  `quince-mobile`). Exported calls block on it; Kotlin calls them from `Dispatchers.IO`.
- **Panics:** release builds keep `panic = "unwind"`. `Library::atomically` rolls back a
  transaction that panics, a command that panics fails alone, a pass that panics fails alone
  (`quince_sync::pass::guarded`), and `drain_checked` turns a panic in a send into a refused
  change.

## Where each service lives

| Service | Apple side (`quince-icloud/src/`) | Library (`quince-core/src/`) | Shared (`quince-sync`) | Desktop (`src-tauri/src/`) | Window (`src/screens/`) | Android (`.../skykeep/`) |
|---|---|---|---|---|---|---|
| Notes | `notes.rs`, `topotext.rs` (CRDT), `ckdocument.rs`, `table.rs`, `notes_attach.rs`, `notes_locked.rs` | `notes.rs` | `notes.rs` | `notes_cmds.rs`, `main.rs` | `NotesView.tsx`, `notes/` | `sync/NotesSync.kt`, `ui/Notes.kt` |
| Reminders | `reminders.rs`, `reminders_details.rs`, `reminders_more.rs` | `reminders.rs`, `alarms.rs` | `reminders.rs` | `reminders_cmds.rs`, `alerts.rs` | `RemindersView.tsx` | `sync/RemindersSync.kt`, `sync/ReminderAlerts.kt`, `ui/Reminders.kt` |
| Calendar | `calendar.rs`, `sync.rs`, `rrule.rs`, `calendar_feeds.rs`, `occasions.rs`, `caldav.rs`, `ical_edit.rs` | `calendar.rs`, `dav.rs` | `calendar.rs` | `main.rs` (`calendar_*`), `dav.rs`, `outbox.rs` | `calendar/` | `sync/CalendarSync.kt`, `ui/NotSent.kt` |
| Contacts | `contacts.rs`, `contacts_address.rs`, `contacts_merge.rs` | `contacts.rs`, `contacts_labels.rs`, `contacts_merge.rs` | `contacts.rs` | `contacts_cmds.rs`, `contacts_merge.rs`, `outbox.rs` | `ContactsView.tsx`, `contacts/` | `sync/ContactsSync.kt`, `sync/GroupsSync.kt`, `ui/NotSent.kt` |
| Drive | `drive.rs`, `drive_changes.rs` | `drive.rs` | none (`quince-vfs`) | `mount.rs`, `drive_watch.rs`, `folder_download.rs` | `DriveView.tsx` | `sync/DriveProvider.kt`, `sync/DriveFiles.kt`, `ui/Drive.kt` |
| Photos | `photos.rs`, `photos_upload.rs`, `shared_albums.rs`, `shared_collections.rs` | `photos.rs`, `shared.rs`, `backup.rs` | `photos.rs` | `photos.rs`, `photo_cache.rs`, `mount_photos.rs`, `backup.rs`, `video.rs` | `PhotosView.tsx`, `photos/` | `sync/PhotosSync.kt`, `sync/PhotoBackup.kt`, `ui/Photos.kt` |

CloudKit paging and zones are in `quince-icloud/src/cloudkit.rs` and `zones.rs`. The push
channel is `courier.rs` and `push.rs` there, driven by `src-tauri/src/push.rs`. Search is
`quince-core/src/search.rs` (FTS5, kept current by triggers in `schema.rs`).

## Invariants

These are what reviews check first.

1. **Screens never wait on the network.** They read the library. A slow or failed refresh
   leaves the last good data on screen.
2. **Notes, Reminders, Contacts and Calendar are only changed through the outbox.** The local
   change and its queue entry are written in one transaction. A command never sends one of
   these straight to Apple.
3. **Every send is safe to repeat.** New records carry their name from the start, and edits
   are merges against a base.
4. **Accounts never mix.** A pass stores only into the library its account owns, a queue
   stops when the library changes hands, and an account's library is set aside, never merged
   or deleted.
5. **Only four kinds of HTTP client exist**, all built in `quince-auth`: `apple_only()` for
   iCloud (its resolver refuses any host outside Apple's domains before a connection exists),
   `listed_only()` for subscribed calendars, `account_only()` for a CalDAV account the person
   added, and `releases::releases_only()` for Skykeep's own updates.
   `crates/quince-auth/tests/apple_only.rs` must pass. No telemetry and no crash reporting.
6. **Nothing personal leaves in a log.** `quince_sync::quiet!` prints only in debug builds and
   never with names, ids, addresses or Apple's replies. Diagnostics (`SKYKEEP_LOG=1`) write
   to a local file and every line passes through `quince_auth::diag`, which strips ids and
   query strings. The Android engine's log cuts every URL at its `?` (`quince-mobile`,
   `log::redact`). Secrets live in the system keyring or the Android Keystore, and the cookie
   file is sealed with a key kept there (`quince_auth::sealed`).
7. **Apple's replies are untrusted input.** A missing field or a malformed date is an error,
   never a panic. Numbers round-trip exactly (`serde_json` with `float_roundtrip`), because a
   save that rewrote a long decimal would change other records.
8. **Nothing is lost.** Merges keep both sides, deletes go to Recently Deleted where Apple has
   one, and a file changed elsewhere is never overwritten.
