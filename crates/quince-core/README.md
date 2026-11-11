# quince-core

The local library: one SQLite database per account, and the only thing the screens read.

- **`Library`**: `open(dir)` (the writer), `open_reader(dir)` (read-only, WAL),
  `open_or_recover`, and `atomically`, which runs a closure in one transaction and rolls it
  back on error or panic. Key/value state (`state`, `set_state`) holds the owner tag and other
  small facts; each service keeps its sync token.
- **Per service:** `notes.rs`, `reminders.rs` and `alarms.rs`, `calendar.rs`, `dav.rs` (CalDAV
  accounts), `contacts.rs`, `drive.rs`, `photos.rs` and `shared.rs` (Shared Albums),
  `backup.rs`.
- **`outbox.rs`**: the queue of local changes (`outbox_push`, `outbox_take`, `outbox_done`,
  `outbox_retry`, `outbox_refuse`, `outbox_resend`, `outbox_discard`). Sending is in
  `quince-sync`.
- **`schema.rs`**: `MIGRATIONS`, append only, tracked by `user_version`. Never edit a shipped
  migration; add a new one. `FIXUPS` are the few that need Rust as well as SQL.
- **`search.rs`**: FTS5 search across every app, kept current by triggers in `schema.rs`.
- **`changes.rs`**: which rows a write touched, so screens redraw only those.
- **`health.rs`**: what each service's last refresh did.
- **`export.rs`**: whole-library reads for Export Everything.
- **`fold.rs`**: case- and accent-insensitive comparison for search and sorting.

Depends on no other workspace crate and makes no network calls. Used by every crate above it.

## Tests

```sh
cargo nextest run -p quince-core
```

Unit tests open throwaway databases. `tests/scale.rs` stores tens of thousands of rows and runs
one test at a time (the `scale` group in `.config/nextest.toml`).
