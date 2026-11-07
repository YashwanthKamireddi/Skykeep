//! iCloud Drive in Windows File Explorer, through the Cloud Files API — the system OneDrive is
//! built on. Every file and folder is a placeholder made from the library, with no network; a
//! file's bytes come from Apple the first time something opens it, and from then on it is a
//! plain NTFS file at full speed. No driver of ours, nothing in the kernel, nothing to install.
//!
//! How it is kept from the bugs other clients shipped (docs/windows-drive.md):
//! - Windows' callbacks only copy what they are told and hand the work to the runtime; nothing
//!   is ever waited on inside one (a callback that blocked deadlocked Nextcloud, PR #2778).
//! - Each request is answered exactly once, whole: every byte asked for in pieces Windows
//!   accepts (`transfer::pieces`), or one failure for the whole range. A request said to be
//!   done before its last byte left a small file waiting for good (Nextcloud #10845).
//! - A long fetch from Apple reports progress, which keeps Windows' 60 s limit from cutting it.
//! - A folder's placeholders are made one per call: batches of them came back "done" and then
//!   would not open (ERROR_CLOUD_FILE_METADATA_CORRUPT, Microsoft Q&A 6057324).
//! - Names Windows can't hold are shown as their look-alikes (`named_on`), never dropped.
//! - Every callback is caught: a panic fails its request rather than taking Explorer's request,
//!   and Skykeep, down with it (Nextcloud #8608).
//! - Population is FULL (never PARTIAL, which Microsoft says is not supported), and a folder
//!   already listed is brought up to date when the library changes, never left stale (#10871).

use crate::{named_on, served_size_differs, transfer, Names, Remote, Store, CHUNK, ROOT};
use quince_core::drive::DriveItem;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{
    NTSTATUS, STATUS_CLOUD_FILE_NETWORK_UNAVAILABLE, STATUS_CLOUD_FILE_UNSUCCESSFUL,
    STATUS_SUCCESS,
};
use windows::Win32::Storage::CloudFilters::*;
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_BASIC_INFO,
};

/// What Explorer's folder is made from and fetched with, and where its requests are answered.
pub struct Provider {
    pub remote: Arc<dyn Remote>,
    pub store: Arc<dyn Store>,
    /// The app's async runtime: every request is answered there, never on Windows' thread.
    pub rt: tokio::runtime::Handle,
}

/// The folder, connected: its requests come in until it is dropped.
pub struct CloudFolder {
    key: CF_CONNECTION_KEY,
    provider: *const Provider,
    pub root: PathBuf,
    /// Registered with the shell (in Explorer's navigation pane), or as the bare filter.
    pub shell: bool,
}

// SAFETY: the connection key is a plain handle Windows hands out for any thread to use, and the
// provider behind the pointer is Send + Sync, owned here until the drop disconnects.
unsafe impl Send for CloudFolder {}
unsafe impl Sync for CloudFolder {}

impl Drop for CloudFolder {
    fn drop(&mut self) {
        // SAFETY: the key is this folder's; once disconnected no callback holds the provider.
        unsafe {
            let _ = CfDisconnectSyncRoot(self.key);
            drop(Box::from_raw(self.provider.cast_mut()));
        }
    }
}

/// How Explorer shows the folder: its name in the navigation pane, its icon, and the id it is
/// registered under (`Skykeep!<user's SID>!<account>`, at most 174 characters).
pub struct Shown<'a> {
    pub id: &'a str,
    pub name: &'a str,
    /// `C:\path\skykeep.exe,0`
    pub icon: &'a str,
}

/// Register `root` as a cloud folder (Explorer's navigation pane through the shell's own
/// registration, or the bare filter where that is refused) and connect to it.
pub fn start(root: &Path, shown: &Shown, provider: Provider) -> Result<CloudFolder, String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let shell = match register_with_shell(root, shown) {
        Ok(()) => true,
        Err(e) => {
            // Refused by the shell (an unpackaged copy, where Windows wants package identity):
            // the folder still works, without its entry in the navigation pane. The shell route
            // is tried again every start.
            eprintln!("quince: Explorer registration: {e}; registering the folder alone");
            register_bare(root, shown)?;
            false
        }
    };
    let mut folder = connect(root, provider)?;
    folder.shell = shell;
    Ok(folder)
}

