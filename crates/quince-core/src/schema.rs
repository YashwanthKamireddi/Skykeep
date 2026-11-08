//! Schema migrations, applied in order and tracked by SQLite's `user_version`.
//!
//! Append only. Editing a shipped migration would leave existing installs on a schema that
//! no longer matches what the code expects.

/// Every migration, oldest first. The index plus one is the `user_version` it produces.
pub const MIGRATIONS: &[&str] = &[
    // 1 — calendars and events.
    r#"
    CREATE TABLE calendars (
        guid        TEXT PRIMARY KEY,
        title       TEXT NOT NULL,
        colour      TEXT,
        -- Apple's collection tag. Unchanged ctag means nothing in this calendar moved, so
        -- the whole collection can be skipped on the next sync.
        ctag        TEXT,
        sort_order  INTEGER NOT NULL DEFAULT 0,
        enabled     INTEGER NOT NULL DEFAULT 1,
        updated_at  INTEGER NOT NULL
    ) STRICT;

    CREATE TABLE events (
        guid          TEXT PRIMARY KEY,
        calendar_guid TEXT NOT NULL REFERENCES calendars(guid) ON DELETE CASCADE,
        title         TEXT NOT NULL,
        location      TEXT,
        -- Unix seconds, UTC. The originating zone is kept separately so an all-day event or
        -- a meeting booked in another timezone still renders the way Apple shows it.
        starts_at     INTEGER NOT NULL,
        ends_at       INTEGER NOT NULL,
        all_day       INTEGER NOT NULL DEFAULT 0,
        timezone      TEXT,
        url           TEXT,
        notes         TEXT,
        etag          TEXT,
        updated_at    INTEGER NOT NULL
    ) STRICT;

    -- The calendar view is always "what is happening between these two instants".
    CREATE INDEX events_by_start ON events (starts_at);
    CREATE INDEX events_by_calendar ON events (calendar_guid, starts_at);

    -- Where each service got to, so a sync resumes instead of refetching everything.
    CREATE TABLE sync_state (
        service        TEXT PRIMARY KEY,
        token          TEXT,
        last_synced_at INTEGER,
        last_error     TEXT
    ) STRICT;
    "#,
    // 2 — reminder lists and reminders.
    r#"
    CREATE TABLE reminder_lists (
        guid       TEXT PRIMARY KEY,
        title      TEXT NOT NULL,
        colour     TEXT,
        ctag       TEXT,
        sort_order INTEGER NOT NULL DEFAULT 0,
        enabled    INTEGER NOT NULL DEFAULT 1,
        updated_at INTEGER NOT NULL
    ) STRICT;

    CREATE TABLE reminders (
        guid         TEXT PRIMARY KEY,
        list_guid    TEXT NOT NULL REFERENCES reminder_lists(guid) ON DELETE CASCADE,
        title        TEXT NOT NULL,
        notes        TEXT,
        -- Unix seconds, UTC. Null means no due date, which is the common case.
        due_at       INTEGER,
        -- Null while outstanding; the completion instant once ticked off.
        completed_at INTEGER,
        priority     INTEGER NOT NULL DEFAULT 0,
        etag         TEXT,
        updated_at   INTEGER NOT NULL
    ) STRICT;

    CREATE INDEX reminders_by_list ON reminders (list_guid, completed_at, due_at);
    CREATE INDEX reminders_by_due ON reminders (due_at);
    "#,
    // 3 — keep Apple's original payload for each reminder.
    //
    // Writes must echo the whole task back. Reconstructing one from parsed columns would
    // silently drop fields Apple sent but we do not model, which is how a sync corrupts
    // someone's data. Storing the original makes a write a mutation of what Apple gave us.
    r#"
    ALTER TABLE reminders ADD COLUMN raw TEXT;
    "#,
    // 4 — notes and their folders.
    r#"
    CREATE TABLE note_folders (
        guid       TEXT PRIMARY KEY,
        title      TEXT NOT NULL,
        sort_order INTEGER NOT NULL DEFAULT 0,
        updated_at INTEGER NOT NULL
    ) STRICT;

    CREATE TABLE notes (
        guid          TEXT PRIMARY KEY,
        folder_guid   TEXT REFERENCES note_folders(guid) ON DELETE SET NULL,
        title         TEXT NOT NULL,
        -- The first line or so, which Apple sends separately so a list can render without
        -- decoding every body.
        snippet       TEXT,
        body          TEXT,
        -- A note the owner locked is genuinely encrypted; we can show that it exists and
        -- nothing more, so say so rather than rendering it blank.
        locked        INTEGER NOT NULL DEFAULT 0,
        pinned        INTEGER NOT NULL DEFAULT 0,
        created_at    INTEGER,
        modified_at   INTEGER,
        etag          TEXT,
        updated_at    INTEGER NOT NULL
    ) STRICT;

    CREATE INDEX notes_by_folder ON notes (folder_guid, pinned DESC, modified_at DESC);
    CREATE INDEX notes_by_modified ON notes (modified_at DESC);
    "#,
    // 5 — contacts, with their phone numbers and addresses normalised alongside.
    r#"
    CREATE TABLE contacts (
        guid        TEXT PRIMARY KEY,
        first_name  TEXT,
        last_name   TEXT,
        company     TEXT,
        is_company  INTEGER NOT NULL DEFAULT 0,
        -- What to sort and search on, so neither depends on which name fields exist.
        sort_name   TEXT NOT NULL,
        birthday    TEXT,
        notes       TEXT,
        etag        TEXT,
        updated_at  INTEGER NOT NULL
    ) STRICT;

    CREATE INDEX contacts_by_name ON contacts (sort_name);

    -- Phones, emails and the rest share a shape, so they share a table.
    CREATE TABLE contact_methods (
        contact_guid TEXT NOT NULL REFERENCES contacts(guid) ON DELETE CASCADE,
        kind         TEXT NOT NULL,
        label        TEXT,
        value        TEXT NOT NULL,
        position     INTEGER NOT NULL DEFAULT 0
    ) STRICT;

    CREATE INDEX contact_methods_by_contact ON contact_methods (contact_guid, kind, position);
    "#,
    // 6 — iCloud Drive, stored as the tree Apple exposes.
    r#"
    CREATE TABLE drive_items (
        -- Apple's own identifier, which encodes the kind and zone: FOLDER::com.apple.CloudDocs::root
        drivewsid   TEXT PRIMARY KEY,
        parent_id   TEXT,
        name        TEXT NOT NULL,
        extension   TEXT,
        -- FOLDER, APP_LIBRARY or FILE.
        kind        TEXT NOT NULL,
        size        INTEGER,
        child_count INTEGER,
        created_at  INTEGER,
        modified_at INTEGER,
        etag        TEXT,
        updated_at  INTEGER NOT NULL
    ) STRICT;

    -- Browsing is always "what is inside this folder", ordered folders first.
    CREATE INDEX drive_by_parent ON drive_items (parent_id, kind, name);
    "#,
    // 7 — flagging, which Apple surfaces as one of its smart lists.
    r#"
    ALTER TABLE reminders ADD COLUMN flagged INTEGER NOT NULL DEFAULT 0;
    CREATE INDEX reminders_flagged ON reminders (flagged, completed_at);
    "#,
    // 8 — recurrence, so completing a repeating reminder can advance it.
    //
    // Ticking a repeating reminder does not mean "set completed" — Apple already holds it
    // completed while the record points at the next occurrence. It means "move to the one
    // after", which needs the rule.
    r#"
    ALTER TABLE reminders ADD COLUMN recur_frequency INTEGER;
    ALTER TABLE reminders ADD COLUMN recur_interval INTEGER;
    "#,
    // 9 — whether a note can be saved without damage.
    //
    // A note body is Apple's document protobuf: text surrounded by version vectors, folder
    // links and merge state. Saving means rewriting that document in place. Where the
    // document cannot be reproduced byte for byte, this app will not write it, and the
    // interface has to be able to say so before someone starts typing rather than after.
    r#"
    ALTER TABLE notes ADD COLUMN editable INTEGER NOT NULL DEFAULT 1;
    "#,
    // 10 — when a service last did a full pass, rather than a delta.
    //
    // A delta is cheap and is what almost every refresh should be, but it can only ever
    // apply what Apple chose to send. A tombstone is delivered once; a sync that consumed
    // one before the code knew what to do with it left that record stranded forever. A full
    // pass now and then is the floor under that, and this is what schedules it.
    r#"
    ALTER TABLE sync_state ADD COLUMN last_full_at INTEGER;
    "#,
    // 11 — subtasks.
    //
    // Apple nests a reminder under another with a ParentReminder reference. Without it,
    // every subtask showed as a sibling of the thing it belongs to, which reads as a flat
    // list of fragments rather than as one piece of work broken down.
    r#"
    ALTER TABLE reminders ADD COLUMN parent_guid TEXT;
    CREATE INDEX reminders_by_parent ON reminders (parent_guid);
    "#,
    // 12 — a contact's middle name.
    //
    // Apple keeps one and four people in a real address book have one; without it they were
    // shown under a name that was not quite theirs.
    r#"
    ALTER TABLE contacts ADD COLUMN middle_name TEXT;
    "#,
    // 13 — where a contact's photo lives.
    //
    // The URL is Apple's; the signature changes when the picture does, which is what tells
    // a cached copy it has gone stale. The bytes themselves are not kept here: an image
    // belongs on disk where the webview can load it directly, not in a row that has to
    // cross an IPC boundary every time a list scrolls.
    r#"
    ALTER TABLE contacts ADD COLUMN photo_url TEXT;
    ALTER TABLE contacts ADD COLUMN photo_signature TEXT;
    "#,
    // 14 — the square of a picture its owner chose to show.
    //
    // Apple stores the photo whole and records the crop beside it. One picture on a real
    // account is 1696x2528 with a crop of 1279x1279 at 251,1249 — a portrait whose face sits
    // near the bottom. Centring it, which is what any viewer does without this, shows a chest.
    // Kept as `x,y,width,height` because it is only ever read and written together.
    r#"
    ALTER TABLE contacts ADD COLUMN photo_crop TEXT;
    "#,
    // 15 — what Apple says about each calendar.
    //
    // A subscribed calendar — a holiday feed, a sports fixture list — and a calendar shared
    // with you for reading only both come back with readOnly set. Offering to put an event in
    // one produces a refusal that reads as a bug in this app rather than as the truth.
    //
    // And Apple names the default calendar outright. Picking the first of the list instead is
    // the same mistake that put every new reminder in whichever list happened to sort first.
    r#"
    ALTER TABLE calendars ADD COLUMN writable INTEGER NOT NULL DEFAULT 1;
    ALTER TABLE calendars ADD COLUMN is_default INTEGER NOT NULL DEFAULT 0;
    "#,
    // 16 — a notes folder can sit inside another one.
    //
    // Apple Notes has nested folders and keeps the link on the folder record as
    // `ParentFolder`. This account has only the two Apple makes for itself, neither of which
    // is nested, so the field was never seen — but a folder created with one is accepted and
    // handed straight back, which is how it was confirmed. Flattened, somebody's filing comes
    // out as one long list with every level jumbled together.
    r#"
    ALTER TABLE note_folders ADD COLUMN parent_guid TEXT;
    "#,
    // 17 — where a reminder's time is kept.
    //
    // Apple records a named zone beside the due date, because nine in the morning means nine
    // in the morning wherever the person is rather than a fixed instant. Without it a
    // repeating reminder was stepped in UTC, where a day is always exactly twenty-four hours —
    // so on the two days a year a clock moves, a daily reminder at nine moved to ten and six
    // months later back again. This account is in one of the few zones that keeps a single
    // offset all year, which is why it was never visible here.
    r#"
    ALTER TABLE reminders ADD COLUMN timezone TEXT;
    "#,
    // 18 — somewhere to keep a thing learned from the account.
    //
    // The first of these is a note document Apple wrote, kept so a new note can be given the
    // same shape. It cannot be fetched on demand: notes sit far enough into the zone walk that
    // the first page of a hundred and thirty-nine records held none of them, and walking the
    // whole zone to write one note is not a trade worth making. Sync already reads every note,
    // so sync is where it is noticed.
    r#"
    CREATE TABLE app_state (
      key   TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );
    "#,
    // 19 — sections inside a reminders list.
    //
    // A section is its own record, but which reminder sits in which one is not on the
    // reminder: it is a file hung off the list, a JSON array of { memberID, groupID } where
    // groupID is the section. A reminder with no groupID sits outside every section.
    r#"
    CREATE TABLE reminder_sections (
        guid        TEXT PRIMARY KEY,
        list_guid   TEXT NOT NULL,
        title       TEXT NOT NULL,
        created_at  INTEGER,
        updated_at  INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX reminder_sections_by_list ON reminder_sections (list_guid, created_at);
    ALTER TABLE reminders ADD COLUMN section_guid TEXT;
    "#,
    // 20 — what a reminders list looks like and where it sits.
    //
    // Apple keeps a list's glyph (`BadgeEmblem`), the group it is filed under (`ParentList`),
    // whether it is pinned beside Today and Scheduled (`PinnedDate`) and how its reminders
    // are ordered (`SortingStyle`). None of it was read, so every list drew as the same white
    // dot in one flat column. Groups are folders of lists, kept apart from the lists
    // themselves so nothing that counts or offers lists can mistake one for a list.
    r#"
    ALTER TABLE reminder_lists ADD COLUMN emblem TEXT;
    ALTER TABLE reminder_lists ADD COLUMN group_guid TEXT;
    ALTER TABLE reminder_lists ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE reminder_lists ADD COLUMN sorting TEXT;
    CREATE TABLE reminder_groups (
        guid        TEXT PRIMARY KEY,
        title       TEXT NOT NULL,
        sort_order  INTEGER NOT NULL,
        updated_at  INTEGER NOT NULL
    ) STRICT;
    "#,
    // 21 — the order a list keeps its reminders in.
    //
    // A list set to manual order holds it as `ReminderIDs`, every reminder in it by id in the
    // order they were dragged into. Ignored, every list came out sorted by due date, which is
    // only right for the lists set to sort that way.
    r#"
    ALTER TABLE reminder_lists ADD COLUMN reminder_order TEXT;
    "#,
    // 22 — read every reminders list again, once.
    //
    // 20 and 21 added what a list looks like and how it orders its reminders, but a library
    // made before them only ever asks Apple what changed since its last pass — and a list
    // nobody has touched never changes, so its new columns stayed empty and it kept drawing
    // as a white dot with its colour stored as raw JSON. Forgetting the position makes the
    // next pass a whole one, which fills them in.
    r#"
    UPDATE sync_state SET token = NULL WHERE service = 'reminders';
    "#,
    // 23 — reminder alarms, so this app can ring when the phone would.
    //
    // A reminder's alert is not on the reminder: it lists `AlarmIDs`, each an `Alarm` record
    // naming an `AlarmTrigger`, which holds the wall-clock moment to fire. The two are kept as
    // Apple has them because either can change without the other in a pass. `alarm_fired`
    // remembers every alert already shown — by alarm and moment, so an alarm moved to a new
    // time rings again and one already shown never does twice, across restarts. Snoozes are
    // this device's own and never go to Apple. The position is forgotten once more so the
    // next pass reads every alarm already on the account.
    r#"
    CREATE TABLE reminder_alarms (
        alarm_id        TEXT PRIMARY KEY,
        reminder_guid   TEXT NOT NULL,
        trigger_id      TEXT NOT NULL,
        acknowledged_at INTEGER
    ) STRICT;
    CREATE INDEX reminder_alarms_by_trigger ON reminder_alarms (trigger_id);
    CREATE TABLE alarm_triggers (
        trigger_id TEXT PRIMARY KEY,
        fires_at   INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX alarm_triggers_by_time ON alarm_triggers (fires_at);
    CREATE TABLE alarm_fired (
        alarm_id TEXT NOT NULL,
        fires_at INTEGER NOT NULL,
        fired_at INTEGER NOT NULL,
        PRIMARY KEY (alarm_id, fires_at)
    ) STRICT;
    CREATE TABLE alarm_snoozes (
        reminder_guid TEXT PRIMARY KEY,
        fires_at      INTEGER NOT NULL
    ) STRICT;
    UPDATE sync_state SET token = NULL WHERE service = 'reminders';
    "#,
    // All-day reminders: Apple's `AllDay`, so a reminder due on a day shows as that day and
    // not as midnight. The position is forgotten so the next pass reads it on every reminder.
    r#"
    ALTER TABLE reminders ADD COLUMN all_day INTEGER NOT NULL DEFAULT 0;
    UPDATE sync_state SET token = NULL WHERE service = 'reminders';
    "#,
    // A note's formatting — Apple's attribute runs — kept beside its body, and the notes zone
    // read again from the start so every note has them.
    r#"
    ALTER TABLE notes ADD COLUMN runs TEXT;
    UPDATE sync_state SET token = NULL WHERE service = 'notes';
    "#,
    // iCloud Photos: every photo's description, mirrored through the change feed, so each
    // view is a query here. The images are cached on disk, not in here.
    r#"
    CREATE TABLE photos (
        id         TEXT PRIMARY KEY,
        master     TEXT NOT NULL,
        taken_at   INTEGER NOT NULL,
        added_at   INTEGER,
        kind       TEXT NOT NULL,
        width      INTEGER NOT NULL,
        height     INTEGER NOT NULL,
        duration   REAL,
        filename   TEXT,
        size       INTEGER,
        uti        TEXT,
        favorite   INTEGER NOT NULL DEFAULT 0,
        hidden     INTEGER NOT NULL DEFAULT 0,
        deleted    INTEGER NOT NULL DEFAULT 0,
        deleted_at INTEGER,
        subtype    INTEGER NOT NULL DEFAULT 0,
        change_tag TEXT
    ) STRICT;
    CREATE INDEX photos_by_taken ON photos (deleted, hidden, taken_at DESC);
    CREATE TABLE photo_albums (
        id         TEXT PRIMARY KEY,
        name       TEXT NOT NULL,
        position   INTEGER NOT NULL DEFAULT 0,
        change_tag TEXT
    ) STRICT;
    CREATE TABLE photo_album_items (
        album    TEXT NOT NULL,
        asset    TEXT NOT NULL,
        position INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (album, asset)
    ) STRICT;
    "#,
    // Photos uploads, kept until each is done: quitting halfway resumes where it stopped, and
    // a file is never thought sent until Apple has made the photo.
    r#"
    CREATE TABLE photo_uploads (
        id       TEXT PRIMARY KEY,
        batch    TEXT NOT NULL,
        path     TEXT NOT NULL,
        name     TEXT NOT NULL,
        state    TEXT NOT NULL,
        error    TEXT,
        asset    TEXT,
        added_at INTEGER NOT NULL
    ) STRICT;
    "#,
    // Which photo is in which album, for a library made before the Photos migration carried
    // it: that one had already run there, so the table never came. Photos is read again
    // from the start so every album fills.
    r#"
    CREATE TABLE IF NOT EXISTS photo_album_items (
        album    TEXT NOT NULL,
        asset    TEXT NOT NULL,
        position INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (album, asset)
    ) STRICT;
    UPDATE sync_state SET token = NULL WHERE service = 'photos';
    "#,
    // When a photo in Recently Deleted goes for good, as Apple counts it; Photos is read
    // again so every deleted photo has it.
    r#"
    ALTER TABLE photos ADD COLUMN expunge_at INTEGER;
    UPDATE sync_state SET token = NULL WHERE service = 'photos';
    "#,
    // A photo's caption, as typed in Photos on any device; Photos is read again so every
    // photo has its own.
    r#"
    ALTER TABLE photos ADD COLUMN caption TEXT;
    UPDATE sync_state SET token = NULL WHERE service = 'photos';
    "#,
    // The album open when a photo was added to Photos, which it goes into once Apple has it.
    r#"
    ALTER TABLE photo_uploads ADD COLUMN album TEXT;
    "#,
    // What was kept as the caption is Photos' title (`captionEnc`); the caption proper is
    // `extendedDescEnc`. Photos is read again so each has its own.
    r#"
    ALTER TABLE photos RENAME COLUMN caption TO title;
    ALTER TABLE photos ADD COLUMN caption TEXT;
    UPDATE sync_state SET token = NULL WHERE service = 'photos';
    "#,
    // A photo's star rating; Photos read again so every photo has its own.
    r#"
    ALTER TABLE photos ADD COLUMN rating INTEGER NOT NULL DEFAULT 0;
    UPDATE sync_state SET token = NULL WHERE service = 'photos';
    "#,
    // A photo's keywords (a JSON list) and where it was taken; Photos read again so every
    // photo has its own.
    r#"
    ALTER TABLE photos ADD COLUMN keywords TEXT;
    ALTER TABLE photos ADD COLUMN latitude REAL;
    ALTER TABLE photos ADD COLUMN longitude REAL;
    UPDATE sync_state SET token = NULL WHERE service = 'photos';
    "#,
    // Folders of albums, and which folder an album sits in; Photos read again so they come.
    r#"
    ALTER TABLE photo_albums ADD COLUMN folder INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE photo_albums ADD COLUMN parent TEXT;
    UPDATE sync_state SET token = NULL WHERE service = 'photos';
    "#,
    // How each photo was rendered (a Portrait is 8); Photos read again so every photo has it.
    r#"
    ALTER TABLE photos ADD COLUMN rendered INTEGER NOT NULL DEFAULT 0;
    UPDATE sync_state SET token = NULL WHERE service = 'photos';
    "#,
    // Changes made here and not yet taken by Apple, in the order they were made: the library
    // shows them at once, and they go to Apple behind it, kept across a restart or a lost
    // connection until Apple has them.
    r#"
    CREATE TABLE outbox (
        id       INTEGER PRIMARY KEY AUTOINCREMENT,
        service  TEXT NOT NULL,
        record   TEXT NOT NULL,
        kind     TEXT NOT NULL,
        payload  TEXT NOT NULL,
        made_at  INTEGER NOT NULL,
        tries    INTEGER NOT NULL DEFAULT 0,
        error    TEXT
    ) STRICT;
    CREATE INDEX outbox_by_service ON outbox (service, id);
    "#,
    // A video's length was kept as Apple sends it, in milliseconds, and shown as seconds: a
    // one-minute clip read 1036:24. Every one stored so far is in milliseconds.
    r#"
    UPDATE photos SET duration = duration / 1000.0 WHERE duration IS NOT NULL;
    "#,
    // The burst each photo was shot in, and whether it is the burst's chosen one; Photos read
    // again so every photo has them.
    r#"
    ALTER TABLE photos ADD COLUMN burst TEXT;
    ALTER TABLE photos ADD COLUMN burst_pick INTEGER NOT NULL DEFAULT 0;
    CREATE INDEX photos_by_burst ON photos (burst) WHERE burst IS NOT NULL;
    UPDATE sync_state SET token = NULL WHERE service = 'photos';
    "#,
    // A Live Photo's moving part, its size: what the Photos folder lists it at. Photos read
    // again so every Live Photo has it.
    r#"
    ALTER TABLE photos ADD COLUMN live_size INTEGER;
    UPDATE sync_state SET token = NULL WHERE service = 'photos';
    "#,
    // Notes read once more from the start, so every stored note knows its block quotes
    // (paragraph field 8): a note stored before would hand back its quoted lines unquoted on
    // its next edit, and the quotes would be lost at Apple.
    r#"
    UPDATE sync_state SET token = NULL, last_full_at = NULL WHERE service = 'notes';
    "#,
    // A change Apple turned down for good stays, marked with when, rather than being thrown
    // away: what was typed is kept for the person to see, send again or let go.
    r#"
    ALTER TABLE outbox ADD COLUMN refused_at INTEGER;
    "#,
    // 43 — a contact's postal addresses, part by part.
    //
    // They were kept only as the flattened lines of an envelope, so a street, a town or a
    // postcode could not be changed on its own. Apple's card keeps each part separately
    // (`streetAddresses[].field.{street, city, state, postalCode, country, ...}`); this keeps
    // them the same way. The raw card was never stored, so nothing here can be filled from
    // what is already in the library: the position is forgotten, and the next Contacts pass
    // reads the book whole and fills it.
    r#"
    CREATE TABLE contact_addresses (
        contact_guid            TEXT NOT NULL REFERENCES contacts(guid) ON DELETE CASCADE,
        position                INTEGER NOT NULL,
        label                   TEXT,
        street                  TEXT,
        sub_locality            TEXT,
        city                    TEXT,
        sub_administrative_area TEXT,
        state                   TEXT,
        postal_code             TEXT,
        country                 TEXT,
        country_code            TEXT,
        PRIMARY KEY (contact_guid, position)
    ) STRICT;
    UPDATE sync_state SET token = NULL WHERE service = 'contacts';
    "#,
    // 44 — reminder details Apple keeps beside the reminder: tags, a link, early alerts, and
    // custom smart lists.
    //
    // A tag is a `Hashtag` record (Name, Reminder) whose bare id the reminder lists in
    // `HashtagIDs`; a link is an `Attachment` record of Type "URL" listed in `AttachmentIDs`;
    // an early alert ("remind me 30 minutes early") is JSON in the reminder's own
    // `DueDateDeltaAlertsData`; a smart list is a `SmartList` record whose `FilterData` is
    // JSON. Reminders read again from the start so every reminder has them.
    r#"
    CREATE TABLE reminder_hashtags (
        guid          TEXT PRIMARY KEY,
        reminder_guid TEXT NOT NULL,
        name          TEXT NOT NULL,
        updated_at    INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX reminder_hashtags_by_reminder ON reminder_hashtags (reminder_guid);
    CREATE INDEX reminder_hashtags_by_name ON reminder_hashtags (name COLLATE NOCASE);
    CREATE TABLE reminder_attachments (
        guid          TEXT PRIMARY KEY,
        reminder_guid TEXT NOT NULL,
        kind          TEXT NOT NULL,
        url           TEXT,
        uti           TEXT,
        updated_at    INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX reminder_attachments_by_reminder ON reminder_attachments (reminder_guid);
    CREATE TABLE reminder_early_alerts (
        id            TEXT NOT NULL,
        reminder_guid TEXT NOT NULL,
        count         INTEGER NOT NULL,
        unit          INTEGER NOT NULL,
        account_id    TEXT,
        PRIMARY KEY (reminder_guid, id)
    ) STRICT;
    CREATE TABLE reminder_smart_lists (
        guid       TEXT PRIMARY KEY,
        kind       TEXT NOT NULL,
        title      TEXT,
        colour     TEXT,
        emblem     TEXT,
        pinned     INTEGER NOT NULL DEFAULT 0,
        sorting    TEXT,
        filter     TEXT,
        updated_at INTEGER NOT NULL
    ) STRICT;
    UPDATE sync_state SET token = NULL WHERE service = 'reminders';
    "#,
    // 45 — one index over the words of everything searchable: notes (title and text), reminders
    // still to do (title and notes), events (title, place, notes), contacts (names, company,
    // every phone, email and link — numbers digits-only too), Drive names and photos (title,
    // file name, caption, keywords, and the year and month they were taken, as Photos finds
    // them). Searching it reads only the rows that hold the words, where every search read
    // every row and folded it.
    //
    // Words are found by how they begin ("spri" finds "Springfield"), case and accents aside
    // (`remove_diacritics 2`), and the marks of Indic scripts stay inside their word (`M*`), so
    // a Telugu or Hindi word is one word. The index holds no text of its own (`content=''`):
    // titles and snippets are read from the records, which stay the one copy. `search_docs`
    // numbers each indexed record and keeps what ranks it — `at`, the time it is ordered by,
    // and `sort`, a contact's place in the address book.
    //
    // The triggers keep it current in the same transaction as every write — a sync pass, an
    // edit, a sign-out — whoever makes it. A record first forgets its old words and then, if it
    // is one search shows (a note outside Recently Deleted, a reminder still to do), is indexed
    // again.
    r#"
    CREATE TABLE search_docs (
        id   INTEGER PRIMARY KEY,
        kind TEXT NOT NULL,
        key  TEXT NOT NULL,
        at   INTEGER,
        sort TEXT,
        UNIQUE (kind, key)
    ) STRICT;
    CREATE VIRTUAL TABLE search_text USING fts5(
        title, body,
        content = '', contentless_delete = 1, prefix = '1 2 3',
        tokenize = "unicode61 remove_diacritics 2 categories 'L* N* Co M*'"
    );
    CREATE TRIGGER search_notes_insert AFTER INSERT ON notes BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'note' AND key = new.guid);
        DELETE FROM search_docs WHERE kind = 'note' AND key = new.guid;
        INSERT INTO search_docs (kind, key, at) SELECT 'note', new.guid, new.modified_at WHERE (new.folder_guid IS NULL OR new.folder_guid != 'TrashFolder-CloudKit');
        INSERT INTO search_text (rowid, title, body)
            SELECT id, new.title, new.body FROM search_docs WHERE kind = 'note' AND key = new.guid;
    END;
    CREATE TRIGGER search_notes_update AFTER UPDATE OF folder_guid, title, body, modified_at ON notes BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'note' AND key = old.guid);
        DELETE FROM search_docs WHERE kind = 'note' AND key = old.guid;
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'note' AND key = new.guid);
        DELETE FROM search_docs WHERE kind = 'note' AND key = new.guid;
        INSERT INTO search_docs (kind, key, at) SELECT 'note', new.guid, new.modified_at WHERE (new.folder_guid IS NULL OR new.folder_guid != 'TrashFolder-CloudKit');
        INSERT INTO search_text (rowid, title, body)
            SELECT id, new.title, new.body FROM search_docs WHERE kind = 'note' AND key = new.guid;
    END;
    CREATE TRIGGER search_notes_delete AFTER DELETE ON notes BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'note' AND key = old.guid);
        DELETE FROM search_docs WHERE kind = 'note' AND key = old.guid;
    END;
    INSERT INTO search_docs (kind, key, at) SELECT 'note', guid, modified_at FROM notes WHERE (folder_guid IS NULL OR folder_guid != 'TrashFolder-CloudKit');
    INSERT INTO search_text (rowid, title, body)
        SELECT d.id, X.title, X.body FROM search_docs d JOIN notes X ON d.kind = 'note' AND d.key = X.guid;
    CREATE TRIGGER search_reminders_insert AFTER INSERT ON reminders BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'reminder' AND key = new.guid);
        DELETE FROM search_docs WHERE kind = 'reminder' AND key = new.guid;
        INSERT INTO search_docs (kind, key, at) SELECT 'reminder', new.guid, new.due_at WHERE new.completed_at IS NULL;
        INSERT INTO search_text (rowid, title, body)
            SELECT id, new.title, new.notes FROM search_docs WHERE kind = 'reminder' AND key = new.guid;
    END;
    CREATE TRIGGER search_reminders_update AFTER UPDATE OF list_guid, title, notes, due_at, completed_at ON reminders BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'reminder' AND key = old.guid);
        DELETE FROM search_docs WHERE kind = 'reminder' AND key = old.guid;
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'reminder' AND key = new.guid);
        DELETE FROM search_docs WHERE kind = 'reminder' AND key = new.guid;
        INSERT INTO search_docs (kind, key, at) SELECT 'reminder', new.guid, new.due_at WHERE new.completed_at IS NULL;
        INSERT INTO search_text (rowid, title, body)
            SELECT id, new.title, new.notes FROM search_docs WHERE kind = 'reminder' AND key = new.guid;
    END;
    CREATE TRIGGER search_reminders_delete AFTER DELETE ON reminders BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'reminder' AND key = old.guid);
        DELETE FROM search_docs WHERE kind = 'reminder' AND key = old.guid;
    END;
    INSERT INTO search_docs (kind, key, at) SELECT 'reminder', guid, due_at FROM reminders WHERE completed_at IS NULL;
    INSERT INTO search_text (rowid, title, body)
        SELECT d.id, X.title, X.notes FROM search_docs d JOIN reminders X ON d.kind = 'reminder' AND d.key = X.guid;
    CREATE TRIGGER search_events_insert AFTER INSERT ON events BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'event' AND key = new.guid);
        DELETE FROM search_docs WHERE kind = 'event' AND key = new.guid;
        INSERT INTO search_docs (kind, key, at) SELECT 'event', new.guid, new.starts_at WHERE 1;
        INSERT INTO search_text (rowid, title, body)
            SELECT id, new.title, concat_ws(' ', new.location, new.notes) FROM search_docs WHERE kind = 'event' AND key = new.guid;
    END;
    CREATE TRIGGER search_events_update AFTER UPDATE OF title, location, notes, starts_at ON events BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'event' AND key = old.guid);
        DELETE FROM search_docs WHERE kind = 'event' AND key = old.guid;
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'event' AND key = new.guid);
        DELETE FROM search_docs WHERE kind = 'event' AND key = new.guid;
        INSERT INTO search_docs (kind, key, at) SELECT 'event', new.guid, new.starts_at WHERE 1;
        INSERT INTO search_text (rowid, title, body)
            SELECT id, new.title, concat_ws(' ', new.location, new.notes) FROM search_docs WHERE kind = 'event' AND key = new.guid;
    END;
    CREATE TRIGGER search_events_delete AFTER DELETE ON events BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'event' AND key = old.guid);
        DELETE FROM search_docs WHERE kind = 'event' AND key = old.guid;
    END;
    INSERT INTO search_docs (kind, key, at) SELECT 'event', guid, starts_at FROM events WHERE 1;
    INSERT INTO search_text (rowid, title, body)
        SELECT d.id, X.title, concat_ws(' ', X.location, X.notes) FROM search_docs d JOIN events X ON d.kind = 'event' AND d.key = X.guid;
    CREATE TRIGGER search_drive_items_insert AFTER INSERT ON drive_items BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'file' AND key = new.drivewsid);
        DELETE FROM search_docs WHERE kind = 'file' AND key = new.drivewsid;
        INSERT INTO search_docs (kind, key, at) SELECT 'file', new.drivewsid, new.modified_at WHERE 1;
        INSERT INTO search_text (rowid, title, body)
            SELECT id, new.name, new.extension FROM search_docs WHERE kind = 'file' AND key = new.drivewsid;
    END;
    CREATE TRIGGER search_drive_items_update AFTER UPDATE OF name, extension, modified_at ON drive_items BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'file' AND key = old.drivewsid);
        DELETE FROM search_docs WHERE kind = 'file' AND key = old.drivewsid;
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'file' AND key = new.drivewsid);
        DELETE FROM search_docs WHERE kind = 'file' AND key = new.drivewsid;
        INSERT INTO search_docs (kind, key, at) SELECT 'file', new.drivewsid, new.modified_at WHERE 1;
        INSERT INTO search_text (rowid, title, body)
            SELECT id, new.name, new.extension FROM search_docs WHERE kind = 'file' AND key = new.drivewsid;
    END;
    CREATE TRIGGER search_drive_items_delete AFTER DELETE ON drive_items BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'file' AND key = old.drivewsid);
        DELETE FROM search_docs WHERE kind = 'file' AND key = old.drivewsid;
    END;
    INSERT INTO search_docs (kind, key, at) SELECT 'file', drivewsid, modified_at FROM drive_items WHERE 1;
    INSERT INTO search_text (rowid, title, body)
        SELECT d.id, X.name, X.extension FROM search_docs d JOIN drive_items X ON d.kind = 'file' AND d.key = X.drivewsid;
    CREATE TRIGGER search_photos_insert AFTER INSERT ON photos BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'photo' AND key = new.id);
        DELETE FROM search_docs WHERE kind = 'photo' AND key = new.id;
        INSERT INTO search_docs (kind, key, at) SELECT 'photo', new.id, new.taken_at WHERE new.deleted = 0 AND new.hidden = 0;
        INSERT INTO search_text (rowid, title, body)
            SELECT id, coalesce(nullif(trim(new.title), ''), new.filename), concat_ws(' ', new.filename, new.caption, new.keywords, strftime('%Y', new.taken_at, 'unixepoch', 'localtime'), CASE strftime('%m', new.taken_at, 'unixepoch', 'localtime') WHEN '01' THEN 'January' WHEN '02' THEN 'February' WHEN '03' THEN 'March' WHEN '04' THEN 'April' WHEN '05' THEN 'May' WHEN '06' THEN 'June' WHEN '07' THEN 'July' WHEN '08' THEN 'August' WHEN '09' THEN 'September' WHEN '10' THEN 'October' WHEN '11' THEN 'November' ELSE 'December' END) FROM search_docs WHERE kind = 'photo' AND key = new.id;
    END;
    CREATE TRIGGER search_photos_update AFTER UPDATE OF title, filename, caption, keywords, taken_at, deleted, hidden ON photos BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'photo' AND key = old.id);
        DELETE FROM search_docs WHERE kind = 'photo' AND key = old.id;
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'photo' AND key = new.id);
        DELETE FROM search_docs WHERE kind = 'photo' AND key = new.id;
        INSERT INTO search_docs (kind, key, at) SELECT 'photo', new.id, new.taken_at WHERE new.deleted = 0 AND new.hidden = 0;
        INSERT INTO search_text (rowid, title, body)
            SELECT id, coalesce(nullif(trim(new.title), ''), new.filename), concat_ws(' ', new.filename, new.caption, new.keywords, strftime('%Y', new.taken_at, 'unixepoch', 'localtime'), CASE strftime('%m', new.taken_at, 'unixepoch', 'localtime') WHEN '01' THEN 'January' WHEN '02' THEN 'February' WHEN '03' THEN 'March' WHEN '04' THEN 'April' WHEN '05' THEN 'May' WHEN '06' THEN 'June' WHEN '07' THEN 'July' WHEN '08' THEN 'August' WHEN '09' THEN 'September' WHEN '10' THEN 'October' WHEN '11' THEN 'November' ELSE 'December' END) FROM search_docs WHERE kind = 'photo' AND key = new.id;
    END;
    CREATE TRIGGER search_photos_delete AFTER DELETE ON photos BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'photo' AND key = old.id);
        DELETE FROM search_docs WHERE kind = 'photo' AND key = old.id;
    END;
    INSERT INTO search_docs (kind, key, at) SELECT 'photo', id, taken_at FROM photos WHERE deleted = 0 AND hidden = 0;
    INSERT INTO search_text (rowid, title, body)
        SELECT d.id, coalesce(nullif(trim(X.title), ''), X.filename), concat_ws(' ', X.filename, X.caption, X.keywords, strftime('%Y', X.taken_at, 'unixepoch', 'localtime'), CASE strftime('%m', X.taken_at, 'unixepoch', 'localtime') WHEN '01' THEN 'January' WHEN '02' THEN 'February' WHEN '03' THEN 'March' WHEN '04' THEN 'April' WHEN '05' THEN 'May' WHEN '06' THEN 'June' WHEN '07' THEN 'July' WHEN '08' THEN 'August' WHEN '09' THEN 'September' WHEN '10' THEN 'October' WHEN '11' THEN 'November' ELSE 'December' END) FROM search_docs d JOIN photos X ON d.kind = 'photo' AND d.key = X.id;
    CREATE TRIGGER search_contacts_insert AFTER INSERT ON contacts BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'contact' AND key = new.guid);
        DELETE FROM search_docs WHERE kind = 'contact' AND key = new.guid;
        INSERT INTO search_docs (kind, key, sort) SELECT 'contact', guid, sort_name FROM contacts WHERE guid = new.guid;
        INSERT INTO search_text (rowid, title, body)
            SELECT d.id, concat_ws(' ', c.first_name, c.middle_name, c.last_name), concat_ws(' ', c.company, (SELECT group_concat(m.value || CASE WHEN m.kind = 'phone' THEN ' ' || replace(replace(replace(replace(replace(replace(replace(m.value, ' ', ''), '-', ''), '(', ''), ')', ''), '+', ''), '.', ''), '/', '') || ' ' || substr(replace(replace(replace(replace(replace(replace(replace(m.value, ' ', ''), '-', ''), '(', ''), ')', ''), '+', ''), '.', ''), '/', ''), -10) ELSE '' END, ' ')
                                            FROM contact_methods m WHERE m.contact_guid = c.guid)) FROM search_docs d JOIN contacts c ON c.guid = d.key
            WHERE d.kind = 'contact' AND d.key = new.guid;
    END;
    CREATE TRIGGER search_contacts_update AFTER UPDATE ON contacts BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'contact' AND key = old.guid);
        DELETE FROM search_docs WHERE kind = 'contact' AND key = old.guid;
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'contact' AND key = new.guid);
        DELETE FROM search_docs WHERE kind = 'contact' AND key = new.guid;
        INSERT INTO search_docs (kind, key, sort) SELECT 'contact', guid, sort_name FROM contacts WHERE guid = new.guid;
        INSERT INTO search_text (rowid, title, body)
            SELECT d.id, concat_ws(' ', c.first_name, c.middle_name, c.last_name), concat_ws(' ', c.company, (SELECT group_concat(m.value || CASE WHEN m.kind = 'phone' THEN ' ' || replace(replace(replace(replace(replace(replace(replace(m.value, ' ', ''), '-', ''), '(', ''), ')', ''), '+', ''), '.', ''), '/', '') || ' ' || substr(replace(replace(replace(replace(replace(replace(replace(m.value, ' ', ''), '-', ''), '(', ''), ')', ''), '+', ''), '.', ''), '/', ''), -10) ELSE '' END, ' ')
                                            FROM contact_methods m WHERE m.contact_guid = c.guid)) FROM search_docs d JOIN contacts c ON c.guid = d.key
            WHERE d.kind = 'contact' AND d.key = new.guid;
    END;
    CREATE TRIGGER search_contacts_delete AFTER DELETE ON contacts BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'contact' AND key = old.guid);
        DELETE FROM search_docs WHERE kind = 'contact' AND key = old.guid;
    END;
    CREATE TRIGGER search_contact_methods_insert AFTER INSERT ON contact_methods BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'contact' AND key = new.contact_guid);
        DELETE FROM search_docs WHERE kind = 'contact' AND key = new.contact_guid;
        INSERT INTO search_docs (kind, key, sort) SELECT 'contact', guid, sort_name FROM contacts WHERE guid = new.contact_guid;
        INSERT INTO search_text (rowid, title, body)
            SELECT d.id, concat_ws(' ', c.first_name, c.middle_name, c.last_name), concat_ws(' ', c.company, (SELECT group_concat(m.value || CASE WHEN m.kind = 'phone' THEN ' ' || replace(replace(replace(replace(replace(replace(replace(m.value, ' ', ''), '-', ''), '(', ''), ')', ''), '+', ''), '.', ''), '/', '') || ' ' || substr(replace(replace(replace(replace(replace(replace(replace(m.value, ' ', ''), '-', ''), '(', ''), ')', ''), '+', ''), '.', ''), '/', ''), -10) ELSE '' END, ' ')
                                            FROM contact_methods m WHERE m.contact_guid = c.guid)) FROM search_docs d JOIN contacts c ON c.guid = d.key
            WHERE d.kind = 'contact' AND d.key = new.contact_guid;
    END;
    CREATE TRIGGER search_contact_methods_update AFTER UPDATE ON contact_methods BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'contact' AND key = old.contact_guid);
        DELETE FROM search_docs WHERE kind = 'contact' AND key = old.contact_guid;
        INSERT INTO search_docs (kind, key, sort) SELECT 'contact', guid, sort_name FROM contacts WHERE guid = old.contact_guid;
        INSERT INTO search_text (rowid, title, body)
            SELECT d.id, concat_ws(' ', c.first_name, c.middle_name, c.last_name), concat_ws(' ', c.company, (SELECT group_concat(m.value || CASE WHEN m.kind = 'phone' THEN ' ' || replace(replace(replace(replace(replace(replace(replace(m.value, ' ', ''), '-', ''), '(', ''), ')', ''), '+', ''), '.', ''), '/', '') || ' ' || substr(replace(replace(replace(replace(replace(replace(replace(m.value, ' ', ''), '-', ''), '(', ''), ')', ''), '+', ''), '.', ''), '/', ''), -10) ELSE '' END, ' ')
                                            FROM contact_methods m WHERE m.contact_guid = c.guid)) FROM search_docs d JOIN contacts c ON c.guid = d.key
            WHERE d.kind = 'contact' AND d.key = old.contact_guid;
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'contact' AND key = new.contact_guid);
        DELETE FROM search_docs WHERE kind = 'contact' AND key = new.contact_guid;
        INSERT INTO search_docs (kind, key, sort) SELECT 'contact', guid, sort_name FROM contacts WHERE guid = new.contact_guid;
        INSERT INTO search_text (rowid, title, body)
            SELECT d.id, concat_ws(' ', c.first_name, c.middle_name, c.last_name), concat_ws(' ', c.company, (SELECT group_concat(m.value || CASE WHEN m.kind = 'phone' THEN ' ' || replace(replace(replace(replace(replace(replace(replace(m.value, ' ', ''), '-', ''), '(', ''), ')', ''), '+', ''), '.', ''), '/', '') || ' ' || substr(replace(replace(replace(replace(replace(replace(replace(m.value, ' ', ''), '-', ''), '(', ''), ')', ''), '+', ''), '.', ''), '/', ''), -10) ELSE '' END, ' ')
                                            FROM contact_methods m WHERE m.contact_guid = c.guid)) FROM search_docs d JOIN contacts c ON c.guid = d.key
            WHERE d.kind = 'contact' AND d.key = new.contact_guid;
    END;
    CREATE TRIGGER search_contact_methods_delete AFTER DELETE ON contact_methods BEGIN
        DELETE FROM search_text WHERE rowid = (SELECT id FROM search_docs WHERE kind = 'contact' AND key = old.contact_guid);
        DELETE FROM search_docs WHERE kind = 'contact' AND key = old.contact_guid;
        INSERT INTO search_docs (kind, key, sort) SELECT 'contact', guid, sort_name FROM contacts WHERE guid = old.contact_guid;
        INSERT INTO search_text (rowid, title, body)
            SELECT d.id, concat_ws(' ', c.first_name, c.middle_name, c.last_name), concat_ws(' ', c.company, (SELECT group_concat(m.value || CASE WHEN m.kind = 'phone' THEN ' ' || replace(replace(replace(replace(replace(replace(replace(m.value, ' ', ''), '-', ''), '(', ''), ')', ''), '+', ''), '.', ''), '/', '') || ' ' || substr(replace(replace(replace(replace(replace(replace(replace(m.value, ' ', ''), '-', ''), '(', ''), ')', ''), '+', ''), '.', ''), '/', ''), -10) ELSE '' END, ' ')
                                            FROM contact_methods m WHERE m.contact_guid = c.guid)) FROM search_docs d JOIN contacts c ON c.guid = d.key
            WHERE d.kind = 'contact' AND d.key = old.contact_guid;
    END;
    INSERT INTO search_docs (kind, key, sort) SELECT 'contact', guid, sort_name FROM contacts;
    INSERT INTO search_text (rowid, title, body)
        SELECT d.id, concat_ws(' ', c.first_name, c.middle_name, c.last_name), concat_ws(' ', c.company, (SELECT group_concat(m.value || CASE WHEN m.kind = 'phone' THEN ' ' || replace(replace(replace(replace(replace(replace(replace(m.value, ' ', ''), '-', ''), '(', ''), ')', ''), '+', ''), '.', ''), '/', '') || ' ' || substr(replace(replace(replace(replace(replace(replace(replace(m.value, ' ', ''), '-', ''), '(', ''), ')', ''), '+', ''), '.', ''), '/', ''), -10) ELSE '' END, ' ')
                                            FROM contact_methods m WHERE m.contact_guid = c.guid)) FROM search_docs d JOIN contacts c ON d.kind = 'contact' AND d.key = c.guid;
    "#,
    // 46 — the rest of a contact's name and work, as Apple's card holds them: prefix, suffix,
    // nickname, the phonetic spellings, job title and department. The book is read whole once
    // so every card has them.
    r#"
    ALTER TABLE contacts ADD COLUMN prefix TEXT;
    ALTER TABLE contacts ADD COLUMN suffix TEXT;
    ALTER TABLE contacts ADD COLUMN nickname TEXT;
    ALTER TABLE contacts ADD COLUMN phonetic_first_name TEXT;
    ALTER TABLE contacts ADD COLUMN phonetic_last_name TEXT;
    ALTER TABLE contacts ADD COLUMN phonetic_company TEXT;
    ALTER TABLE contacts ADD COLUMN job_title TEXT;
    ALTER TABLE contacts ADD COLUMN department TEXT;
    UPDATE sync_state SET token = NULL WHERE service = 'contacts';
    "#,
    // 47 — the records Apple keeps beside a reminder that say how it repeats and where it
    // rings (RecurrenceRule, Alarm, AlarmTrigger), each kept whole as Apple sent it, so the
    // phone's whole schedule — its end date, a place, arriving or leaving — can be shown. The
    // zone is walked again once so every library has them.
    r#"
    CREATE TABLE reminder_parts (
        name          TEXT PRIMARY KEY,
        reminder_guid TEXT,
        kind          TEXT NOT NULL,
        raw           TEXT NOT NULL
    ) STRICT;
    CREATE INDEX reminder_parts_by_reminder ON reminder_parts (reminder_guid);
    UPDATE sync_state SET token = NULL WHERE service = 'reminders';
    "#,
    // 48 — a reminder's alarms found by the reminder: each row's marks and details joined
    // reminder_alarms on reminder_guid with no index, a whole-table scan per reminder, under
    // the library lock every list open — ticks and edits queued behind it.
    r#"
    CREATE INDEX IF NOT EXISTS reminder_alarms_by_reminder ON reminder_alarms (reminder_guid);
    "#,
    // 49 — all-day and floating events kept as the day and the time on the clock, not as an
    // instant. Each was turned into an instant in this computer's zone as it was stored, and
    // nothing turned it back when the zone changed: flown from London to New York, every
    // birthday and holiday drew on the day before until the next full pass, and an edit made
    // meanwhile wrote that wrong day back to Apple. `floating` rows hold their wall-clock
    // time counted as if in UTC, read in whatever zone the computer is in when shown; the
    // rows already stored are turned over by `float_events`, in the same transaction.
    r#"
    ALTER TABLE events ADD COLUMN floating INTEGER NOT NULL DEFAULT 0;
    "#,
    // 50 — a large library at the speed of a small one. A pass that stores rows unchanged
    // rewrites nothing the search index holds (`fts_when`, run with this migration, makes each
    // index trigger fire only on a real change); Recents, Favorites and Videos are read in
    // order from indexes of their own, as the Library is, each ending in the photo's id so a
    // page carries on from the last photo shown (keyset) instead of counting past the ones
    // before it; and how many photos each view holds is kept as the rows change, so the
    // sidebar's counts never count the library.
    r#"
    DROP INDEX IF EXISTS photos_by_taken;
    CREATE INDEX photos_by_taken ON photos (deleted, hidden, taken_at DESC, id);
    CREATE INDEX photos_by_added ON photos (deleted, hidden, COALESCE(added_at, taken_at) DESC, id);
    CREATE INDEX photos_favorites ON photos (deleted, hidden, taken_at DESC, id) WHERE favorite = 1;
    CREATE INDEX photos_videos ON photos (deleted, hidden, taken_at DESC, id) WHERE kind = 'video';
    CREATE INDEX photos_months ON photos (deleted, hidden, taken_at, id, kind, favorite, burst, burst_pick);
    CREATE TABLE photo_view_counts (
        view TEXT PRIMARY KEY,
        n    INTEGER NOT NULL
    ) STRICT;
    INSERT INTO photo_view_counts (view, n) SELECT 'library', COUNT(*) FROM photos WHERE deleted = 0 AND hidden = 0;
    INSERT INTO photo_view_counts (view, n) SELECT 'favorites', COUNT(*) FROM photos WHERE deleted = 0 AND hidden = 0 AND favorite = 1;
    INSERT INTO photo_view_counts (view, n) SELECT 'videos', COUNT(*) FROM photos WHERE deleted = 0 AND hidden = 0 AND kind = 'video';
    INSERT INTO photo_view_counts (view, n) SELECT 'screenshots', COUNT(*) FROM photos WHERE deleted = 0 AND hidden = 0 AND subtype = 3;
    INSERT INTO photo_view_counts (view, n) SELECT 'panoramas', COUNT(*) FROM photos WHERE deleted = 0 AND hidden = 0 AND subtype = 1;
    INSERT INTO photo_view_counts (view, n) SELECT 'portrait', COUNT(*) FROM photos WHERE deleted = 0 AND hidden = 0 AND rendered = 8;
    INSERT INTO photo_view_counts (view, n) SELECT 'hidden', COUNT(*) FROM photos WHERE deleted = 0 AND hidden = 1;
    INSERT INTO photo_view_counts (view, n) SELECT 'rated', COUNT(*) FROM photos WHERE deleted = 0 AND hidden = 0 AND rating > 0;
    INSERT INTO photo_view_counts (view, n) SELECT 'recentlyDeleted', COUNT(*) FROM photos WHERE deleted = 1;
    CREATE TRIGGER photo_counts_insert AFTER INSERT ON photos BEGIN
        UPDATE photo_view_counts SET n = n + (CASE view WHEN 'library' THEN (new.deleted = 0 AND new.hidden = 0) WHEN 'favorites' THEN (new.deleted = 0 AND new.hidden = 0 AND new.favorite = 1) WHEN 'videos' THEN (new.deleted = 0 AND new.hidden = 0 AND new.kind = 'video') WHEN 'screenshots' THEN (new.deleted = 0 AND new.hidden = 0 AND new.subtype = 3) WHEN 'panoramas' THEN (new.deleted = 0 AND new.hidden = 0 AND new.subtype = 1) WHEN 'portrait' THEN (new.deleted = 0 AND new.hidden = 0 AND new.rendered = 8) WHEN 'hidden' THEN (new.deleted = 0 AND new.hidden = 1) WHEN 'rated' THEN (new.deleted = 0 AND new.hidden = 0 AND new.rating > 0) WHEN 'recentlyDeleted' THEN (new.deleted = 1) ELSE 0 END);
    END;
    CREATE TRIGGER photo_counts_delete AFTER DELETE ON photos BEGIN
        UPDATE photo_view_counts SET n = n - (CASE view WHEN 'library' THEN (old.deleted = 0 AND old.hidden = 0) WHEN 'favorites' THEN (old.deleted = 0 AND old.hidden = 0 AND old.favorite = 1) WHEN 'videos' THEN (old.deleted = 0 AND old.hidden = 0 AND old.kind = 'video') WHEN 'screenshots' THEN (old.deleted = 0 AND old.hidden = 0 AND old.subtype = 3) WHEN 'panoramas' THEN (old.deleted = 0 AND old.hidden = 0 AND old.subtype = 1) WHEN 'portrait' THEN (old.deleted = 0 AND old.hidden = 0 AND old.rendered = 8) WHEN 'hidden' THEN (old.deleted = 0 AND old.hidden = 1) WHEN 'rated' THEN (old.deleted = 0 AND old.hidden = 0 AND old.rating > 0) WHEN 'recentlyDeleted' THEN (old.deleted = 1) ELSE 0 END);
    END;
    CREATE TRIGGER photo_counts_update AFTER UPDATE OF deleted, hidden, favorite, kind, subtype, rendered, rating ON photos
    WHEN (old.deleted, old.hidden, old.favorite, old.kind, old.subtype, old.rendered, old.rating)
         IS NOT (new.deleted, new.hidden, new.favorite, new.kind, new.subtype, new.rendered, new.rating)
    BEGIN
        UPDATE photo_view_counts SET n = n - (CASE view WHEN 'library' THEN (old.deleted = 0 AND old.hidden = 0) WHEN 'favorites' THEN (old.deleted = 0 AND old.hidden = 0 AND old.favorite = 1) WHEN 'videos' THEN (old.deleted = 0 AND old.hidden = 0 AND old.kind = 'video') WHEN 'screenshots' THEN (old.deleted = 0 AND old.hidden = 0 AND old.subtype = 3) WHEN 'panoramas' THEN (old.deleted = 0 AND old.hidden = 0 AND old.subtype = 1) WHEN 'portrait' THEN (old.deleted = 0 AND old.hidden = 0 AND old.rendered = 8) WHEN 'hidden' THEN (old.deleted = 0 AND old.hidden = 1) WHEN 'rated' THEN (old.deleted = 0 AND old.hidden = 0 AND old.rating > 0) WHEN 'recentlyDeleted' THEN (old.deleted = 1) ELSE 0 END) + (CASE view WHEN 'library' THEN (new.deleted = 0 AND new.hidden = 0) WHEN 'favorites' THEN (new.deleted = 0 AND new.hidden = 0 AND new.favorite = 1) WHEN 'videos' THEN (new.deleted = 0 AND new.hidden = 0 AND new.kind = 'video') WHEN 'screenshots' THEN (new.deleted = 0 AND new.hidden = 0 AND new.subtype = 3) WHEN 'panoramas' THEN (new.deleted = 0 AND new.hidden = 0 AND new.subtype = 1) WHEN 'portrait' THEN (new.deleted = 0 AND new.hidden = 0 AND new.rendered = 8) WHEN 'hidden' THEN (new.deleted = 0 AND new.hidden = 1) WHEN 'rated' THEN (new.deleted = 0 AND new.hidden = 0 AND new.rating > 0) WHEN 'recentlyDeleted' THEN (new.deleted = 1) ELSE 0 END);
    END;
    "#,
    // 51 — Shared Albums as iOS 27 keeps them (shared collections): each album, where it lives
    // in Apple's databases and how far its change feed was read, and its photos — kept apart
    // from the library's own, so nothing shared ever shows in Library, its counts or search.
    // A photo is stored as the library describes one (JSON of `Photo`): an album is replaced
    // whole whenever it changes, so it never needs a column of its own.
    r#"
    CREATE TABLE shared_albums (
        zone       TEXT PRIMARY KEY,
        database   TEXT NOT NULL,
        owner      TEXT,
        title      TEXT,
        expires_at INTEGER,
        token      TEXT
    ) STRICT;
    CREATE TABLE shared_album_photos (
        zone     TEXT NOT NULL REFERENCES shared_albums (zone) ON DELETE CASCADE,
        id       TEXT NOT NULL,
        taken_at INTEGER NOT NULL,
        photo    TEXT NOT NULL,
        PRIMARY KEY (zone, id)
    ) STRICT;
    CREATE INDEX shared_album_photos_by_taken ON shared_album_photos (zone, taken_at DESC, id);
    CREATE INDEX shared_album_photos_by_id ON shared_album_photos (id);
    "#,
    // 52 — the Shared Library (iOS 16+): a second photo library, shared with family, in a
    // `SharedSync-<id>` zone beside the personal one. Its photos are the library's own rows,
    // each marked with the zone it lives in (none: the personal library, so no existing row is
    // rewritten); where each Shared Library is in Apple's databases is kept beside them. A whole
    // read is noted per zone, so finishing one library's read lets go only of that library's
    // photos it never saw, never the other's.
    r#"
    ALTER TABLE photos ADD COLUMN zone TEXT;
    CREATE INDEX photos_by_zone ON photos (zone) WHERE zone IS NOT NULL;
    CREATE TABLE shared_libraries (
        zone     TEXT PRIMARY KEY,
        database TEXT NOT NULL,
        owner    TEXT,
        title    TEXT
    ) STRICT;
    CREATE TABLE IF NOT EXISTS photo_walk (
        kind TEXT NOT NULL,
        id   TEXT NOT NULL,
        PRIMARY KEY (kind, id)
    ) STRICT, WITHOUT ROWID;
    CREATE TABLE photo_walk_by_zone (
        zone TEXT NOT NULL DEFAULT '',
        kind TEXT NOT NULL,
        id   TEXT NOT NULL,
        PRIMARY KEY (zone, kind, id)
    ) STRICT, WITHOUT ROWID;
    INSERT INTO photo_walk_by_zone (kind, id) SELECT kind, id FROM photo_walk;
    DROP TABLE photo_walk;
    ALTER TABLE photo_walk_by_zone RENAME TO photo_walk;
    CREATE TABLE IF NOT EXISTS photo_walk_held (
        id     TEXT PRIMARY KEY,
        record TEXT NOT NULL
    ) STRICT;
    CREATE TABLE photo_walk_held_by_zone (
        zone   TEXT NOT NULL DEFAULT '',
        id     TEXT NOT NULL,
        record TEXT NOT NULL,
        UNIQUE (zone, id)
    ) STRICT;
    INSERT INTO photo_walk_held_by_zone (id, record) SELECT id, record FROM photo_walk_held ORDER BY rowid;
    DROP TABLE photo_walk_held;
    ALTER TABLE photo_walk_held_by_zone RENAME TO photo_walk_held;
    "#,
    // 53 — Backup: every file copied into the backup folder, where, and how large: a photo's
    // original, its Live Photo video, later its edited version; later iCloud Drive's files too.
    // One row per file (`source` photos or drive, `item` its id there, `part` which file of it),
    // so each later phase adds rows, never a table. A file is backed up exactly when its row
    // names a file of that size there (backup.rs checks), so a backup folder moved, emptied or
    // replaced is noticed and filled again. `version`: what it was copied from (Drive's etag),
    // so a file changed in iCloud is copied again.
    r#"
    CREATE TABLE backup_files (
        source  TEXT NOT NULL,
        item    TEXT NOT NULL,
        part    TEXT NOT NULL,
        path    TEXT NOT NULL,
        size    INTEGER NOT NULL,
        version TEXT,
        PRIMARY KEY (source, item, part)
    ) STRICT, WITHOUT ROWID;
    "#,
];

