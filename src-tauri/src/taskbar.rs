//! Skykeep's six apps on the Windows taskbar and Start menu, as Linux has them in its launcher
//! (apps.rs, packaging/apps): each app's window a taskbar button of its own, and the jump list's
//! tasks — New Note, New Reminder, New Event, and the six apps.
//!
//! Windows groups windows by AppUserModelID. The process says Skykeep's, the identifier, which
//! the installer stamps on the "Skykeep" Start-menu shortcut; each app's window says its own,
//! `<identifier>.Notes` and so on — Microsoft's "SubProduct", a separate taskbar button per
//! subapplication — which the installer stamps on that app's own shortcut
//! (packaging/windows). A window with the same ID as a shortcut is pinned as that shortcut;
//! one with none to match is pinned by the relaunch command it carries here.
//!
//! From an MSIX package (the Microsoft Store, package.rs) Windows knows the package's entries
//! only as `<family name>!<application id>`, and a process may claim no other ID; the manifest
//! (packaging/windows/msix) gives the entries the same IDs as above, so there each ID is the
//! usual one with the family name in front.
//!
//! A jump list's tasks are written by the running app; no installer can declare them. Each is
//! a command line the app already takes (`--new note`, `--app notes`), handed by the single
//! instance to the Skykeep already running.

use std::sync::OnceLock;

use windows::core::{Interface, HSTRING, PWSTR};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Storage::EnhancedStorage::{
    PKEY_AppUserModel_ID, PKEY_AppUserModel_IsDestListSeparator, PKEY_AppUserModel_RelaunchCommand,
    PKEY_AppUserModel_RelaunchDisplayNameResource, PKEY_AppUserModel_RelaunchIconResource,
    PKEY_Title,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemAlloc, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::Win32::UI::Shell::{
    DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW,
    SetCurrentProcessExplicitAppUserModelID, ShellLink,
};

use crate::apps::{launch_name, APPS, NEW};

/// Skykeep's AppUserModelID: the identifier the installer was built with, so it is the one on
/// its shortcuts even in a launch run under the old identifier (migrate.rs) — in a package,
/// with the package's family name in front.
static APP_ID: OnceLock<String> = OnceLock::new();

/// The ID Skykeep's toasts are sent as: the one claimed, or `identifier` before any claim.
pub fn own_id(identifier: &str) -> String {
    APP_ID.get().cloned().unwrap_or_else(|| {
        crate::package::app_id(crate::package::family_name().as_deref(), identifier)
    })
}

/// Said before any window is made: every window, toast and jump list after it is Skykeep's.
/// The jump lists are written meanwhile, off the startup path.
pub fn claim(identifier: &str) {
    let id = APP_ID.get_or_init(|| {
        crate::package::app_id(crate::package::family_name().as_deref(), identifier)
    });
    // SAFETY: a plain call with a valid, NUL-terminated string the shell copies.
    if let Err(e) = unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(id.as_str())) }
    {
        quiet!("the taskbar was not told Skykeep's app ID: {e}");
    }
    std::thread::spawn(|| {
        // SAFETY: this thread's own apartment, entered and left here; every COM object is
        // released before CoUninitialize, as each goes out of scope inside write_jump_lists.
        let entered = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
        if let Err(e) = write_jump_lists() {
            quiet!("the jump list could not be written: {e}");
        }
        if entered {
            // SAFETY: as above, once for the CoInitializeEx that succeeded.
            unsafe { CoUninitialize() };
        }
    });
}

/// The ID a window of app `key` is known by: Skykeep's with the app's name, `<id>.Notes`.
fn app_id(key: &str) -> Option<String> {
    let base = APP_ID.get()?;
    let (_, name) = APPS.iter().find(|(k, _)| *k == key)?;
    Some(format!("{base}.{name}"))
}

/// This program, as a jump list or a relaunch command runs it.
fn exe() -> Option<String> {
    std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Skykeep's jump list — New Note, New Reminder, New Event, then the six apps — and each app's
/// own: Notes' button offers New Note, Reminders' New Reminder, Calendar's New Event.
fn write_jump_lists() -> windows::core::Result<()> {
    let Some(exe) = exe() else {
        return Ok(());
    };
    let Some(main) = APP_ID.get() else {
        return Ok(());
    };
    let mut tasks: Vec<Task> = NEW
        .iter()
        .map(|(kind, title, _)| Task::Run(title, format!("--new {kind}")))
        .collect();
    tasks.push(Task::Separator);
    tasks.extend(
        APPS.iter()
            .map(|(key, name)| Task::Run(name, format!("--app {}", launch_name(key)))),
    );
    write_jump_list(main, &exe, &tasks)?;
    for (key, _) in APPS {
        let Some(id) = app_id(key) else { continue };
        let own: Vec<Task> = NEW
            .iter()
            .filter(|(_, _, app)| *app == key)
            .map(|(kind, title, _)| Task::Run(title, format!("--new {kind}")))
            .collect();
        write_jump_list(&id, &exe, &own)?;
    }
    Ok(())
}

/// One line of a jump list's Tasks: a command, or the line between two groups of them.
enum Task<'a> {
    Run(&'a str, String),
    Separator,
}

/// The Tasks of the jump list of `app_id`, replacing whatever was there.
fn write_jump_list(app_id: &str, exe: &str, tasks: &[Task]) -> windows::core::Result<()> {
    // SAFETY: COM calls on objects made here, on a thread in an apartment (claim).
    unsafe {
        let list: ICustomDestinationList =
            CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)?;
        list.SetAppID(&HSTRING::from(app_id))?;
        let mut slots = 0u32;
        let _removed: IObjectArray = list.BeginList(&mut slots)?;
        if !tasks.is_empty() {
            let collection: IObjectCollection =
                CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
            for task in tasks {
                collection.AddObject(&link(exe, task)?)?;
            }
            list.AddUserTasks(&collection.cast::<IObjectArray>()?)?;
        }
        list.CommitList()
    }
}