/// The shell's registration: the folder in Explorer's navigation pane, with its name and icon,
/// "Free up space" and "Always keep on this device" on every file.
fn register_with_shell(root: &Path, shown: &Shown) -> Result<(), String> {
    use windows::Storage::Provider::*;
    use windows::Storage::StorageFolder;
    let e = |e: windows::core::Error| e.to_string();
    let folder = StorageFolder::GetFolderFromPathAsync(&HSTRING::from(root.as_os_str()))
        .and_then(|op| op.join())
        .map_err(e)?;
    let info = StorageProviderSyncRootInfo::new().map_err(e)?;
    info.SetId(&HSTRING::from(shown.id)).map_err(e)?;
    info.SetPath(&folder).map_err(e)?;
    info.SetDisplayNameResource(&HSTRING::from(shown.name)).map_err(e)?;
    info.SetIconResource(&HSTRING::from(shown.icon)).map_err(e)?;
    info.SetHydrationPolicy(StorageProviderHydrationPolicy::Full).map_err(e)?;
    info.SetHydrationPolicyModifier(StorageProviderHydrationPolicyModifier::None)
        .map_err(e)?;
    info.SetPopulationPolicy(StorageProviderPopulationPolicy::Full).map_err(e)?;
    info.SetInSyncPolicy(
        StorageProviderInSyncPolicy::FileCreationTime
            | StorageProviderInSyncPolicy::DirectoryCreationTime,
    )
    .map_err(e)?;
    info.SetHardlinkPolicy(StorageProviderHardlinkPolicy::None).map_err(e)?;
    info.SetShowSiblingsAsGroup(false).map_err(e)?;
    info.SetVersion(&HSTRING::from(env!("CARGO_PKG_VERSION"))).map_err(e)?;
    StorageProviderSyncRootManager::Register(&info).map_err(e)
}

/// The filter alone: the folder's files download on open, but it has no navigation-pane entry.
fn register_bare(root: &Path, shown: &Shown) -> Result<(), String> {
    let name = HSTRING::from(shown.name);
    let version = HSTRING::from(env!("CARGO_PKG_VERSION"));
    let identity = shown.id.as_bytes();
    let registration = CF_SYNC_REGISTRATION {
        StructSize: size_of::<CF_SYNC_REGISTRATION>() as u32,
        ProviderName: PCWSTR(name.as_ptr()),
        ProviderVersion: PCWSTR(version.as_ptr()),
        SyncRootIdentity: identity.as_ptr().cast(),
        SyncRootIdentityLength: identity.len() as u32,
        ..Default::default()
    };
    let policies = CF_SYNC_POLICIES {
        StructSize: size_of::<CF_SYNC_POLICIES>() as u32,
        Hydration: CF_HYDRATION_POLICY {
            Primary: CF_HYDRATION_POLICY_FULL,
            Modifier: CF_HYDRATION_POLICY_MODIFIER_NONE,
        },
        Population: CF_POPULATION_POLICY {
            Primary: CF_POPULATION_POLICY_FULL,
            Modifier: CF_POPULATION_POLICY_MODIFIER_NONE,
        },
        InSync: CF_INSYNC_POLICY_TRACK_ALL,
        HardLink: CF_HARDLINK_POLICY_NONE,
        PlaceholderManagement: CF_PLACEHOLDER_MANAGEMENT_POLICY_DEFAULT,
    };
    let path = HSTRING::from(root.as_os_str());
    // SAFETY: every pointer is to a value alive for the call.
    unsafe { CfRegisterSyncRoot(&path, &registration, &policies, CF_REGISTER_FLAG_UPDATE) }
        .map_err(|e| e.to_string())
}

