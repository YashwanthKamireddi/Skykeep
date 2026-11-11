//! Photos: one round of a pass stored (the pass itself, a few rounds, readiness, Shared Albums
//! and the Shared Library, is `pass::photos`), shared by the desktop and the phone.
//!
//! Favourite, Hide, Delete, Recover and Delete for good go through the queue as Notes' and
//! Reminders' changes do: recorded and shown at once (`record_flags`), laid back over every
//! pass until sent (`lay`, `replay`), and sent behind (`send`), so a change made offline, or one
//! the phone's system stopped mid-send, is never lost and never shown undone by a pass.

use crate::library_lock::LibraryLock;
use crate::queue::{json, transient, unreadable};
use quince_core::outbox::Pending;
use quince_core::photos::Photo;
use quince_core::Library;
use quince_icloud::photos::{self as p, Flag, Links};
use quince_icloud::shared_collections::Place;
use quince_icloud::Error;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// One round's records stored, and every flag change still on its way laid back over them, as
/// one change: killed between the two, a hidden photo showed again until the next pass. A round
/// of a pass from the start marks the library as having had one. Answers how many records it
/// held and their links.
pub fn apply(
    library: &mut Library,
    fetched: p::PhotosFetch,
    full: bool,
) -> Result<(usize, HashMap<String, Links>), Error> {
    let records = fetched.records.len();
    library.atomically(|library| {
        let (_, links) = p::apply(library, fetched)?;
        replay(library)?;
        if full {
            library.mark_full_sync(p::SERVICE)?;
        }
        Ok((records, links))
    })
}

/// One round of a Shared Library's own pass stored, with the flag changes still on its way laid
/// back over it, as [`apply`] stores the personal library's. Answers how many photos changed and
/// their links.
pub fn apply_shared_library(
    library: &mut Library,
    fetched: p::PhotosFetch,
) -> Result<(usize, HashMap<String, Links>), Error> {
    library.atomically(|library| {
        let applied = p::apply(library, fetched)?;
        replay(library)?;
        Ok(applied)
    })
}

// --- Flags: recorded and laid over the library as one transaction, sent behind.

/// The queue Photos' changes wait in: the service's own name, as the window knows it.
pub const PHOTOS: &str = p::SERVICE;

/// The most photos one queued change carries: as many as one request to Apple takes. A
/// selection of thousands is a few changes, each sent whole or kept whole, so a send cut off
/// part way is never more than one request to send again.
pub const AT_ONCE: usize = quince_icloud::cloudkit::MODIFY_AT_ONCE;

/// How long Apple keeps a photo in Recently Deleted: shown until Apple says its own date.
const IN_RECENTLY_DELETED: i64 = 30 * 86_400;

/// A photo's flags before a change: what a refusal puts back.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Was {
    pub favorite: bool,
    pub hidden: bool,
    pub deleted: bool,
    pub deleted_at: Option<i64>,
    pub expunge_at: Option<i64>,
}

impl Was {
    fn of(p: &Photo) -> Self {
        Self {
            favorite: p.favorite,
            hidden: p.hidden,
            deleted: p.deleted,
            deleted_at: p.deleted_at,
            expunge_at: p.expunge_at,
        }
    }

    /// Whether `flag` was set.
    fn value(&self, flag: Flag) -> bool {
        match flag {
            Flag::Favorite => self.favorite,
            Flag::Hidden => self.hidden,
            Flag::Deleted | Flag::Expunged => self.deleted,
        }
    }

    /// `flag` put back on `p` as it was; its other flags, which have changes of their own, left.
    fn put_back(&self, flag: Flag, p: &mut Photo) {
        match flag {
            Flag::Favorite => p.favorite = self.favorite,
            Flag::Hidden => p.hidden = self.hidden,
            Flag::Deleted => {
                p.deleted = self.deleted;
                p.deleted_at = self.deleted_at;
                p.expunge_at = self.expunge_at;
            }
            Flag::Expunged => {}
        }
    }
}

/// One photo in a flag change: its record, the version of it the change was made over (Apple
/// takes a change against it, or — moved on since — against the version it holds now), the
/// Shared Library it is in (none: the person's own), and its flags before.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Flagged {
    pub id: String,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub zone: Option<String>,
    #[serde(default)]
    pub was: Was,
}

/// A flag set or cleared on photos here (the kind names the flag), and when: a delete's thirty
/// days are counted from then until Apple says its own date.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FlagChange {
    pub on: bool,
    pub at: i64,
    pub photos: Vec<Flagged>,
}

/// The queue's name for a flag's changes.
pub fn kind_of(flag: Flag) -> &'static str {
    match flag {
        Flag::Favorite => "favorite",
        Flag::Hidden => "hidden",
        Flag::Deleted => "deleted",
        Flag::Expunged => "expunged",
    }
}

