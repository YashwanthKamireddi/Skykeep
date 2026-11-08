//! Send to iPhone in Windows 11's own right-click menu: a File Explorer command (`IExplorerCommand`)
//! that the MSIX package declares. Windows 11 lists only packaged apps' commands at the top of
//! the menu; an installed copy offers Send to → iPhone (Skykeep) instead (src-tauri/src/send.rs).
//!
//! The menu asks for the title, icon and state as it opens, so those answer at once with nothing
//! read. Chosen, it hands the paths to Skykeep (`skykeep.exe --send-to-iphone …`, beside this
//! DLL) and returns: the sending is Skykeep's, never the menu's. An item with no path on disk
//! (the navigation pane can hand items without one) is left out, never an error.

use crate::provider::{caught, LIVE};
use std::sync::atomic::Ordering;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::UI::Shell::*;
use windows_core::{implement, Ref, BOOL, GUID, PWSTR};

/// The command's class, declared in the package manifest.
pub const CLSID: GUID = GUID::from_u128(0x9ece47db_eea3_4515_b644_4d7666ddf149);

#[implement(IExplorerCommand)]
pub struct SendToIphone;

impl SendToIphone {
    pub fn new() -> Self {
        LIVE.fetch_add(1, Ordering::SeqCst);
        SendToIphone
    }
}

impl Drop for SendToIphone {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

impl IExplorerCommand_Impl for SendToIphone_Impl {
    fn GetTitle(&self, _items: Ref<IShellItemArray>) -> windows_core::Result<PWSTR> {
        // SAFETY: a static string, copied into memory the caller frees.
        unsafe { SHStrDupW(windows_core::w!("Send to iPhone")) }
    }

    fn GetIcon(&self, _items: Ref<IShellItemArray>) -> windows_core::Result<PWSTR> {
        let exe = skykeep_exe().ok_or(E_FAIL)?;
        // SAFETY: a string alive for the call, copied into memory the caller frees.
        unsafe { SHStrDupW(&windows_core::HSTRING::from(format!("{},0", exe.display()))) }
    }

    fn GetToolTip(&self, _items: Ref<IShellItemArray>) -> windows_core::Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn GetCanonicalName(&self) -> windows_core::Result<GUID> {
        Ok(CLSID)
    }

    fn GetState(&self, _items: Ref<IShellItemArray>, _slow_ok: BOOL) -> windows_core::Result<u32> {
        Ok(ECS_ENABLED.0 as u32)
    }

    fn Invoke(
        &self,
        items: Ref<IShellItemArray>,
        _bind: Ref<IBindCtx>,
    ) -> windows_core::Result<()> {
        caught(|| {
            let items = items.ok()?;
            let paths = paths_of(items);
            if paths.is_empty() {
                return Ok(());
            }
            let exe = skykeep_exe().ok_or(E_FAIL)?;
            std::process::Command::new(exe)
                .arg("--send-to-iphone")
                .args(paths)
                .spawn()
                .map(|_| ())
                .map_err(|_| E_FAIL.into())
        })
    }

    fn GetFlags(&self) -> windows_core::Result<u32> {
        Ok(ECF_DEFAULT.0 as u32)
    }

    fn EnumSubCommands(&self) -> windows_core::Result<IEnumExplorerCommand> {
        Err(E_NOTIMPL.into())
    }
}

/// The paths on disk of the items chosen; those without one are left out.
fn paths_of(items: &IShellItemArray) -> Vec<std::path::PathBuf> {
    // SAFETY: a live array from Explorer; each string is freed once read.
    unsafe {
        let count = items.GetCount().unwrap_or(0);
        (0..count)
            .filter_map(|i| {
                let item = items.GetItemAt(i).ok()?;
                let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
                let path = name.to_string().ok();
                windows::Win32::System::Com::CoTaskMemFree(Some(name.0 as *const _));
                path.map(std::path::PathBuf::from)
            })
            .collect()
    }
}

/// skykeep.exe, beside this DLL.
fn skykeep_exe() -> Option<std::path::PathBuf> {
    use windows::Win32::System::LibraryLoader::*;
    let mut module = HMODULE::default();
    let mut name = [0u16; 1024];
    // SAFETY: an address inside this DLL names it; the buffer is ours.
    let len = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            windows_core::PCWSTR(
                skykeep_exe as fn() -> Option<std::path::PathBuf> as usize as *const u16,
            ),
            &mut module,
        )
        .ok()?;
        GetModuleFileNameW(Some(module), &mut name)
    } as usize;
    let dll = std::path::PathBuf::from(String::from_utf16_lossy(name.get(..len)?));
    Some(dll.parent()?.join("skykeep.exe"))
}
