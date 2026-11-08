//! The COM class Explorer creates, one per thumbnail: told the file (`IInitializeWithItem`, the
//! interface a cloud folder's provider must use), then asked for its picture (`IThumbnailProvider`).

use crate::protocol;
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Pipes::WaitNamedPipeW;
use windows::Win32::System::Threading::CreateEventW;
use windows::Win32::System::IO::{
    CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED,
};
use windows::Win32::UI::Shell::*;
use windows_core::{implement, IUnknown, Interface, Ref, BOOL, GUID, HRESULT, HSTRING};

/// The provider's class, named in the registry for the iCloud Photos folder only.
pub const CLSID: GUID = GUID::from_u128(0x4ee8b6ba_8522_41f1_a17e_19126a88c38e);

/// How long Explorer's thread may wait on Skykeep, all told: a picture already kept answers in
/// milliseconds; one fetched from Apple may take a few seconds; past this, no thumbnail.
const PATIENCE_MS: u32 = 8_000;

/// What the provider did, step by step, appended to `trace.txt` in a folder of that name under
/// `AppData\LocalLow\Skykeep thumbnails trace` only when that folder exists: off everywhere
/// except where a test made it (LocalLow, so a low-integrity host can write it too).
fn trace(line: &str) {
    use std::io::Write;
    let Some(home) = std::env::var_os("USERPROFILE") else {
        return;
    };
    let dir = std::path::Path::new(&home).join(r"AppData\LocalLow\Skykeep thumbnails trace");
    if !dir.is_dir() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("trace.txt"))
    {
        let _ = writeln!(f, "{} {line}", std::process::id());
    }
}

/// Objects alive, for `DllCanUnloadNow`.
static LIVE: AtomicUsize = AtomicUsize::new(0);

#[implement(IThumbnailProvider, IInitializeWithItem)]
struct Provider {
    path: Mutex<Option<Vec<u16>>>,
}

impl Provider {
    fn new() -> Self {
        LIVE.fetch_add(1, Ordering::SeqCst);
        Provider {
            path: Mutex::new(None),
        }
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

impl IInitializeWithItem_Impl for Provider_Impl {
    fn Initialize(&self, item: Ref<IShellItem>, _mode: u32) -> windows_core::Result<()> {
        caught(|| {
            trace("initialize");
            let item = item.ok()?;
            // SAFETY: a live shell item; the string is ours to free, and freed once read.
            let path = unsafe {
                let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
                let path = name.as_wide().to_vec();
                CoTaskMemFree(Some(name.0 as *const c_void));
                path
            };
            trace(&format!("initialized: {}", String::from_utf16_lossy(&path)));
            *self.path.lock().map_err(|_| E_UNEXPECTED)? = Some(path);
            Ok(())
        })
    }
}

impl IThumbnailProvider_Impl for Provider_Impl {
    fn GetThumbnail(
        &self,
        cx: u32,
        bitmap: *mut HBITMAP,
        alpha: *mut WTS_ALPHATYPE,
    ) -> windows_core::Result<()> {
        caught(|| {
            if bitmap.is_null() || alpha.is_null() {
                return Err(E_POINTER.into());
            }
            let path = self
                .path
                .lock()
                .map_err(|_| E_UNEXPECTED)?
                .clone()
                .ok_or(E_UNEXPECTED)?;
            trace(&format!("thumbnail asked at {cx}"));
            let (w, h, pixels) = ask(&path, cx.clamp(1, protocol::MAX_SIDE)).ok_or(E_FAIL)?;
            trace(&format!("answered {w}x{h}"));
            let made = drawn(w, h, &pixels)?;
            // SAFETY: both checked non-null above; Explorer owns the bitmap from here.
            unsafe {
                *bitmap = made;
                *alpha = WTSAT_RGB;
            }
            Ok(())
        })
    }
}

/// Never a panic across into Explorer: one becomes a failed call, and Explorer draws its icon.
fn caught(f: impl FnOnce() -> windows_core::Result<()>) -> windows_core::Result<()> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err(E_UNEXPECTED.into()))
}