fn flag_of(kind: &str) -> Option<Flag> {
    Some(match kind {
        "favorite" => Flag::Favorite,
        "hidden" => Flag::Hidden,
        "deleted" => Flag::Deleted,
        "expunged" => Flag::Expunged,
        _ => return None,
    })
}

/// `flag` set to `on` on `p`, as a change made at `at` shows it.
fn set(flag: Flag, on: bool, at: i64, p: &mut Photo) {
    match flag {
        Flag::Favorite => p.favorite = on,
        Flag::Hidden => p.hidden = on,
        Flag::Deleted => {
            p.deleted = on;
            p.deleted_at = on.then_some(at);
            p.expunge_at = on.then_some(at + IN_RECENTLY_DELETED);
        }
        Flag::Expunged => {}
    }
}

/// Favorite, Hide, Delete (to Recently Deleted), Recover, or (from Recently Deleted only) delete
/// for good, on many photos, as Photos does it: recorded and shown at once, in one transaction,
/// and sent behind by the queue (`send`). Deleted for good, a photo leaves every view now but
/// is kept aside, whole, until Apple has it gone; refused, it comes back.
///
/// A change not yet sent is the last word on its photos: one that takes it back (Undo) takes it
/// off the queue, and nothing goes to Apple. Answers the photos as they now are; an error is
/// said to the person.
pub fn record_flags(
    library: &mut Library,
    ids: &[String],
    flag: Flag,
    on: bool,
) -> Result<Vec<Photo>, String> {
    let at = jiff::Timestamp::now().as_second();
    library
        .atomically(|l| {
            let mut seen = HashSet::new();
            let mut found = Vec::with_capacity(ids.len());
            for id in ids {
                if seen.insert(id.as_str()) {
                    if let Some(photo) = l.photo(id)? {
                        found.push(photo);
                    }
                }
            }
            // Deleting for good is only ever from Recently Deleted, and only ever on.
            if flag == Flag::Expunged && (!on || found.iter().any(|p| !p.deleted)) {
                return Ok(Err(
                    "Only photos in Recently Deleted can be deleted for good.".to_string(),
                ));
            }
            record_found(l, found, flag, on, at).map(Ok)
        })
        .map_err(|e: quince_core::Error| e.to_string())?
}

fn record_found(
    l: &mut Library,
    found: Vec<Photo>,
    flag: Flag,
    on: bool,
    at: i64,
) -> quince_core::Result<Vec<Photo>> {
    let kind = kind_of(flag);
    let ids: Vec<String> = found.iter().map(|p| p.id.clone()).collect();
    let mut zones = Vec::with_capacity(found.len());
    for p in &found {
        zones.push(l.photo_zone(&p.id)?);
    }
    let flagged = |p: &Photo, zone: Option<String>| Flagged {
        id: p.id.clone(),
        tag: p.change_tag.clone(),
        zone,
        was: Was::of(p),
    };
    if flag == Flag::Expunged {
        let photos = found
            .iter()
            .zip(zones)
            .map(|(p, z)| flagged(p, z))
            .collect();
        queue(l, kind, on, at, photos)?;
        l.set_photos_aside(&ids)?;
        return Ok(Vec::new());
    }
    // The same flag on these photos, waiting and never sent: taken back into this change, so
    // only the last word goes, sent from the version and flags the first was made over.
    let wanted: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let mut earlier: HashMap<String, Flagged> = HashMap::new();
    for w in l.outbox_pending(PHOTOS)? {
        if w.kind != kind || w.tries != 0 {
            continue;
        }
        let Ok(mut change) = serde_json::from_str::<FlagChange>(&w.payload) else {
            continue;
        };
        let (back, kept): (Vec<Flagged>, Vec<Flagged>) = change
            .photos
            .into_iter()
            .partition(|f| wanted.contains(f.id.as_str()));
        if back.is_empty() {
            continue;
        }
        let taken = if kept.is_empty() {
            l.outbox_withdraw(w.id)?
        } else {
            change.photos = kept;
            l.outbox_amend(w.id, &json(&change)?)?
        };
        if taken {
            for f in back {
                earlier.entry(f.id.clone()).or_insert(f);
            }
        }
    }
    let mut queued = Vec::new();
    let mut changed = Vec::new();
    for (mut photo, zone) in found.into_iter().zip(zones) {
        match earlier.remove(&photo.id) {
            // Back to what it was before the change taken back: nothing to send.
            Some(before) if before.was.value(flag) == on => {
                before.was.put_back(flag, &mut photo);
                changed.push(photo);
            }
            Some(before) => {
                set(flag, on, at, &mut photo);
                queued.push(before);
                changed.push(photo);
            }
            // Already so, here and in what waits to go: nothing to say to Apple.
            None if Was::of(&photo).value(flag) == on => {}
            None => {
                queued.push(flagged(&photo, zone));
                set(flag, on, at, &mut photo);
                changed.push(photo);
            }
        }
    }
    queue(l, kind, on, at, queued)?;
    l.put_photos(&changed)?;
    let mut now = Vec::with_capacity(ids.len());
    for id in &ids {
        if let Some(p) = l.photo(id)? {
            now.push(p);
        }
    }
    Ok(now)
}

