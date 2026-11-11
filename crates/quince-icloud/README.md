# quince-icloud

Clients for Apple's iCloud web services. These are private endpoints, not a published API, so
every reply is untrusted: a missing field or a malformed date is an `Error`, never a panic.

- **`Error`**: `Network`, `Protocol`, `ReauthRequired`, `NeedsDeviceApproval`, `Conflict`,
  `Busy`, `Refused` and the rest. `quince_sync::queue::transient` uses these to decide whether
  a queued change is retried or held as refused.
- **CloudKit** (`cloudkit.rs`, `zones.rs`): `ZoneClient` reads a zone through `changes/zone`
  with paging and sync tokens, and writes through `records/modify`. Notes, Reminders and Photos
  are CloudKit.
- **Per service:** `notes.rs` with `topotext.rs` (Apple's text CRDT), `ckdocument.rs`,
  `table.rs`, `notes_attach.rs`, `notes_locked.rs`; `reminders.rs`, `reminders_details.rs`,
  `reminders_more.rs`; `calendar.rs`, `sync.rs`, `rrule.rs`, `calendar_feeds.rs`,
  `occasions.rs`; `contacts.rs` and its helpers; `drive.rs`, `drive_changes.rs`; `photos.rs`,
  `photos_upload.rs`, `shared_albums.rs`, `shared_collections.rs`.
- **Other calendar servers:** `caldav.rs`, `nextcloud.rs` (browser sign-in), `ical_edit.rs`
  (line-level edits that keep fields Skykeep doesn't model).
- **Push:** `courier.rs` (Apple's web push socket) and `push.rs`.

Where it matters, a service has a `fetch_*` that only talks to Apple and an `apply_*` that
writes the result into a `quince_core::Library`, so callers can fetch without holding the
library lock (`notes::fetch_cloudkit` / `apply_cloudkit`, and the same in `reminders`).

Depends on `quince-core`, but not on `quince-auth`: callers pass a `reqwest::Client` built
there, the service's root URL and the session's parameters.

## Tests

```sh
cargo nextest run -p quince-icloud
```

Unit tests decode trimmed, anonymised replies with the same functions the app uses.
`tests/fuzz_notes.rs` and `tests/table.rs` exercise the note and table decoders. Add a test for
every field you parse or write, and never commit a reply captured from a real account.
