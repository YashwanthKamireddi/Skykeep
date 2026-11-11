//! Photos: one round of a pass stored (the pass itself, a few rounds, readiness, Shared Albums
//! and the Shared Library, is `pass::photos`), and Favorite, Hide and Delete sent to each
//! photo's own zone, shared by the desktop and the phone.

use crate::library_lock::LibraryLock;
use quince_core::photos::Photo;
use quince_icloud::photos::{self as p, Links};
use quince_icloud::Error;
use std::collections::HashMap;

/// One round's records stored; a round of a pass from the start marks the library as having had
/// one. Answers how many records it held and their links.
pub fn apply(
    library: &mut quince_core::Library,
    fetched: p::PhotosFetch,
    full: bool,
) -> Result<(usize, HashMap<String, Links>), Error> {
    let records = fetched.records.len();
    let (_, links) = p::apply(library, fetched)?;
    if full {
        library.mark_full_sync(p::SERVICE)?;
    }
    Ok((records, links))
}

/// Favorite, Hide, Delete (to Recently Deleted), Recover, or (from Recently Deleted only) delete
/// for good, on many photos, as Photos does it: the library changes first, so every view (and a
/// sync pass reading it) shows it at once, then Apple is told in batches, each library's photos
/// to its own zone, with no pass between (`pass` is the photos pass lock); whatever Apple
/// refuses is put back as it was. Answers the photos as they now are, or an error naming how
/// many could not be changed. `apple`: the client, CloudKit's address and the session's
/// parameters. The desktop and the phone both call this.
#[allow(clippy::too_many_lines)]
pub async fn set_flags(
    lock: &LibraryLock,
    pass: &tokio::sync::Mutex<()>,
    (http, root, params): (&reqwest::Client, &str, Vec<(String, String)>),
    ids: Vec<String>,
    flag: p::Flag,
    on: bool,
) -> Result<Vec<Photo>, String> {
    use quince_icloud::photos::Flag;
    let now = jiff::Timestamp::now().as_second();
    let set = |p: &mut Photo| match flag {
        Flag::Favorite => p.favorite = on,
        Flag::Hidden => p.hidden = on,
        Flag::Deleted => {
            p.deleted = on;
            p.deleted_at = on.then_some(now);
            p.expunge_at = on.then_some(now + 30 * 86_400);
        }
        Flag::Expunged => {}
    };
    // At once, in the library.
    let before: Vec<Photo> = {
        let mut library = lock.lock().await;
        let mut found = Vec::with_capacity(ids.len());
        for id in &ids {
            if let Some(p) = library.photo(id).map_err(|e| e.to_string())? {
                found.push(p);
            }
        }
        // Deleting for good is only ever from Recently Deleted, and only ever on.
        if flag == Flag::Expunged && (!on || found.iter().any(|p| !p.deleted)) {
            return Err("Only photos in Recently Deleted can be deleted for good.".into());
        }
        // Deleted for good is the one change not made here first: forgotten only once Apple has
        // taken it (below). Forgotten first, a crash or a killed app mid-send left them gone
        // here and still in Recently Deleted at Apple, where no later pass brought them back.
        if flag != Flag::Expunged {
            let changed: Vec<Photo> = found
                .iter()
                .cloned()
                .map(|mut p| {
                    set(&mut p);
                    p
                })
                .collect();
            library.put_photos(&changed).map_err(|e| e.to_string())?;
        }
        found
    };
    if before.is_empty() {
        return Ok(Vec::new());
    }
    // Then to Apple, with no sync pass between the two.
    let _one = pass.lock().await;
    let sent = async {
        // A selection can hold photos of both libraries: each library's are sent to its own
        // zone, and the answers put back in the order the photos were given.
        let places = {
            let library = lock.lock().await;
            before
                .iter()
                .map(|p| library_place(&library, &p.id))
                .collect::<Result<Vec<_>, String>>()?
        };
        let mut answers: Vec<Option<Result<serde_json::Value, String>>> =
            (0..before.len()).map(|_| None).collect();
        let mut zones: Vec<Option<&quince_icloud::shared_collections::Place>> = Vec::new();
        for place in &places {
            if !zones
                .iter()
                .any(|z| z.map(|z| &z.zone) == place.as_ref().map(|p| &p.zone))
            {
                zones.push(place.as_ref());
            }
        }
        for zone in zones {
            let wanted: Vec<usize> = (0..before.len())
                .filter(|&i| places[i].as_ref().map(|p| &p.zone) == zone.map(|z| &z.zone))
                .collect();
            let assets: Vec<(String, Option<String>)> = wanted
                .iter()
                .map(|&i| (before[i].id.clone(), before[i].change_tag.clone()))
                .collect();
            // One library's batch failing leaves the other's answers as Apple gave them: only
            // this library's photos are said not to have gone.
            match quince_icloud::photos::set_flags(
                http,
                root,
                params.clone(),
                zone,
                &assets,
                flag,
                on,
            )
            .await
            {
                Ok(got) => {
                    for (&i, answer) in wanted.iter().zip(got) {
                        answers[i] = Some(answer);
                    }
                }
                Err(e)
                    if places
                        .iter()
                        .all(|p| p.as_ref().map(|p| &p.zone) == zone.map(|z| &z.zone)) =>
                {
                    return Err(e.to_string());
                }
                Err(e) => {
                    for &i in &wanted {
                        answers[i] = Some(Err(e.to_string()));
                    }
                }
            }
        }
        Ok::<_, String>(
            answers
                .into_iter()
                .map(|a| a.unwrap_or_else(|| Err("Not sent.".into())))
                .collect::<Vec<_>>(),
        )
    }
    .await;
    let answers = match sent {
        Ok(a) => a,
        Err(e) => {
            // Nothing reached Apple: the flags as they were, on each photo as it is now.
            let mut library = lock.lock().await;
            let back = flags_back(&library, &before, None);
            library.put_photos(&back).map_err(|e| e.to_string())?;
            return Err(e);
        }
    };
    let mut now_are = Vec::with_capacity(before.len());
    let mut refused = Vec::new();
    let mut gone = Vec::new();
    let mut why = None;
    for (old, answer) in before.iter().zip(answers) {
        match answer {
            Ok(record) => {
                if flag == Flag::Expunged {
                    gone.push(old.id.clone());
                    continue;
                }
                let mut p = old.clone();
                set(&mut p);
                if flag == Flag::Deleted && on {
                    // Apple's own date for when it goes for good.
                    if let Some(ms) = record
                        .pointer("/fields/dateExpunged/value")
                        .and_then(serde_json::Value::as_i64)
                    {
                        p.expunge_at = Some(ms / 1000);
                    }
                }
                p.change_tag = record
                    .get("recordChangeTag")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .or(p.change_tag);
                now_are.push(p);
            }
            Err(e) => {
                why.get_or_insert(e);
                refused.push(old.clone());
            }
        }
    }
    {
        // Only the flags and Apple's new version are laid on each photo as the library holds
        // it now: written whole from the copy read before the wait, a title or caption that a
        // sync pass brought in meanwhile was put back as it had been.
        let mut library = lock.lock().await;
        let done = flags_back(&library, &now_are, Some(()));
        let back = flags_back(&library, &refused, None);
        library.put_photos(&done).map_err(|e| e.to_string())?;
        library.put_photos(&back).map_err(|e| e.to_string())?;
        if !gone.is_empty() {
            library.forget_photos(&gone).map_err(|e| e.to_string())?;
        }
    }
    if let Some(why) = why {
        let n = refused.len();
        return Err(format!(
            "{n} of {} {} could not be changed: {why}",
            before.len(),
            if before.len() == 1 { "photo" } else { "photos" }
        ));
    }
    Ok(now_are)
}