/// `photos` queued as changes of [`AT_ONCE`] each, named by the first photo of each.
fn queue(
    l: &mut Library,
    kind: &str,
    on: bool,
    at: i64,
    photos: Vec<Flagged>,
) -> quince_core::Result<()> {
    for chunk in photos.chunks(AT_ONCE) {
        let change = FlagChange {
            on,
            at,
            photos: chunk.to_vec(),
        };
        l.outbox_append(PHOTOS, &chunk[0].id, kind, &json(&change)?)?;
    }
    Ok(())
}

/// One waiting flag change laid over the library.
pub fn lay(l: &mut Library, p: &Pending) -> quince_core::Result<()> {
    let Some(flag) = flag_of(&p.kind) else {
        return Ok(());
    };
    let Ok(change) = serde_json::from_str::<FlagChange>(&p.payload) else {
        return Ok(());
    };
    if flag == Flag::Expunged {
        let ids: Vec<String> = change.photos.iter().map(|f| f.id.clone()).collect();
        return l.set_photos_aside(&ids);
    }
    let mut changed = Vec::with_capacity(change.photos.len());
    for f in &change.photos {
        if let Some(mut photo) = l.photo(&f.id)? {
            set(flag, change.on, change.at, &mut photo);
            changed.push(photo);
        }
    }
    // A photo stored just as it was is not written again: laying a change already shown costs
    // a read per photo, and nothing redraws.
    l.put_photos(&changed)
}

/// The flag changes still waiting, laid over what a pass stored, in the order made.
pub fn replay(library: &mut Library) -> quince_core::Result<()> {
    replay_but(library, None)
}

/// [`replay`], all but `except`. A photo set aside that no waiting delete-for-good names any
/// more — its change let go of, or turned down — is put back: aside only while Apple may still
/// be deleting it.
fn replay_but(l: &mut Library, except: Option<i64>) -> quince_core::Result<()> {
    let mut claimed = HashSet::new();
    for w in l.outbox_pending(PHOTOS)? {
        if Some(w.id) == except {
            continue;
        }
        if w.kind == kind_of(Flag::Expunged) {
            if let Ok(change) = serde_json::from_str::<FlagChange>(&w.payload) {
                claimed.extend(change.photos.into_iter().map(|f| f.id));
            }
        }
        lay(l, &w)?;
    }
    let stray: Vec<String> = l
        .photos_aside()?
        .into_iter()
        .filter(|id| !claimed.contains(id))
        .collect();
    if !stray.is_empty() {
        l.bring_photos_back(&stray)?;
    }
    Ok(())
}

// --- Sending: the queue's half.

/// What Apple answered a flag set on one library's photos: each photo's outcome, in order, or
/// the request's own failure.
pub type Answers = Result<Vec<Result<serde_json::Value, Error>>, Error>;

/// One queued flag change sent to Apple with the session `(http, root, params)`.
pub async fn send(
    library: &LibraryLock,
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    pending: Pending,
) -> Result<(), Error> {
    send_with(library, pending, |place, assets, flag, on| {
        let params = params.clone();
        async move { p::set_flags(http, root, params, place.as_ref(), &assets, flag, on).await }
    })
    .await
}

