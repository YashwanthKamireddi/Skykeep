# quince-sync

A service's sync, shared by the desktop app and the Android engine. Nothing in it knows about a
window or a platform.

- **`library_lock::LibraryLock`**: the library behind one writer lock, with read-only WAL
  connections beside it for reads that would otherwise wait on a sync pass. `lock`,
  `blocking_lock`, `lock_for_pass`, `read`, `blocking_read`, `forget_readers`.
- **`library_lock::hold_or_mark`**: one Skykeep per library (`skykeep.lock`, or `skykeep.pid`
  where the disk takes no locks).
- **`queue`**: `drain_as` sends one service's queued changes oldest first as one account, and
  stops as soon as the library belongs to someone else. `transient` decides between retry and
  refuse. `account_tag` and `asked_as` name accounts the way the library does. `drain` skips
  the owner check, for tests.
- **`notes`, `reminders`**: each kind of change as `record_*` (local change and queue entry in
  one transaction), `lay` (its local effect), `replay` (lay everything still waiting over a
  pass), plus `apply`, `refresh` and `send`.
- **`photos`**: the paged library pass (`apply` lays waiting flag changes over each round), and
  the flags through the queue: `record_flags` (up to `AT_ONCE` photos a change; an unsent one
  undone is taken back), `lay`, `replay`, and `send` / `send_with` (what Apple took kept, the
  rest narrowed and retried or put back; a delete for good kept aside until Apple confirms).
- **`quiet!`**: messages printed in debug builds only, never with personal data.

Contacts and Calendar sends still live in the desktop app (`src-tauri/src/outbox.rs`). On
Android those two go through Android's own providers instead.

Depends on `quince-core` and `quince-icloud`. Used by `quince-app` and `quince-mobile`.

## Tests

```sh
cargo nextest run -p quince-sync         # the lock, and Photos' flags against a pretend Apple
cargo nextest run -p quince-app outbox   # the queue, against a pretend Apple
```
