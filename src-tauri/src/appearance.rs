//! The desktop's Increase Contrast setting, for the window to follow.
//!
//! WebKitGTK does not map it to `prefers-contrast` (docs/accessibility.md), so on Linux it is
//! read here from the Settings portal — `org.freedesktop.appearance` `contrast`, which GNOME's
//! High Contrast and KDE's set — and handed to the window, which marks its root with
//! `data-contrast="more"`; the same rules as `prefers-contrast: more` then apply (a11y.css).
//! Followed as it changes: switched on in the desktop's settings, the window firms up at once.
//! Elsewhere the engine's own media query already says it, and this reads as unset.
//!
//! The same portal's `color-scheme` says whether the desktop is dark: GNOME 42+ says it only
//! there, which GTK3 (and so WebKitGTK) never reads, so "Auto" would open light on a dark
//! GNOME and stay so at sunset. It is read for the window's own theme and followed as well.

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

/// Whether the desktop is dark, from the portal's `color-scheme` (1 prefers dark, 2 prefers
/// light) or, with no preference (0) or no portal, from what GTK said.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn dark(color_scheme: Option<u32>, gtk: bool) -> bool {
    match color_scheme {
        Some(1) => true,
        Some(2) => false,
        _ => gtk,
    }
}

/// Whether the desktop asks for more contrast now.
#[tauri::command]
pub async fn system_contrast() -> bool {
    #[cfg(target_os = "linux")]
    {
        if let Some(settings) = portal().await {
            return read(&settings, "contrast").await.is_some_and(more);
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
async fn read(settings: &zbus::Proxy<'static>, key: &str) -> Option<u32> {
    use zbus::zvariant::OwnedValue;
    let key = ("org.freedesktop.appearance", key);
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

/// Follow the settings for as long as the app runs: contrast is told to every window, and a
/// change of `color-scheme` redraws the windows' theme while "Auto" is chosen.
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
            if ns != "org.freedesktop.appearance" {
                continue;
            }
            if key == "contrast" {
                let on = u32::try_from(value).is_ok_and(more);
                let _ = app.emit(CHANGED, on);
            } else if key == "color-scheme" {
                let chosen = CHOSEN
                    .lock()
                    .ok()
                    .and_then(|c| c.as_ref().map(|c| c.0.clone()));
                if chosen.as_deref() == Some("auto") {
                    let system = dark(u32::try_from(value).ok(), gtk_dark(&app).await);
                    apply(&app, "auto", system);
                }
            }
        }
    });
    #[cfg(not(target_os = "linux"))]
    let _ = app;
}

/// The appearance chosen in Settings, as the window's own title bar and frame are drawn: the
/// page's `data-theme` darkens only the page, so Dark on a light desktop left a light system
/// title bar over a dark window (Windows, GNOME, KDE). "auto" is the system's: on Windows and
/// macOS `None` hands it back to the system; on Linux the theme is GTK's, app-wide, and `None`
/// there means "not dark", so it is put back to what GTK said before the app first changed it.
fn native(chosen: &str, system_dark: bool) -> Option<tauri::Theme> {
    match chosen {
        "dark" => Some(tauri::Theme::Dark),
        "light" => Some(tauri::Theme::Light),
        _ if system_dark => Some(tauri::Theme::Dark),
        _ => None,
    }
}

/// The appearance last chosen this run, as theme.ts says it, and the theme it drew: for a
/// window made after it (`dress`), and for the desktop turning dark or light under "Auto"
/// (`watch`). Unset until a page says it: a window made before then is left to the system, as
/// it would be anyway.
type Chosen = Option<(String, Option<tauri::Theme>)>;
static CHOSEN: std::sync::Mutex<Chosen> = std::sync::Mutex::new(None);

/// Whether GTK preferred dark before the app first set it: read once, on the main thread GTK
/// is touched from, before any window's theme is changed.
#[cfg(target_os = "linux")]
static GTK_DARK: tokio::sync::OnceCell<bool> = tokio::sync::OnceCell::const_new();

/// The desktop's dark, read afresh from the portal each time (it follows the sun on GNOME).
#[cfg(target_os = "linux")]
async fn system_dark(app: &tauri::AppHandle) -> bool {
    let color_scheme = match portal().await {
        Some(settings) => read(&settings, "color-scheme").await,
        None => None,
    };
    dark(color_scheme, gtk_dark(app).await)
}

#[cfg(target_os = "linux")]
async fn gtk_dark(app: &tauri::AppHandle) -> bool {
    *GTK_DARK
        .get_or_init(|| async {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let asked = app.run_on_main_thread(move || {
                use gtk::prelude::GtkSettingsExt;
                let dark = gtk::Settings::default()
                    .is_some_and(|s| s.is_gtk_application_prefer_dark_theme());
                let _ = tx.send(dark);
            });
            if asked.is_err() {
                return false;
            }
            rx.await.unwrap_or(false)
        })
        .await
}

/// Every window, now and made later, drawn in the appearance chosen: "auto", "light" or
/// "dark", as theme.ts stores it. Each page says it as it starts and again when it changes.
#[tauri::command]
pub async fn window_theme(app: tauri::AppHandle, theme: String) {
    #[cfg(target_os = "linux")]
    let system = system_dark(&app).await;
    #[cfg(not(target_os = "linux"))]
    let system = false;
    apply(&app, &theme, system);
}

fn apply(app: &tauri::AppHandle, theme: &str, system_dark: bool) {
    use tauri::Manager;
    let native = native(theme, system_dark);
    if let Ok(mut chosen) = CHOSEN.lock() {
        *chosen = Some((theme.to_owned(), native));
    }
    for window in app.webview_windows().values() {
        window.set_theme(native).ok();
    }
}

/// A window just made, in the appearance chosen this run, before it is first shown: its page
/// says it again once loaded, but on Windows the frame would first be drawn in the system's.
pub fn dress(window: &tauri::WebviewWindow) {
    if let Some(native) = CHOSEN.lock().ok().and_then(|c| c.as_ref().map(|c| c.1)) {
        window.set_theme(native).ok();
    }
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
    fn the_chosen_appearance_is_the_windows_own() {
        use tauri::Theme::{Dark, Light};
        assert_eq!(super::native("dark", false), Some(Dark));
        assert_eq!(super::native("light", false), Some(Light));
        // Chosen over a dark GTK: Light is still light.
        assert_eq!(super::native("light", true), Some(Light));
        // The system's: handed back, or on Linux GTK's own dark put back.
        assert_eq!(super::native("auto", false), None);
        assert_eq!(super::native("auto", true), Some(Dark));
        assert_eq!(super::native("anything else", false), None);
    }

    #[test]
    fn the_portals_color_scheme_says_dark_before_gtk() {
        // 1 prefers dark, 2 prefers light, whatever GTK says: GNOME's dark style leaves GTK3
        // light.
        assert!(super::dark(Some(1), false));
        assert!(!super::dark(Some(2), true));
        // No preference, an unknown value or no portal: GTK's own.
        assert!(super::dark(Some(0), true));
        assert!(!super::dark(Some(0), false));
        assert!(super::dark(Some(7), true));
        assert!(super::dark(None, true));
        assert!(!super::dark(None, false));
    }

    #[test]
    fn only_the_portals_high_contrast_value_asks_for_more() {
        assert!(super::more(1));
        assert!(!super::more(0));
        assert!(!super::more(2));
    }
}