/// One queued flag change sent through `apple`: each library's photos to its own place, in one
/// request a library (a change holds no more than one request takes).
///
/// Safe to send twice: a flag is set to a value, not toggled, and Apple takes it over the
/// version it holds now when the one it was made over has moved; a photo Apple no longer has
/// is one there is nothing left to change, and one deleted for good is gone as asked.
///
/// What Apple took is kept (its new version, and its own date for leaving Recently Deleted) and
/// comes off the change; what is left of it waits — the queue tries again when Apple could not
/// be reached — or, refused, is put back as it was and kept, with Apple's reason, to be sent
/// again or let go.
pub async fn send_with<F, Fut>(
    library: &LibraryLock,
    pending: Pending,
    apple: F,
) -> Result<(), Error>
where
    F: Fn(Option<Place>, Vec<(String, Option<String>)>, Flag, bool) -> Fut,
    Fut: std::future::Future<Output = Answers>,
{
    let flag = flag_of(&pending.kind).ok_or_else(|| {
        Error::Refused(format!(
            "Skykeep does not know how to send “{}”.",
            pending.kind
        ))
    })?;
    let change: FlagChange = serde_json::from_str(&pending.payload).map_err(|e| unreadable(&e))?;
    // Each photo's library, as the library knows them now; a photo with nothing left to change
    // (gone since, or its Shared Library left) is done.
    let mut done: Vec<(usize, Option<serde_json::Value>)> = Vec::new();
    let mut places: Vec<(Option<Place>, Vec<usize>)> = Vec::new();
    {
        let l = library.lock().await;
        let libraries = l.shared_libraries()?;
        // Set aside by a delete-for-good queued behind this change: still here, and this
        // change goes first, as made (Apple deletes for good only what is in Recently Deleted).
        let aside: HashSet<String> = l.photos_aside()?.into_iter().collect();
        let mut missing = false;
        for (i, f) in change.photos.iter().enumerate() {
            let place = match &f.zone {
                None => Some(None),
                Some(zone) => libraries
                    .iter()
                    .find(|s| s.zone == *zone)
                    .map(|s| Some(place_of(s))),
            };
            let Some(place) = place else {
                done.push((i, None));
                continue;
            };
            // Deleted for good, a photo is aside, not in the library: expected, not gone.
            if flag != Flag::Expunged && !aside.contains(&f.id) && l.photo(&f.id)?.is_none() {
                missing = true;
                done.push((i, None));
                continue;
            }
            match places.iter_mut().find(|(p, _)| *p == place) {
                Some((_, at)) => at.push(i),
                None => places.push((place, vec![i])),
            }
        }
        // Not in a library that has never had a pass (made again after damage) is not known
        // yet, not gone: the change waits for the pass.
        if missing && l.last_synced(p::SERVICE)?.is_none() {
            return Err(Error::Busy);
        }
    }
    let mut failed: Vec<(Vec<usize>, Error)> = Vec::new();
    for (place, at) in places {
        let assets: Vec<(String, Option<String>)> = at
            .iter()
            .map(|&i| (change.photos[i].id.clone(), change.photos[i].tag.clone()))
            .collect();
        match apple(place, assets, flag, change.on).await {
            Ok(answers) => {
                let mut answers = answers.into_iter();
                for &i in &at {
                    match answers.next() {
                        Some(Ok(record)) => done.push((i, Some(record))),
                        Some(Err(Error::NotFound)) => done.push((i, None)),
                        Some(Err(e)) => failed.push((vec![i], e)),
                        None => failed.push((
                            vec![i],
                            Error::Protocol("CloudKit did not answer for it".into()),
                        )),
                    }
                }
            }
            // One library's request failing leaves the others' answers as Apple gave them.
            Err(e) => failed.push((at, e)),
        }
    }
    let mut left: Vec<usize> = failed
        .iter()
        .flat_map(|(at, _)| at.iter().copied())
        .collect();
    left.sort_unstable();
    let waits = failed.iter().position(|(_, e)| transient(e, pending.tries));
    library.lock().await.atomically(|l| {
        settle(l, flag, &change, &done)?;
        if left.is_empty() {
            return Ok(());
        }
        let rest = FlagChange {
            on: change.on,
            at: change.at,
            photos: left.iter().map(|&i| change.photos[i].clone()).collect(),
        };
        l.outbox_narrow(pending.id, &json(&rest)?)?;
        if waits.is_none() {
            // Refused: as it was, under whatever else still waits to go.
            put_back(l, flag, &rest)?;
            replay_but(l, Some(pending.id))?;
        }
        Ok::<_, quince_core::Error>(())
    })?;
    if left.is_empty() {
        return Ok(());
    }
    if let Some(i) = waits {
        return Err(failed.swap_remove(i).1);
    }
    let why = &failed[0].1;
    let total = change.photos.len();
    Err(Error::Refused(if total == 1 {
        why.to_string()
    } else {
        format!(
            "{} of {total} photos could not be changed: {why}",
            left.len()
        )
    }))
}

/// What Apple took, kept: each photo's new version and, put in Recently Deleted, Apple's own
/// date for when it goes; deleted for good, forgotten.
fn settle(
    l: &mut Library,
    flag: Flag,
    change: &FlagChange,
    done: &[(usize, Option<serde_json::Value>)],
) -> quince_core::Result<()> {
    if flag == Flag::Expunged {
        let ids: Vec<String> = done
            .iter()
            .map(|(i, _)| change.photos[*i].id.clone())
            .collect();
        l.forget_photos_aside(&ids)?;
        return l.forget_photos(&ids);
    }
    let mut kept = Vec::new();
    for (i, record) in done {
        let Some(record) = record else { continue };
        let Some(mut photo) = l.photo(&change.photos[*i].id)? else {
            continue;
        };
        if let Some(tag) = record
            .get("recordChangeTag")
            .and_then(serde_json::Value::as_str)
        {
            photo.change_tag = Some(tag.to_string());
        }
        if flag == Flag::Deleted && change.on {
            if let Some(ms) = record
                .pointer("/fields/dateExpunged/value")
                .and_then(serde_json::Value::as_i64)
            {
                photo.expunge_at = Some(ms / 1000);
            }
        }
        kept.push(photo);
    }
    l.put_photos(&kept)
}