fn connect(root: &Path, provider: Provider) -> Result<CloudFolder, String> {
    let table = [
        CF_CALLBACK_REGISTRATION {
            Type: CF_CALLBACK_TYPE_FETCH_DATA,
            Callback: Some(on_fetch_data),
        },
        CF_CALLBACK_REGISTRATION {
            Type: CF_CALLBACK_TYPE_FETCH_PLACEHOLDERS,
            Callback: Some(on_fetch_placeholders),
        },
        CF_CALLBACK_REGISTRATION {
            Type: CF_CALLBACK_TYPE_NONE,
            Callback: None,
        },
    ];
    let provider = Box::into_raw(Box::new(provider));
    let path = HSTRING::from(root.as_os_str());
    // Our own reads of a placeholder (an indexer of ours, a copy) never fetch it: blocked here,
    // they can't wait on themselves.
    let flags = CF_CONNECT_FLAG_BLOCK_SELF_IMPLICIT_HYDRATION | CF_CONNECT_FLAG_REQUIRE_FULL_FILE_PATH;
    // SAFETY: the table lives for the call (Windows copies it); the provider lives until the
    // CloudFolder's drop disconnects.
    match unsafe { CfConnectSyncRoot(&path, table.as_ptr(), Some(provider.cast()), flags) } {
        Ok(key) => Ok(CloudFolder {
            key,
            provider,
            root: root.to_path_buf(),
            shell: false,
        }),
        Err(e) => {
            // SAFETY: not handed to Windows after all.
            drop(unsafe { Box::from_raw(provider) });
            Err(e.to_string())
        }
    }
}

/// The id Windows wants a cloud folder registered under: `Skykeep!<this user's SID>!<account>`,
/// at most 174 characters (Microsoft's limit; longer is refused with ERROR_INSUFFICIENT_BUFFER).
pub fn sync_root_id(account: &str) -> Result<String, String> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    // SAFETY: the token is closed and the SID string freed before returning; the buffer is
    // sized by the first call.
    let sid = unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).map_err(|e| e.to_string())?;
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        let mut buf = vec![0u8; needed as usize];
        let got = GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr().cast()), needed, &mut needed);
        let _ = CloseHandle(token);
        got.map_err(|e| e.to_string())?;
        let user = &*buf.as_ptr().cast::<TOKEN_USER>();
        let mut text = windows::core::PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text).map_err(|e| e.to_string())?;
        let sid = text.to_string().map_err(|e| e.to_string());
        let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        sid?
    };
    // The account, as letters and digits only: `!` separates the parts.
    let account: String = account.chars().filter(char::is_ascii_alphanumeric).take(40).collect();
    let id = format!("Skykeep!{sid}!{account}");
    Ok(id.chars().take(174).collect())
}

/// Stop being the folder's provider for good: its placeholders go back to plain files the
/// shell no longer fetches. Only at sign-out or when the person turns the folder off.
pub fn unregister(root: &Path, id: &str) {
    let _ = windows::Storage::Provider::StorageProviderSyncRootManager::Unregister(&HSTRING::from(id));
    // SAFETY: a path string alive for the call.
    let _ = unsafe { CfUnregisterSyncRoot(&HSTRING::from(root.as_os_str())) };
}

/// What a request is for, copied out of Windows' callback so it can be answered elsewhere.
struct Request {
    connection: CF_CONNECTION_KEY,
    transfer: i64,
    request: i64,
    item: Option<DriveItem>,
    size: u64,
}

// SAFETY: plain values; the connection key is usable from any thread.
unsafe impl Send for Request {}

