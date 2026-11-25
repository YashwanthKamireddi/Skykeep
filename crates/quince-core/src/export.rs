//! Whole-library reads for Export Everything: every row of a kind, unfiltered and unlimited.
//!
//! The screens' own reads are shaped for screens — a thousand at most, enabled calendars only,
//! no note bodies in a list. An export wants the opposite: everything the library holds, once.

use crate::calendar::Event;
use crate::drive::DriveItem;
use crate::notes::Note;
use crate::reminders::Reminder;
use crate::{Library, Result};

/// A photo as an export needs it: where its picture is kept, when it was taken, its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportPhoto {
    pub id: String,
    pub taken_at: i64,
    pub filename: Option<String>,
    /// `image`, `video` …, as the library stores it.
    pub kind: String,
}

impl Library {
    /// Every note, Recently Deleted too, with its whole body and its runs (JSON), oldest first.
    pub fn export_notes(&self) -> Result<Vec<Note>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, folder_guid, title, snippet, body, locked, pinned,
                    created_at, modified_at, editable, etag, runs
             FROM notes ORDER BY created_at, guid",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Note {
                guid: row.get(0)?,
                folder_guid: row.get(1)?,
                title: row.get(2)?,
                snippet: row.get(3)?,
                body: row.get(4)?,
                locked: row.get::<_, i64>(5)? != 0,
                pinned: row.get::<_, i64>(6)? != 0,
                created_at: row.get(7)?,
                modified_at: row.get(8)?,
                editable: row.get::<_, i64>(9)? != 0,
                etag: row.get(10)?,
                runs: row.get(11)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Every reminder in every list, done ones too, without Apple's raw record.
    pub fn export_reminders(&self) -> Result<Vec<Reminder>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, list_guid, title, notes, due_at, completed_at, priority, flagged,
                    parent_guid, recur_frequency, recur_interval, etag, timezone, section_guid,
                    all_day
             FROM reminders ORDER BY list_guid, completed_at IS NOT NULL, due_at, title, guid",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Reminder {
                guid: row.get(0)?,
                list_guid: row.get(1)?,
                title: row.get(2)?,
                notes: row.get(3)?,
                due_at: row.get(4)?,
                completed_at: row.get(5)?,
                priority: row.get(6)?,
                flagged: row.get::<_, i64>(7)? != 0,
                parent_guid: row.get(8)?,
                recur_frequency: row.get(9)?,
                recur_interval: row.get(10)?,
                etag: row.get(11)?,
                raw: None,
                timezone: row.get(12)?,
                section_guid: row.get(13)?,
                all_day: row.get::<_, i64>(14)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Every event the library holds, in every calendar, shown or hidden, by start.
    pub fn export_events(&self) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} FROM events ORDER BY calendar_guid, starts_at, guid",
            crate::calendar::EVENT_COLUMNS
        ))?;
        let zone = self.local_zone();
        let rows = stmt.query_map([], |row| Self::event_from_row(row, &zone))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Every Drive item the library has listed, in every folder it has read.
    pub fn export_drive_items(&self) -> Result<Vec<DriveItem>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} FROM drive_items ORDER BY drivewsid",
            crate::drive::ITEM_COLUMNS
        ))?;
        let rows = stmt.query_map([], crate::drive::item_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Every photo and video not in Recently Deleted — hidden ones and every shot of a burst
    /// included — oldest first.
    pub fn export_photos(&self) -> Result<Vec<ExportPhoto>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, taken_at, filename, kind FROM photos
             WHERE deleted = 0 ORDER BY taken_at, id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ExportPhoto {
                id: row.get(0)?,
                taken_at: row.get(1)?,
                filename: row.get(2)?,
                kind: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reminders::ReminderList;

    #[test]
    fn reads_everything_past_the_screens_limits() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_lists(&[ReminderList {
            guid: "L".into(),
            title: "List".into(),
            enabled: true,
            ..Default::default()
        }])
        .unwrap();
        let many: Vec<Reminder> = (0..1_200)
            .map(|i| Reminder {
                guid: format!("R{i}"),
                list_guid: "L".into(),
                title: format!("r{i}"),
                completed_at: (i % 2 == 0).then_some(1),
                ..Default::default()
            })
            .collect();
        lib.put_reminders(&many).unwrap();
        assert_eq!(lib.export_reminders().unwrap().len(), 1_200);
        assert!(lib.export_notes().unwrap().is_empty());
        assert!(lib.export_events().unwrap().is_empty());
        assert!(lib.export_drive_items().unwrap().is_empty());
        assert!(lib.export_photos().unwrap().is_empty());
    }
}