/// A refused change's photos as they were before it: its flag on each (the rest are their own
/// changes'), or — deleted for good — brought back from aside.
fn put_back(l: &mut Library, flag: Flag, change: &FlagChange) -> quince_core::Result<()> {
    if flag == Flag::Expunged {
        let ids: Vec<String> = change.photos.iter().map(|f| f.id.clone()).collect();
        return l.bring_photos_back(&ids);
    }
    let mut back = Vec::with_capacity(change.photos.len());
    for f in &change.photos {
        if let Some(mut photo) = l.photo(&f.id)? {
            f.was.put_back(flag, &mut photo);
            back.push(photo);
        }
    }
    l.put_photos(&back)
}

fn place_of(s: &quince_core::shared::SharedLibrary) -> Place {
    Place {
        database: if s.database == "shared" {
            quince_icloud::cloudkit::Database::Shared
        } else {
            quince_icloud::cloudkit::Database::Private
        },
        zone: s.zone.clone(),
        owner: s.owner.clone(),
    }
}

/// Which library a photo is in: the person's own (`None`), or a Shared Library's zone.
pub fn library_place(library: &Library, id: &str) -> Result<Option<Place>, String> {
    Ok(library
        .shared_library_of(id)
        .map_err(|e| e.to_string())?
        .as_ref()
        .map(place_of))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::{account_tag, drain, drain_as, Outcome, Report, ACCOUNT_KEY};
    use quince_core::photos::{Kind, View};
    use serde_json::{json, Value};
    use tokio::sync::Mutex;

    /// Every request the pretend Apple was sent: the photos, the flag and its value.
    type Calls = std::sync::Mutex<Vec<(Vec<String>, Flag, bool)>>;

    fn photo(id: &str) -> Photo {
        Photo {
            id: id.into(),
            master: format!("m-{id}"),
            taken_at: 1_700_000_000,
            added_at: None,
            kind: Kind::Image,
            width: 4032,
            height: 3024,
            duration: None,
            filename: Some(format!("{id}.jpg")),
            size: Some(1),
            uti: Some("public.jpeg".into()),
            favorite: false,
            hidden: false,
            deleted: false,
            deleted_at: None,
            subtype: 0,
            change_tag: Some(format!("{id}-1")),
            expunge_at: None,
            title: None,
            caption: None,
            rating: 0,
            keywords: Vec::new(),
            latitude: None,
            longitude: None,
            rendered: 0,
            burst: None,
            burst_pick: false,
            burst_count: 0,
            live_size: None,
        }
    }

    fn in_recently_deleted(id: &str) -> Photo {
        Photo {
            deleted: true,
            deleted_at: Some(1_700_000_000),
            expunge_at: Some(1_702_592_000),
            ..photo(id)
        }
    }

    /// A library that has had a pass, holding `photos`.
    fn stocked(l: &mut Library, photos: &[Photo]) {
        l.put_photos(photos).unwrap();
        l.mark_synced(PHOTOS, Some("token")).unwrap();
    }

    fn ids(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| (*s).to_string()).collect()
    }

    /// Apple taking a photo's change: its record with a new tag, and a date to leave Recently
    /// Deleted.
    fn taken(id: &str) -> Result<Value, Error> {
        Ok(json!({
            "recordName": id,
            "recordChangeTag": format!("{id}-2"),
            "fields": { "dateExpunged": { "value": 1_800_000_000_000_i64 } },
        }))
    }

    /// One pass over the queue, sent through a pretend Apple that answers each request with
    /// `reply`, and notes it in `calls`.
    async fn drain_with(
        lib: &LibraryLock,
        calls: &Calls,
        reply: impl Fn(&[(String, Option<String>)]) -> Answers,
    ) -> Report {
        let pass = Mutex::new(());
        drain(lib, PHOTOS, &pass, |p| {
            send_with(lib, p, |_place, assets, flag, on| {
                calls.lock().unwrap().push((
                    assets.iter().map(|a| a.0.clone()).collect(),
                    flag,
                    on,
                ));
                std::future::ready(reply(&assets))
            })
        })
        .await
    }

    fn all_taken(assets: &[(String, Option<String>)]) -> Answers {
        Ok(assets.iter().map(|(id, _)| taken(id)).collect())
    }

    fn waiting(lib: &Library) -> Vec<(String, usize, i64)> {
        lib.outbox_pending(PHOTOS)
            .unwrap()
            .into_iter()
            .map(|p| {
                let c: FlagChange = serde_json::from_str(&p.payload).unwrap();
                (p.kind, c.photos.len(), p.tries)
            })
            .collect()
    }

    /// Recorded, then the app dies — before it was sent, and again with it on its way —
    /// reopened, the change is there and shown, and the next drain sends it, once, from the
    /// version it was made over.
    #[tokio::test]
    async fn a_flag_recorded_before_a_crash_is_sent_on_the_next_drain() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut l = Library::open(dir.path()).unwrap();
            stocked(&mut l, &[photo("A"), photo("B")]);
            let now = record_flags(&mut l, &ids(&["A", "B"]), Flag::Favorite, true).unwrap();
            assert!(now.iter().all(|p| p.favorite), "shown at once");
        }
        {
            // Taken to be sent when the process died.
            let mut l = Library::open(dir.path()).unwrap();
            assert_eq!(waiting(&l), vec![("favorite".into(), 2, 0)]);
            l.outbox_take(PHOTOS).unwrap();
        }
        let lib = LibraryLock::new(
            Library::open(dir.path()).unwrap(),
            Some(dir.path().to_path_buf()),
        );
        {
            let l = lib.lock().await;
            assert!(l.photo("A").unwrap().unwrap().favorite, "still shown");
            let change: FlagChange =
                serde_json::from_str(&l.outbox_pending(PHOTOS).unwrap()[0].payload).unwrap();
            assert_eq!(
                change.photos[0].tag.as_deref(),
                Some("A-1"),
                "made over A-1"
            );
        }
        let calls = Calls::default();
        let report = drain_with(&lib, &calls, all_taken).await;
        assert!(report.sent && report.outcome.is_clear() && report.refused.is_empty());
        assert_eq!(
            *calls.lock().unwrap(),
            vec![(ids(&["A", "B"]), Flag::Favorite, true)]
        );
        let l = lib.lock().await;
        assert!(l.outbox_pending(PHOTOS).unwrap().is_empty());
        let a = l.photo("A").unwrap().unwrap();
        assert!(a.favorite);
        assert_eq!(a.change_tag.as_deref(), Some("A-2"), "Apple's version kept");
    }

    /// A pass that stores Apple's version while changes wait — the old flags, the photo
    /// deleted for good back in Recently Deleted — shows the changes still, not what it read.
    #[tokio::test]
    async fn a_pass_never_shows_a_waiting_change_undone() {
        let mut l = Library::in_memory().unwrap();
        stocked(&mut l, &[photo("A"), photo("B"), in_recently_deleted("C")]);
        record_flags(&mut l, &ids(&["A"]), Flag::Hidden, true).unwrap();
        record_flags(&mut l, &ids(&["B"]), Flag::Deleted, true).unwrap();
        record_flags(&mut l, &ids(&["C"]), Flag::Expunged, true).unwrap();
        assert!(
            l.photo("C").unwrap().is_none(),
            "gone from every view at once"
        );
        // Apple's records, as a pass read them before any of it was sent.
        l.atomically(|l| {
            l.put_photos(&[photo("A"), photo("B"), in_recently_deleted("C")])?;
            replay(l)
        })
        .unwrap();
        assert!(l.photo("A").unwrap().unwrap().hidden);
        let b = l.photo("B").unwrap().unwrap();
        assert!(b.deleted && b.deleted_at.is_some() && b.expunge_at.is_some());
        assert!(l.photo("C").unwrap().is_none());
        assert!(l
            .photos(View::RecentlyDeleted)
            .unwrap()
            .iter()
            .all(|p| p.id != "C"));
        assert_eq!(
            l.photos_aside().unwrap(),
            ids(&["C"]),
            "kept aside, not forgotten"
        );
        // Laid again with nothing new: nothing moves.
        replay(&mut l).unwrap();
        assert_eq!(waiting(&l).len(), 3);
    }

    /// Deleted for good: gone from view at once, forgotten only once Apple has it; one Apple
    /// refuses (recovered on the phone meanwhile) comes back, and the refusal is kept, with
    /// Apple's reason, for that photo alone.
    #[tokio::test]
    async fn a_delete_for_good_refused_brings_the_photo_back() {
        let lib = LibraryLock::new(Library::in_memory().unwrap(), None);
        {
            let mut l = lib.lock().await;
            stocked(
                &mut l,
                &[in_recently_deleted("C"), in_recently_deleted("D")],
            );
            assert!(
                record_flags(&mut l, &ids(&["C", "A"]), Flag::Expunged, false).is_err(),
                "never off"
            );
            l.put_photos(&[photo("E")]).unwrap();
            assert!(
                record_flags(&mut l, &ids(&["E"]), Flag::Expunged, true).is_err(),
                "only from Recently Deleted"
            );
            record_flags(&mut l, &ids(&["C", "D"]), Flag::Expunged, true).unwrap();
            assert!(l.photos(View::RecentlyDeleted).unwrap().is_empty());
        }
        let calls = Calls::default();
        let report = drain_with(&lib, &calls, |assets| {
            Ok(assets
                .iter()
                .map(|(id, _)| {
                    if id == "C" {
                        Err(Error::Refused(
                            "This photo was recovered on another device, so it was kept.".into(),
                        ))
                    } else {
                        taken(id)
                    }
                })
                .collect())
        })
        .await;
        assert_eq!(report.refused.len(), 1);
        assert!(
            report.refused[0].1.contains("1 of 2"),
            "{:?}",
            report.refused
        );
        let mut l = lib.lock().await;
        let c = l.photo("C").unwrap().expect("back");
        assert!(c.deleted, "back where it was");
        assert!(l.photo("D").unwrap().is_none(), "D is gone");
        assert!(
            l.photos_aside().unwrap().is_empty(),
            "D forgotten once Apple had it"
        );
        let refused = l.outbox_refused().unwrap();
        let left: FlagChange = serde_json::from_str(&refused[0].change.payload).unwrap();
        assert_eq!(
            left.photos
                .iter()
                .map(|f| f.id.as_str())
                .collect::<Vec<_>>(),
            ["C"]
        );
        // A pass after: the refusal is not laid, so C stays.
        replay(&mut l).unwrap();
        assert!(l.photo("C").unwrap().is_some());
    }

    /// A photo Apple no longer has — deleted for good by an earlier send of this same change,
    /// or on another device — is done, not refused.
    #[tokio::test]
    async fn a_delete_for_good_sent_twice_is_done() {
        let lib = LibraryLock::new(Library::in_memory().unwrap(), None);
        {
            let mut l = lib.lock().await;
            stocked(&mut l, &[in_recently_deleted("C")]);
            record_flags(&mut l, &ids(&["C"]), Flag::Expunged, true).unwrap();
        }
        let calls = Calls::default();
        let report = drain_with(&lib, &calls, |assets| {
            Ok(assets.iter().map(|_| Err(Error::NotFound)).collect())
        })
        .await;
        assert!(report.sent && report.refused.is_empty());
        let l = lib.lock().await;
        assert!(l.outbox_pending(PHOTOS).unwrap().is_empty());
        assert!(l.photos_aside().unwrap().is_empty());
        assert!(l.photo("C").unwrap().is_none());
    }

    /// A refused flag goes back as it was before the change, under whatever else waits.
    #[tokio::test]
    async fn a_refused_flag_is_put_back_under_what_still_waits() {
        let lib = LibraryLock::new(Library::in_memory().unwrap(), None);
        {
            let mut l = lib.lock().await;
            stocked(&mut l, &[photo("A")]);
            record_flags(&mut l, &ids(&["A"]), Flag::Favorite, true).unwrap();
            record_flags(&mut l, &ids(&["A"]), Flag::Hidden, true).unwrap();
        }
        let calls = Calls::default();
        let report = drain_with(&lib, &calls, |assets| {
            Ok(assets
                .iter()
                .map(|_| Err(Error::Refused("not allowed".into())))
                .collect())
        })
        .await;
        // Both refused, in turn; each put back.
        assert_eq!(report.refused.len(), 2);
        let l = lib.lock().await;
        let a = l.photo("A").unwrap().unwrap();
        assert!(!a.favorite && !a.hidden);
        assert_eq!(l.outbox_refused().unwrap().len(), 2);
    }

    /// Apple out of reach part way: what it took comes off the change, the rest waits and goes
    /// alone next time; nothing is put back meanwhile.
    #[tokio::test]
    async fn a_send_cut_off_part_way_sends_only_what_is_left() {
        let lib = LibraryLock::new(Library::in_memory().unwrap(), None);
        {
            let mut l = lib.lock().await;
            stocked(&mut l, &[photo("A"), photo("B")]);
            record_flags(&mut l, &ids(&["A", "B"]), Flag::Hidden, true).unwrap();
        }
        let calls = Calls::default();
        let report = drain_with(&lib, &calls, |assets| {
            Ok(assets
                .iter()
                .map(|(id, _)| {
                    if id == "B" {
                        Err(Error::Busy)
                    } else {
                        taken(id)
                    }
                })
                .collect())
        })
        .await;
        assert_eq!(report.outcome, Outcome::Later);
        {
            let l = lib.lock().await;
            assert_eq!(waiting(&l), vec![("hidden".into(), 1, 1)]);
            assert!(l.photo("B").unwrap().unwrap().hidden, "still shown as made");
        }
        let report = drain_with(&lib, &calls, all_taken).await;
        assert!(report.outcome.is_clear());
        assert_eq!(calls.lock().unwrap()[1].0, ids(&["B"]));
    }

    /// A library whose owner is another account (signed in over this session's) never has its
    /// queued change sent with this session; its own session sends it.
    #[tokio::test]
    async fn another_accounts_change_is_never_sent_with_this_session() {
        let lib = LibraryLock::new(Library::in_memory().unwrap(), None);
        {
            let mut l = lib.lock().await;
            l.set_state(ACCOUNT_KEY, &account_tag("111")).unwrap();
            stocked(&mut l, &[photo("A")]);
            record_flags(&mut l, &ids(&["A"]), Flag::Favorite, true).unwrap();
        }
        let calls = Calls::default();
        let pass = Mutex::new(());
        let send = |p| {
            send_with(
                &lib,
                p,
                |_place, assets: Vec<(String, Option<String>)>, flag, on| {
                    calls.lock().unwrap().push((
                        assets.iter().map(|a| a.0.clone()).collect(),
                        flag,
                        on,
                    ));
                    std::future::ready(all_taken(&assets))
                },
            )
        };
        let report = drain_as(&lib, PHOTOS, &pass, &account_tag("222"), send).await;
        assert_eq!(report.outcome, Outcome::Later);
        assert!(calls.lock().unwrap().is_empty(), "nothing sent as 222");
        assert_eq!(waiting(&*lib.lock().await), vec![("favorite".into(), 1, 0)]);
        let report = drain_as(&lib, PHOTOS, &pass, &account_tag("111"), send).await;
        assert!(report.sent && report.outcome.is_clear());
        assert_eq!(calls.lock().unwrap().len(), 1);
    }

    /// Five hundred photos in one action: three changes of at most one request each; taken
    /// back before any is sent, none goes; sent with one request failing, the others' are kept
    /// and only that one goes again.
    #[tokio::test]
    async fn five_hundred_photos_are_three_requests_and_an_undo_sends_nothing() {
        let names: Vec<String> = (0..500).map(|i| format!("P{i:03}")).collect();
        let lib = LibraryLock::new(Library::in_memory().unwrap(), None);
        {
            let mut l = lib.lock().await;
            let photos: Vec<Photo> = names.iter().map(|n| photo(n)).collect();
            stocked(&mut l, &photos);
            let now = record_flags(&mut l, &names, Flag::Favorite, true).unwrap();
            assert_eq!(now.len(), 500);
            assert_eq!(
                waiting(&l).iter().map(|(_, n, _)| *n).collect::<Vec<_>>(),
                vec![200, 200, 100]
            );
            assert_eq!(l.photos(View::Favorites).unwrap().len(), 500);
            // Undo, before anything went: nothing waits, nothing is favourite.
            record_flags(&mut l, &names, Flag::Favorite, false).unwrap();
            assert!(waiting(&l).is_empty());
            assert!(l.photos(View::Favorites).unwrap().is_empty());
            record_flags(&mut l, &names, Flag::Favorite, true).unwrap();
        }
        let calls = Calls::default();
        let tries = std::sync::atomic::AtomicUsize::new(0);
        let report = drain_with(&lib, &calls, |assets| {
            if tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                Err(Error::Busy)
            } else {
                all_taken(assets)
            }
        })
        .await;
        assert_eq!(report.outcome, Outcome::Later);
        assert_eq!(
            waiting(&*lib.lock().await),
            vec![("favorite".into(), 200, 1), ("favorite".into(), 100, 0)]
        );
        let report = drain_with(&lib, &calls, all_taken).await;
        assert!(report.outcome.is_clear());
        let calls = calls.into_inner().unwrap();
        assert_eq!(calls.len(), 4);
        assert!(calls.iter().all(|(ids, _, _)| ids.len() <= AT_ONCE));
        let l = lib.lock().await;
        assert!(l.outbox_pending(PHOTOS).unwrap().is_empty());
        assert_eq!(l.photos(View::Favorites).unwrap().len(), 500);
        assert!(l
            .photos(View::Favorites)
            .unwrap()
            .iter()
            .all(|p| p.change_tag.as_deref() == Some(&format!("{}-2", p.id))));
    }

    /// A delete taken back by a recover before it went sends nothing, and the photo keeps the
    /// dates it had; a delete then deleted for good, both waiting, go in that order.
    #[tokio::test]
    async fn a_delete_and_what_follows_it_go_as_made() {
        let lib = LibraryLock::new(Library::in_memory().unwrap(), None);
        {
            let mut l = lib.lock().await;
            stocked(&mut l, &[photo("A"), in_recently_deleted("B")]);
            record_flags(&mut l, &ids(&["B"]), Flag::Deleted, false).unwrap();
            record_flags(&mut l, &ids(&["B"]), Flag::Deleted, true).unwrap();
            assert!(waiting(&l).is_empty());
            assert_eq!(
                l.photo("B").unwrap().unwrap(),
                in_recently_deleted("B"),
                "as it was"
            );
            record_flags(&mut l, &ids(&["A"]), Flag::Deleted, true).unwrap();
            record_flags(&mut l, &ids(&["A"]), Flag::Expunged, true).unwrap();
        }
        let calls = Calls::default();
        let report = drain_with(&lib, &calls, all_taken).await;
        assert!(report.outcome.is_clear() && report.refused.is_empty());
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                (ids(&["A"]), Flag::Deleted, true),
                (ids(&["A"]), Flag::Expunged, true)
            ]
        );
        let l = lib.lock().await;
        assert!(l.photo("A").unwrap().is_none());
        assert!(l.photos_aside().unwrap().is_empty());
    }
}