/// Skykeep's answer for `path` at `size`: (width, height, pixels), or `None` when Skykeep is
/// closed, has no picture for it, or doesn't answer in time.
fn ask(path: &[u16], size: u32) -> Option<(u32, u32, Vec<u8>)> {
    let name = HSTRING::from(protocol::pipe_name(crate::this_session()?));
    // Busy with another thumbnail: wait a little for it, never long. Closed: false at once.
    // SAFETY: a name string alive for the call.
    if !unsafe { WaitNamedPipeW(&name, 1_000) }.as_bool() {
        trace(&format!(
            "no pipe: {:?}",
            windows_core::Error::from_thread()
        ));
        return None;
    }
    // SAFETY: as above; the handle is closed by `Pipe`'s drop.
    let pipe = Pipe(
        unsafe {
            CreateFileW(
                &name,
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )
        }
        .map_err(|e| trace(&format!("pipe refused: {e:?}")))
        .ok()?,
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(PATIENCE_MS.into());
    pipe.write_all(&protocol::request(size, path), deadline)?;
    let mut head = [0u8; 8];
    pipe.read_exact(&mut head, deadline)?;
    let (w, h) = protocol::reply_head(head)?;
    let mut pixels = vec![0u8; protocol::pixel_bytes(w, h)];
    pipe.read_exact(&mut pixels, deadline)?;
    Some((w, h, pixels))
}

struct Pipe(HANDLE);

impl Drop for Pipe {
    fn drop(&mut self) {
        // SAFETY: ours, closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

impl Pipe {
    fn write_all(&self, mut bytes: &[u8], deadline: std::time::Instant) -> Option<()> {
        while !bytes.is_empty() {
            let n = self.io(Io::Write(bytes), deadline)?;
            bytes = bytes.get(n..)?;
        }
        Some(())
    }

    fn read_exact(&self, mut into: &mut [u8], deadline: std::time::Instant) -> Option<()> {
        while !into.is_empty() {
            let n = self.io(Io::Read(&mut *into), deadline)?;
            into = into.get_mut(n..)?;
        }
        Some(())
    }

    /// One overlapped read or write, waited on only until `deadline`; past it, cancelled and
    /// waited out (the buffer must outlive the operation), and `None`.
    fn io(&self, op: Io<'_>, deadline: std::time::Instant) -> Option<usize> {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let ms = u32::try_from(left.as_millis()).unwrap_or(u32::MAX);
        if ms == 0 {
            return None;
        }
        // SAFETY: an unnamed manual-reset event, closed below.
        let event = unsafe { CreateEventW(None, true, false, None) }.ok()?;
        let event = Pipe(event);
        let mut ov = OVERLAPPED {
            hEvent: event.0,
            ..Default::default()
        };
        // SAFETY: the buffer and `ov` live until the operation is finished or cancelled below.
        let started = unsafe {
            match op {
                Io::Write(b) => WriteFile(self.0, Some(b), None, Some(&mut ov)),
                Io::Read(b) => ReadFile(self.0, Some(b), None, Some(&mut ov)),
            }
        };
        if let Err(e) = started {
            if e.code() != ERROR_IO_PENDING.to_hresult() {
                return None;
            }
        }
        let mut done = 0u32;
        // SAFETY: the operation started above on our handle with this `ov`.
        if unsafe { GetOverlappedResultEx(self.0, &ov, &mut done, ms, false) }.is_err() {
            // SAFETY: as above; then waited for, so nothing writes into the buffer after.
            unsafe {
                let _ = CancelIoEx(self.0, Some(&ov));
                let _ = GetOverlappedResult(self.0, &ov, &mut done, true);
            }
            return None;
        }
        (done > 0).then_some(done as usize)
    }
}

enum Io<'a> {
    Write(&'a [u8]),
    Read(&'a mut [u8]),
}

/// The pixels as a bitmap Explorer can draw: 32-bit, top row first.
fn drawn(w: u32, h: u32, pixels: &[u8]) -> windows_core::Result<HBITMAP> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = std::ptr::null_mut();
    // SAFETY: a valid header; Windows allocates the bits, sized by it.
    let made = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }?;
    if bits.is_null() {
        // SAFETY: the bitmap just made, never handed out.
        let _ = unsafe { DeleteObject(made.into()) };
        return Err(E_OUTOFMEMORY.into());
    }
    // SAFETY: `bits` holds exactly w × h × 4 bytes, the length `pixels` was read at.
    unsafe { std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len()) };
    Ok(made)
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        iid: *const GUID,
        object: *mut *mut c_void,
    ) -> windows_core::Result<()> {
        caught(|| {
            if object.is_null() || iid.is_null() {
                return Err(E_POINTER.into());
            }
            // SAFETY: checked non-null.
            unsafe { *object = std::ptr::null_mut() };
            if outer.is_some() {
                return Err(CLASS_E_NOAGGREGATION.into());
            }
            let made: IUnknown = Provider::new().into();
            // SAFETY: iid and object checked non-null; `query` fills `object` on success.
            unsafe { made.query(iid, object) }.ok()
        })
    }

    fn LockServer(&self, _lock: BOOL) -> windows_core::Result<()> {
        Ok(())
    }
}

/// COM's way in: the class factory, for our class only.
///
/// # Safety
/// Called by COM with valid pointers.
#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(
    class: *const GUID,
    iid: *const GUID,
    object: *mut *mut c_void,
) -> HRESULT {
    std::panic::catch_unwind(|| {
        trace("class asked");
        if class.is_null() || iid.is_null() || object.is_null() {
            return E_POINTER;
        }
        // SAFETY: checked non-null; COM's pointers.
        unsafe {
            *object = std::ptr::null_mut();
            if *class != CLSID {
                return CLASS_E_CLASSNOTAVAILABLE;
            }
            let factory: IClassFactory = Factory.into();
            factory.query(iid, object)
        }
    })
    .unwrap_or(E_UNEXPECTED)
}

/// Whether Explorer may unload the DLL: only with none of our objects alive.
#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if LIVE.load(Ordering::SeqCst) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}