/// Migrations that need more than SQL, run in the same transaction as the one numbered.
pub const FIXUPS: &[crate::Fixup] = &[(49, float_events), (50, fts_when)];

/// The search index's update triggers, and the columns whose change is a change to what they
/// index. Each fires on `UPDATE OF` those columns — which SQLite counts whenever an upsert
/// names them, changed or not: a pass storing fifty thousand notes as they were rewrote every
/// one's words (27 s, measured).
const FTS_UPDATES: &[(&str, &[&str])] = &[
    (
        "search_notes_update",
        &["folder_guid", "title", "body", "modified_at"],
    ),
    (
        "search_reminders_update",
        &["list_guid", "title", "notes", "due_at", "completed_at"],
    ),
    (
        "search_events_update",
        &["title", "location", "notes", "starts_at"],
    ),
    (
        "search_drive_items_update",
        &["name", "extension", "modified_at"],
    ),
    (
        "search_photos_update",
        &[
            "title", "filename", "caption", "keywords", "taken_at", "deleted", "hidden",
        ],
    ),
    (
        "search_contacts_update",
        &[
            "first_name",
            "middle_name",
            "last_name",
            "company",
            "sort_name",
        ],
    ),
    (
        "search_contact_methods_update",
        &["contact_guid", "kind", "value"],
    ),
];

