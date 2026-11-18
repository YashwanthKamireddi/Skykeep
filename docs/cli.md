# Skykeep from other programs

Launchers, status bars, widgets and scripts can read and add to your Reminders, Calendar, Notes
and Contacts through the `skykeep` command. These are the same paths Skykeep's own windows use,
so whatever you add reaches iCloud the way an edit made in the app does.

The command is `skykeep` (Linux packages, Snap), `flatpak run io.github.yashwanthkamireddi.Skykeep`
(Flatpak) or `skykeep.exe` in the install folder (Windows).

## Reading

A read command answers from what Skykeep has already synced to this computer. It opens no window,
doesn't start Skykeep, doesn't use the network and changes nothing. A read usually takes well under
a tenth of a second. The read command must be the first argument.

| Command | Answers |
|---|---|
| `--reminders-today` | Reminders due today or overdue, as in Reminders' Today list |
| `--reminders <list id>` | A list's reminders that aren't done |
| `--reminder-lists` | Your reminder lists in use |
| `--events [--days N]` | Events from now for N days (7 unless given, up to 366), including one already running |
| `--calendars` | Your calendars, and which ones take new events |
| `--search-notes <words>` | Notes holding every word, as Notes' search finds them (Recently Deleted left out) |
| `--note-folders` | Your Notes folders |
| `--search-contacts <words>` | Contacts matching the words, as Contacts' search finds them |

The answer is JSON on stdout. Times are seconds since 1970 (UTC).

```sh
$ skykeep --reminders-today
[
  {
    "id": "6F1C…",
    "list": "A0B2…",
    "title": "Call the bank",
    "notes": null,
    "due": 1795000000,
    "allDay": false,
    "flagged": true,
    "priority": 0,
    "parent": null
  }
]
```

| Command | Fields of each item |
|---|---|
| `--reminders-today`, `--reminders` | `id`, `list`, `title`, `notes`, `due`, `allDay`, `flagged`, `priority` (Apple's: 0 none, 1 high, 5 medium, 9 low), `parent` (the reminder this is a subtask of) |
| `--reminder-lists` | `id`, `title` |
| `--events` | `id`, `calendar`, `title`, `location`, `start`, `end`, `allDay`, `timezone` |
| `--calendars` | `id`, `title`, `writable` (takes new events), `default` (where `--create-event` puts one) |
| `--search-notes` | `id`, `folder`, `title`, `snippet` (`null` for a locked note), `locked`, `pinned`, `modified` |
| `--note-folders` | `id`, `title`, `parent` |
| `--search-contacts` | `id`, `name`, `company`, `phones`, `emails` (each `{ "label", "value" }`) |

A field that has no value is `null`. Field names don't change between versions. New fields may
be added, so ignore any you don't know.

## Adding and changing

| Command | Does |
|---|---|
| `--create-reminder <title> [--list <id>] [--due <time>] [--all-day] [--flagged]` | Adds a reminder. Without `--list`, it goes in the list you last added to. `--all-day` needs `--due`. |
| `--create-event <title> --start <time> [--end <time>] [--calendar <id>] [--all-day]` | Adds an event. It lasts an hour unless `--end` is given. Without `--calendar`, it goes in your default calendar. An all-day event covers each day from the start to the end. |
| `--create-note <text> [--folder <id>]` | Adds a note. Its first line is the title, as in Notes. Without `--folder`, it goes in Notes. |
| `--complete-reminder <id>`, `--uncomplete-reminder <id>` | Ticks a reminder off, or puts it back. |

Each of these also takes the `--flag=value` form, which lets a title or text start with `--`
(`--create-note=--draft`). Times are seconds since 1970, for example `date -d 'tomorrow 9:00' +%s`.

If Skykeep isn't running, it starts in the background without a window. It adds the item and
stays running so the change reaches iCloud. When that launch was the one that added the item, it
prints the new item's id on stdout. When Skykeep was already running, the item is handed to that
copy and nothing is printed. To find the item, use a read command.

## Opening Skykeep

| Command | Opens |
|---|---|
| `--app <notes, reminders, calendar, contacts, drive or photos>` | That app's window |
| `--new <note, reminder or event>` | A new note or reminder in the quick capture, or a new event in Calendar |
| `--reminder <id>` | Reminders, showing that reminder |

## Exit codes and errors

| Code | Means |
|---|---|
| 0 | Done |
| 1 | Asked wrongly (an unknown id, a missing title, a time that isn't a number), or the library couldn't be read |
| 2 | No library on this computer yet: open Skykeep and sign in first |

The reason is on stderr, starting with `skykeep:`. A refused command changes nothing.

A create asked wrongly (no title, a time that isn't a number) is refused with exit 1 before
Skykeep starts. Some problems can only be found in your library, such as a list id that doesn't
exist. If Skykeep was already running, that refusal shows only in the running copy's log, so
check the result with a read command.