/// The callback's request, and our provider: read whole before the callback returns.
unsafe fn request(info: *const CF_CALLBACK_INFO) -> Option<(Request, &'static Provider)> {
    let info = unsafe { info.as_ref()? };
    let provider = unsafe { info.CallbackContext.cast::<Provider>().as_ref()? };
    let identity = if info.FileIdentity.is_null() || info.FileIdentityLength == 0 {
        None
    } else {
        // SAFETY: Windows gives the identity we stored, for the length it says.
        let bytes = unsafe {
            std::slice::from_raw_parts(info.FileIdentity.cast::<u8>(), info.FileIdentityLength as usize)
        };
        serde_json::from_slice::<DriveItem>(bytes).ok()
    };
    Some((
        Request {
            connection: info.ConnectionKey,
            transfer: info.TransferKey,
            request: info.RequestKey,
            item: identity,
            size: u64::try_from(info.FileSize).unwrap_or(0),
        },
        provider,
    ))
}

unsafe extern "system" fn on_fetch_data(info: *const CF_CALLBACK_INFO, params: *const CF_CALLBACK_PARAMETERS) {
    let caught = std::panic::catch_unwind(|| {
        // SAFETY: Windows' pointers, valid for this call; everything needed is copied out.
        let Some((req, provider)) = (unsafe { request(info) }) else { return };
        let fetch = unsafe { (*params).Anonymous.FetchData };
        let offset = u64::try_from(fetch.RequiredFileOffset).unwrap_or(0);
        let len = u64::try_from(fetch.RequiredLength).unwrap_or(0);
        let (remote, rt) = (Arc::clone(&provider.remote), provider.rt.clone());
        rt.spawn(async move { fetch_data(remote.as_ref(), req, offset, len).await });
    });
    if caught.is_err() {
        eprintln!("quince: a file request failed inside Skykeep; it is answered as failed");
    }
}

/// A file's bytes, `len` from `offset`, from Apple to Windows: every piece in order, then done;
/// or the whole range failed, once.
async fn fetch_data(remote: &dyn Remote, req: Request, offset: u64, len: u64) {
    let Some(item) = req.item.clone() else {
        fail_data(&req, offset, len, STATUS_CLOUD_FILE_UNSUCCESSFUL);
        return;
    };
    let pieces = transfer::pieces(req.size, offset, len, CHUNK);
    let Some(&(start, _)) = pieces.first() else {
        fail_data(&req, offset, len, STATUS_CLOUD_FILE_UNSUCCESSFUL);
        return;
    };
    let end = pieces.last().map_or(start, |&(at, n)| at + n);
    for (at, n) in pieces {
        // Each piece read takes seconds on a slow line: progress keeps Windows' 60 s limit off.
        report(&req, end - start, at - start);
        let bytes = if n == 0 {
            Vec::new()
        } else {
            match remote.read_part(&item, at, n).await {
                Ok(part) if part.bytes.len() as u64 == n => part.bytes,
                Ok(part) => {
                    eprintln!("quince: {} answered {} of {n} bytes at {at}", item.drivewsid, part.bytes.len());
                    fail_data(&req, at, end - at, STATUS_CLOUD_FILE_UNSUCCESSFUL);
                    return;
                }
                Err(e) => {
                    eprintln!("quince: fetching {}: {e}", item.drivewsid);
                    fail_data(&req, at, end - at, STATUS_CLOUD_FILE_NETWORK_UNAVAILABLE);
                    return;
                }
            }
        };
        if let Err(e) = transfer_data(&req, at, &bytes, STATUS_SUCCESS) {
            // Windows took the request back (the reader closed it, it timed out): nothing more
            // is sent for it.
            eprintln!("quince: handing {} to Windows: {e}", item.drivewsid);
            return;
        }
    }
}

fn report(req: &Request, total: u64, done: u64) {
    // SAFETY: a plain call with keys Windows gave us.
    let _ = unsafe {
        CfReportProviderProgress(req.connection, req.transfer, total as i64, done as i64)
    };
}

/// One piece handed to Windows, or with a failure status the range it fails.
fn transfer_data(req: &Request, offset: u64, bytes: &[u8], status: NTSTATUS) -> windows::core::Result<()> {
    transfer_range(req, offset, bytes.len() as u64, bytes.as_ptr().cast(), status)
}

