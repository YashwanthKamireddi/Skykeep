//! The desktop's Increase Contrast setting, for the window to follow.
//!
//! WebKitGTK does not map it to `prefers-contrast` (docs/accessibility.md), so on Linux it is
//! read here from the Settings portal — `org.freedesktop.appearance` `contrast`, which GNOME's
//! High Contrast and KDE's set — and handed to the window, which marks its root with
//! `data-contrast="more"`; the same rules as `prefers-contrast: more` then apply (a11y.css).
//! Followed as it changes: switched on in the desktop's settings, the window firms up at once.
//! Elsewhere the engine's own media query already says it, and this reads as unset.

#[cfg(target_os = "linux")]
use tauri::Emitter;

/// Told to every window when the setting changes: `true` for more contrast.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const CHANGED: &str = "quince://contrast";

/// The portal's value: 1 is "higher contrast"; 0, absent or anything else is the default.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn more(value: u32) -> bool {
    value == 1
}

/// Whether the desktop asks for more contrast now.
#[tauri::command]
pub async fn system_contrast() -> bool {
    #[cfg(target_os = "linux")]
    {
        if let Some(settings) = portal().await {
            return read(&settings).await.is_some_and(more);
        }
    }
    false
}

#[cfg(target_os = "linux")]
async fn portal() -> Option<zbus::Proxy<'static>> {
    let bus = zbus::Connection::session().await.ok()?;
    zbus::proxy::Builder::<zbus::Proxy<'static>>::new(&bus)
        .destination("org.freedesktop.portal.Desktop")
        .ok()?
        .path("/org/freedesktop/portal/desktop")
        .ok()?
        .interface("org.freedesktop.portal.Settings")
        .ok()?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .ok()
}

/// `ReadOne`, or `Read` on a portal older than version 2, which wraps the value once more.
#[cfg(target_os = "linux")]
async fn read(settings: &zbus::Proxy<'static>) -> Option<u32> {
    use zbus::zvariant::OwnedValue;
    let key = ("org.freedesktop.appearance", "contrast");
    if let Ok(v) = settings.call::<_, _, OwnedValue>("ReadOne", &key).await {
        return u32::try_from(v).ok();
    }
    let v: OwnedValue = settings.call("Read", &key).await.ok()?;
    let inner = zbus::zvariant::Value::from(v);
    match inner {
        zbus::zvariant::Value::Value(boxed) => u32::try_from(*boxed).ok(),
        other => u32::try_from(other).ok(),
    }
}

/// Follow the setting for as long as the app runs, telling every window when it moves.
pub fn watch(app: tauri::AppHandle) {
    #[cfg(target_os = "linux")]
    tauri::async_runtime::spawn(async move {
        use futures_util::StreamExt;
        use zbus::zvariant::OwnedValue;
        let Some(settings) = portal().await else {
            return;
        };
        let Ok(mut changes) = settings.receive_signal("SettingChanged").await else {
            return;
        };
        while let Some(signal) = changes.next().await {
            let Ok((ns, key, value)) = signal.body().deserialize::<(String, String, OwnedValue)>()
            else {
                continue;
            };
            if ns == "org.freedesktop.appearance" && key == "contrast" {
                let on = u32::try_from(value).is_ok_and(more);
                let _ = app.emit(CHANGED, on);
            }
        }
    });
    #[cfg(not(target_os = "linux"))]
    let _ = app;
}

/// The locale dates are written in here, as a BCP 47 tag (`en-GB`): `LC_ALL`, then `LC_TIME`,
/// then `LANG`, as the C library reads them. WebKitGTK's `navigator.language` follows `LANG`
/// alone, so a British `LC_TIME` under an American `LANG` would start the week on Sunday; every
/// window is given this before its page runs (main.rs, "time-locale"). None for C, POSIX or
/// nothing set.
pub fn system_time_locale() -> Option<String> {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    tag(&var("LC_ALL")
        .or_else(|| var("LC_TIME"))
        .or_else(|| var("LANG"))?)
}

/// `en_GB.UTF-8@euro` as `en-GB`.
fn tag(posix: &str) -> Option<String> {
    let bare = posix.split(['.', '@']).next()?.trim();
    if bare.is_empty() || bare == "C" || bare == "POSIX" {
        return None;
    }
    Some(bare.replace('_', "-"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_posix_locale_reads_as_a_language_tag() {
        assert_eq!(super::tag("en_GB.UTF-8").as_deref(), Some("en-GB"));
        assert_eq!(super::tag("de_DE@euro").as_deref(), Some("de-DE"));
        assert_eq!(super::tag("fr").as_deref(), Some("fr"));
        assert_eq!(super::tag("C.UTF-8"), None);
        assert_eq!(super::tag("POSIX"), None);
    }

    #[test]
    fn only_the_portals_high_contrast_value_asks_for_more() {
        assert!(super::more(1));
        assert!(!super::more(0));
        assert!(!super::more(2));
    }
}
