//! Thumbnails in File Explorer's iCloud Photos folder for photos not on this computer yet, as
//! OneDrive shows them: Explorer asks this DLL, the DLL asks Skykeep over a local pipe, and
//! Skykeep answers with the photo's small picture it already keeps (or fetches just that, a few
//! kilobytes, never the photo), ready to draw.
//!
//! This code runs inside Explorer, so it does as little as it can: no decoding, no network, no
//! panics (every entry point is caught), and every wait is bounded. With Skykeep closed the
//! pipe is not there, the answer is "none" at once, and Windows draws its usual icon.
//!
//! The pipe's name and messages are here too, for Skykeep's side ([`protocol`]).

pub mod protocol {
    //! The pipe between this DLL and Skykeep. A request: the size Explorer wants (`u32`), then
    //! the file's path as UTF-16 (`u32` count, then the units). A reply: width and height
    //! (`u32` each; 0 for "none"), then `width × height` pixels of four bytes, blue first, top
    //! row first. Little-endian throughout.

    /// The largest side either end will make or take: Explorer asks for at most 2560.
    pub const MAX_SIDE: u32 = 2560;
    /// The longest path accepted, in UTF-16 units: Windows' own longest.
    pub const MAX_PATH_UNITS: u32 = 32_767;

    /// The pipe for a Windows session: one per signed-in person, never another's.
    pub fn pipe_name(session: u32) -> String {
        format!(r"\\.\pipe\Skykeep-thumbnails-{session}")
    }

    pub fn request(size: u32, path: &[u16]) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + path.len() * 2);
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(path.len() as u32).to_le_bytes());
        for u in path {
            out.extend_from_slice(&u.to_le_bytes());
        }
        out
    }

    /// The request's head: (size, how many UTF-16 units of path follow). `None` past the limits.
    pub fn request_head(head: [u8; 8]) -> Option<(u32, u32)> {
        let size = u32::from_le_bytes(head[..4].try_into().ok()?);
        let units = u32::from_le_bytes(head[4..].try_into().ok()?);
        (size > 0 && size <= MAX_SIDE && units > 0 && units <= MAX_PATH_UNITS)
            .then_some((size, units))
    }

    pub fn path_of(body: &[u8]) -> Vec<u16> {
        body.chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect()
    }

    /// A reply carrying a picture, or "none" (`None`, or one that breaks the limits).
    pub fn reply(picture: Option<(u32, u32, &[u8])>) -> Vec<u8> {
        match picture {
            Some((w, h, px)) if fits(w, h) && px.len() == pixel_bytes(w, h) => {
                let mut out = Vec::with_capacity(8 + px.len());
                out.extend_from_slice(&w.to_le_bytes());
                out.extend_from_slice(&h.to_le_bytes());
                out.extend_from_slice(px);
                out
            }
            _ => [0u8; 8].to_vec(),
        }
    }

    /// The reply's head: (width, height) of the pixels that follow; `None` for "none".
    pub fn reply_head(head: [u8; 8]) -> Option<(u32, u32)> {
        let w = u32::from_le_bytes(head[..4].try_into().ok()?);
        let h = u32::from_le_bytes(head[4..].try_into().ok()?);
        fits(w, h).then_some((w, h))
    }

    pub fn pixel_bytes(w: u32, h: u32) -> usize {
        w as usize * h as usize * 4
    }

    fn fits(w: u32, h: u32) -> bool {
        (1..=MAX_SIDE).contains(&w) && (1..=MAX_SIDE).contains(&h)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_request_and_reply_read_back_as_written() {
            let path: Vec<u16> = r"C:\Users\a\iCloud Photos\IMG_1.HEIC"
                .encode_utf16()
                .collect();
            let r = request(256, &path);
            let (size, units) = request_head(r[..8].try_into().unwrap()).unwrap();
            assert_eq!((size, units as usize), (256, path.len()));
            assert_eq!(path_of(&r[8..]), path);

            let px = vec![7u8; pixel_bytes(3, 2)];
            let r = reply(Some((3, 2, &px)));
            assert_eq!(reply_head(r[..8].try_into().unwrap()), Some((3, 2)));
            assert_eq!(&r[8..], &px[..]);
        }

        #[test]
        fn anything_past_the_limits_is_none() {
            assert_eq!(
                request_head(request(0, &[1])[..8].try_into().unwrap()),
                None
            );
            assert_eq!(
                request_head(request(4000, &[1])[..8].try_into().unwrap()),
                None
            );
            assert_eq!(
                request_head(request(96, &[])[..8].try_into().unwrap()),
                None
            );
            assert_eq!(reply(None), vec![0; 8]);
            assert_eq!(reply(Some((2, 2, &[0; 3]))), vec![0; 8], "short pixels");
            assert_eq!(reply_head([0; 8]), None);
        }
    }
}