/// `from`'s flags — favourite, hidden, in Recently Deleted and its dates — laid on each photo as
/// the library holds it now; with `tag`, its change tag too. A photo no longer there is `from`
/// as it was.
fn flags_back(library: &quince_core::Library, from: &[Photo], tag: Option<()>) -> Vec<Photo> {
    from.iter()
        .map(|f| match library.photo(&f.id).ok().flatten() {
            Some(mut now) => {
                now.favorite = f.favorite;
                now.hidden = f.hidden;
                now.deleted = f.deleted;
                now.deleted_at = f.deleted_at;
                now.expunge_at = f.expunge_at;
                if tag.is_some() {
                    now.change_tag.clone_from(&f.change_tag);
                }
                now
            }
            None => f.clone(),
        })
        .collect()
}

/// Which library a photo is in: the person's own (`None`), or a Shared Library's zone.
pub fn library_place(
    library: &quince_core::Library,
    id: &str,
) -> Result<Option<quince_icloud::shared_collections::Place>, String> {
    Ok(library
        .shared_library_of(id)
        .map_err(|e| e.to_string())?
        .map(|s| quince_icloud::shared_collections::Place {
            database: if s.database == "shared" {
                quince_icloud::cloudkit::Database::Shared
            } else {
                quince_icloud::cloudkit::Database::Private
            },
            zone: s.zone,
            owner: s.owner,
        }))
}