fn fail_data(req: &Request, offset: u64, len: u64, status: NTSTATUS) {
    // Failed whole, over the very range the pieces would have covered: a short or empty failure
    // can be refused, leaving the reader waiting out the timeout (cloud-filter-rs notes).
    let whole = u64::MAX / transfer::BLOCK * transfer::BLOCK;
    let (start, n) = transfer::pieces(req.size, offset, len, whole)
        .first()
        .copied()
        .unwrap_or((offset / transfer::BLOCK * transfer::BLOCK, transfer::BLOCK));
    let _ = transfer_range(req, start, n, std::ptr::null(), status);
}

fn transfer_range(req: &Request, offset: u64, len: u64, buffer: *const c_void, status: NTSTATUS) -> windows::core::Result<()> {
    let info = CF_OPERATION_INFO {
        StructSize: size_of::<CF_OPERATION_INFO>() as u32,
        Type: CF_OPERATION_TYPE_TRANSFER_DATA,
        ConnectionKey: req.connection,
        TransferKey: req.transfer,
        RequestKey: req.request,
        ..Default::default()
    };
    let mut params = CF_OPERATION_PARAMETERS {
        ParamSize: size_of::<CF_OPERATION_PARAMETERS>() as u32,
        Anonymous: CF_OPERATION_PARAMETERS_0 {
            TransferData: CF_OPERATION_PARAMETERS_0_0 {
                Flags: CF_OPERATION_TRANSFER_DATA_FLAG_NONE,
                CompletionStatus: status,
                Buffer: buffer,
                Offset: offset as i64,
                Length: len as i64,
            },
        },
    };
    // SAFETY: the buffer (or null, on failure) is alive for the call.
    unsafe { CfExecute(&info, &mut params) }
}

unsafe extern "system" fn on_fetch_placeholders(info: *const CF_CALLBACK_INFO, _params: *const CF_CALLBACK_PARAMETERS) {
    let caught = std::panic::catch_unwind(|| {
        // SAFETY: as in on_fetch_data.
        let Some((req, provider)) = (unsafe { request(info) }) else { return };
        let (remote, store, rt) = (Arc::clone(&provider.remote), Arc::clone(&provider.store), provider.rt.clone());
        rt.spawn(async move { fetch_placeholders(remote.as_ref(), store.as_ref(), req).await });
    });
    if caught.is_err() {
        eprintln!("quince: a folder request failed inside Skykeep; it is answered as failed");
    }
}

/// A folder's items as placeholders, from the library (from Apple first when the folder was
/// never listed), one per call, the last marking the folder whole.
async fn fetch_placeholders(remote: &dyn Remote, store: &dyn Store, req: Request) {
    let folder = req.item.as_ref().map_or(ROOT.to_string(), |i| i.drivewsid.clone());
    let items = match store.children(&folder) {
        Some(items) => items,
        None => match remote.list(&folder).await {
            Ok(items) => items,
            Err(e) => {
                eprintln!("quince: listing {folder}: {e}");
                let _ = transfer_placeholders(&req, &mut [], 0, true, STATUS_CLOUD_FILE_NETWORK_UNAVAILABLE);
                return;
            }
        },
    };
    let named = named_on(&items, Names::Windows);
    let total = named.len() as i64;
    if named.is_empty() {
        let _ = transfer_placeholders(&req, &mut [], 0, true, STATUS_SUCCESS);
        return;
    }
    let last = named.len() - 1;
    for (i, (name, item)) in named.into_iter().enumerate() {
        let size = placeholder_size(remote, &item).await;
        let made = Placeholder::of(&name, &item, size);
        let mut one = [made.info()];
        if let Err(e) = transfer_placeholders(&req, &mut one, total, i == last, STATUS_SUCCESS) {
            eprintln!("quince: {name} in {folder}: {e}");
            return;
        }
    }
}