#[cfg(windows)]
mod provider;

#[cfg(windows)]
pub use provider::{class_factory, CLSID};

/// Who may use the pipe, for Skykeep's side to create it with: this user (at any integrity
/// level: Windows runs thumbnail providers in a low-integrity process, which the default pipe
/// security shuts out) and the system. Nobody else on the computer.
#[cfg(windows)]
pub struct PipeSecurity {
    descriptor: windows::Win32::Security::PSECURITY_DESCRIPTOR,
    attributes: windows::Win32::Security::SECURITY_ATTRIBUTES,
}

#[cfg(windows)]
impl PipeSecurity {
    pub fn new() -> Option<Self> {
        use windows::Win32::Security::Authorization::*;
        use windows::Win32::Security::*;
        let user = this_user()?;
        // Full access for this user and the system, nothing for anyone else (`P`: nothing
        // inherited), and a low integrity label that lets low-integrity processes write.
        let sddl = format!("D:P(A;;GA;;;{user})(A;;GA;;;SY)S:(ML;;NW;;;LW)");
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: a string alive for the call; the descriptor is ours, freed in `drop`.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &windows_core::HSTRING::from(sddl),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .ok()?;
        Some(PipeSecurity {
            descriptor,
            attributes: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.0,
                bInheritHandle: false.into(),
            },
        })
    }

    /// For `CreateNamedPipe`'s security attributes: valid while `self` lives.
    pub fn as_raw(&mut self) -> *mut std::ffi::c_void {
        std::ptr::addr_of_mut!(self.attributes).cast()
    }
}

#[cfg(windows)]
impl Drop for PipeSecurity {
    fn drop(&mut self) {
        use windows::Win32::Foundation::{LocalFree, HLOCAL};
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW, freed once.
        unsafe { LocalFree(Some(HLOCAL(self.descriptor.0))) };
    }
}

/// This process's user, as a SID string (`S-1-5-21-…`).
#[cfg(windows)]
fn this_user() -> Option<String> {
    use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::*;
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let mut token = HANDLE::default();
    // SAFETY: our own process; the token is closed below.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.ok()?;
    let mut needed = 0u32;
    // SAFETY: asked for its size first, then read into a buffer of that size.
    unsafe {
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
        let read = GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        );
        let _ = CloseHandle(token);
        read.ok()?;
        let sid = (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid;
        let mut text = windows_core::PWSTR::null();
        ConvertSidToStringSidW(sid, &mut text).ok()?;
        let user = text.to_string().ok();
        LocalFree(Some(HLOCAL(text.0.cast())));
        user
    }
}

/// This process's Windows session: the pipe's name is made from it on both sides.
#[cfg(windows)]
pub fn this_session() -> Option<u32> {
    use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    use windows::Win32::System::Threading::GetCurrentProcessId;
    let mut session = 0u32;
    // SAFETY: our own process id and an out value on the stack.
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) }.ok()?;
    Some(session)
}