/// The icon of the app a task opens (`--app notes`, `--new note`): that app's own, installed in
/// icons\ beside the program; Skykeep's where there is none (an MSIX package, a build run in
/// place).
fn icon_for(exe: &str, args: &str) -> String {
    let app = match args.split_whitespace().last() {
        Some("note") => "notes",
        Some("reminder") => "reminders",
        Some("event") => "calendar",
        Some(other) => other,
        None => "",
    };
    std::path::Path::new(exe)
        .parent()
        .map(|dir| dir.join("icons").join(format!("{app}.ico")))
        .filter(|ico| ico.exists())
        .map_or_else(|| exe.to_string(), |ico| ico.display().to_string())
}

/// A shell link that runs this program with a task's arguments, under the task's title.
fn link(exe: &str, task: &Task) -> windows::core::Result<IShellLinkW> {
    // SAFETY: as in write_jump_list; each string lives across the call that copies it.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let props: IPropertyStore = link.cast()?;
        match task {
            Task::Run(title, args) => {
                link.SetPath(&HSTRING::from(exe))?;
                link.SetArguments(&HSTRING::from(args.as_str()))?;
                link.SetIconLocation(&HSTRING::from(icon_for(exe, args)), 0)?;
                props.SetValue(&PKEY_Title, &text(title))?;
            }
            Task::Separator => {
                props.SetValue(
                    &PKEY_AppUserModel_IsDestListSeparator,
                    &PROPVARIANT::from(true),
                )?;
            }
        }
        props.Commit()?;
        Ok(link)
    }
}

/// An app's window given its own taskbar button: its ID, and how to start it again when it is
/// pinned (`skykeep --app notes`, named "Skykeep Notes", with Skykeep's icon). Called once the
/// window exists and before it is first shown; done on the main thread, the window's own.
pub fn name_window(window: &tauri::WebviewWindow, key: &str) {
    let (Some(id), Some(exe)) = (app_id(key), exe()) else {
        return;
    };
    let Some((_, name)) = APPS.iter().find(|(k, _)| *k == key) else {
        return;
    };
    let values = vec![
        (
            PKEY_AppUserModel_RelaunchCommand,
            format!("\"{exe}\" --app {}", launch_name(key)),
        ),
        (
            PKEY_AppUserModel_RelaunchDisplayNameResource,
            format!("Skykeep {name}"),
        ),
        (PKEY_AppUserModel_RelaunchIconResource, format!("{exe},0")),
        // The ID last: with it, the relaunch properties above are read.
        (PKEY_AppUserModel_ID, id),
    ];
    set_on_main(window, values);
}

/// The window's properties taken off before it is destroyed, as the shell asks of anyone who
/// set them.
pub fn unname_window(window: &tauri::WebviewWindow) {
    set_on_main(
        window,
        vec![
            (PKEY_AppUserModel_ID, String::new()),
            (PKEY_AppUserModel_RelaunchCommand, String::new()),
            (PKEY_AppUserModel_RelaunchDisplayNameResource, String::new()),
            (PKEY_AppUserModel_RelaunchIconResource, String::new()),
        ],
    );
}

/// Each value written to the window's property store on the main thread; an empty one clears
/// its key.
fn set_on_main(window: &tauri::WebviewWindow, values: Vec<(PROPERTYKEY, String)>) {
    let w = window.clone();
    let done = window.run_on_main_thread(move || {
        let Ok(hwnd) = w.hwnd() else { return };
        // SAFETY: a live window of this process, on its own thread; each value lives across
        // its SetValue, which copies it.
        let set = unsafe {
            SHGetPropertyStoreForWindow::<IPropertyStore>(hwnd).and_then(|store| {
                for (key, value) in &values {
                    let v = if value.is_empty() {
                        PROPVARIANT::default()
                    } else {
                        text(value)
                    };
                    store.SetValue(key, &v)?;
                }
                store.Commit()
            })
        };
        if let Err(e) = set {
            quiet!("a window's taskbar properties were not set: {e}");
        }
    });
    if let Err(e) = done {
        quiet!("a window's taskbar properties were not set: {e}");
    }
}

/// A string as the shell's properties take one: VT_LPWSTR, in memory COM frees (the
/// PROPVARIANT's own drop clears it).
fn text(s: &str) -> PROPVARIANT {
    let wide: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
    let mut v = PROPVARIANT::default();
    // SAFETY: room for every unit and the NUL, copied in whole; the union is written as the
    // VT_LPWSTR it is then marked as, and PropVariantClear (on drop) frees it with CoTaskMemFree.
    unsafe {
        let p = CoTaskMemAlloc(wide.len() * 2).cast::<u16>();
        if p.is_null() {
            return v;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
        let inner = &mut *v.Anonymous.Anonymous;
        inner.vt = VT_LPWSTR;
        inner.Anonymous.pwszVal = PWSTR(p);
    }
    v
}