/// The size a placeholder is made at: the listing's, except where Apple serves another (Pages,
/// Numbers and Keynote packages), which is asked once — a placeholder at the wrong size gives
/// the reader a broken file.
async fn placeholder_size(remote: &dyn Remote, item: &DriveItem) -> u64 {
    let listed = u64::try_from(item.size.unwrap_or(0)).unwrap_or(0);
    if !served_size_differs(item) {
        return listed;
    }
    match remote.read_part(item, 0, 1).await {
        Ok(part) => part.total.unwrap_or(listed),
        Err(_) => listed,
    }
}

fn transfer_placeholders(
    req: &Request,
    array: &mut [CF_PLACEHOLDER_CREATE_INFO],
    total: i64,
    last: bool,
    status: NTSTATUS,
) -> windows::core::Result<()> {
    let info = CF_OPERATION_INFO {
        StructSize: size_of::<CF_OPERATION_INFO>() as u32,
        Type: CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS,
        ConnectionKey: req.connection,
        TransferKey: req.transfer,
        RequestKey: req.request,
        ..Default::default()
    };
    let mut params = CF_OPERATION_PARAMETERS {
        ParamSize: size_of::<CF_OPERATION_PARAMETERS>() as u32,
        Anonymous: CF_OPERATION_PARAMETERS_0 {
            TransferPlaceholders: CF_OPERATION_PARAMETERS_0_4 {
                // The last says the folder is whole: no more requests for it, its updates come
                // from us (`refresh`).
                Flags: if last {
                    CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAG_DISABLE_ON_DEMAND_POPULATION
                } else {
                    CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAG_NONE
                },
                CompletionStatus: status,
                PlaceholderTotalCount: total,
                PlaceholderArray: array.as_mut_ptr(),
                PlaceholderCount: array.len() as u32,
                EntriesProcessed: 0,
            },
        },
    };
    // SAFETY: the array and the names and identities it points at are alive for the call.
    unsafe { CfExecute(&info, &mut params) }
}

/// One placeholder's name, identity and metadata, kept alive while Windows reads them.
struct Placeholder {
    name: Vec<u16>,
    identity: Vec<u8>,
    meta: CF_FS_METADATA,
}

impl Placeholder {
    fn of(name: &str, item: &DriveItem, size: u64) -> Placeholder {
        let folder = item.is_container();
        let time = |secs: Option<i64>| secs.map_or(0, filetime);
        let modified = time(item.modified_at.or(item.created_at));
        Placeholder {
            name: name.encode_utf16().chain(Some(0)).collect(),
            // The item itself: a request for it is answered from this, with no lookup.
            identity: serde_json::to_vec(item).unwrap_or_default(),
            meta: CF_FS_METADATA {
                BasicInfo: FILE_BASIC_INFO {
                    CreationTime: time(item.created_at.or(item.modified_at)),
                    LastAccessTime: modified,
                    LastWriteTime: modified,
                    ChangeTime: modified,
                    FileAttributes: if folder { FILE_ATTRIBUTE_DIRECTORY.0 } else { FILE_ATTRIBUTE_NORMAL.0 },
                },
                FileSize: if folder { 0 } else { size as i64 },
            },

        }
    }

    fn info(&self) -> CF_PLACEHOLDER_CREATE_INFO {
        CF_PLACEHOLDER_CREATE_INFO {
            RelativeFileName: PCWSTR(self.name.as_ptr()),
            FsMetadata: self.meta,
            FileIdentity: self.identity.as_ptr().cast(),
            FileIdentityLength: self.identity.len() as u32,
            // In sync as made. A folder is filled when it is first opened (FETCH_PLACEHOLDERS),
            // so it is never made as already populated.
            Flags: CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC,
            ..Default::default()
        }
    }
}

/// Seconds since 1970 as Windows' FILETIME: 100 ns ticks since 1601.
fn filetime(secs: i64) -> i64 {
    (secs + 11_644_473_600).saturating_mul(10_000_000)
}