/// Each search index update trigger made again exactly as it stands, with a `WHEN` that lets
/// it fire only when a column it indexes really changed.
fn fts_when(tx: &rusqlite::Transaction<'_>) -> crate::Result<()> {
    for (name, columns) in FTS_UPDATES {
        let sql: String = tx.query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
            [name],
            |r| r.get(0),
        )?;
        let olds = columns
            .iter()
            .map(|c| format!("old.{c}"))
            .collect::<Vec<_>>();
        let news = columns
            .iter()
            .map(|c| format!("new.{c}"))
            .collect::<Vec<_>>();
        let when = format!(
            " WHEN ({}) IS NOT ({}) BEGIN",
            olds.join(", "),
            news.join(", ")
        );
        let at = sql.find(" BEGIN").ok_or(rusqlite::Error::InvalidQuery)?;
        let made = format!("{}{}{}", &sql[..at], when, &sql[at + " BEGIN".len()..]);
        tx.execute_batch(&format!("DROP TRIGGER {name};\n{made};"))?;
    }
    Ok(())
}

/// The all-day and floating events already stored, from instants to wall-clock time: read in
/// the zone they were stored in — this computer's, by name, or UTC where it had none, as the
/// sync did — so nothing moves by being turned over.
fn float_events(tx: &rusqlite::Transaction<'_>) -> crate::Result<()> {
    let system = jiff::tz::TimeZone::system();
    let zone = if system.iana_name().is_some() {
        system
    } else {
        jiff::tz::TimeZone::UTC
    };
    let rows: Vec<(String, i64, i64)> = {
        let mut stmt = tx.prepare(
            "SELECT guid, starts_at, ends_at FROM events
              WHERE floating = 0 AND (all_day != 0 OR timezone IS NULL)",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut update =
        tx.prepare("UPDATE events SET starts_at = ?2, ends_at = ?3, floating = 1 WHERE guid = ?1")?;
    for (guid, starts, ends) in rows {
        update.execute(rusqlite::params![
            guid,
            crate::calendar::wall_clock(starts, &zone),
            crate::calendar::wall_clock(ends, &zone),
        ])?;
    }
    Ok(())
}
