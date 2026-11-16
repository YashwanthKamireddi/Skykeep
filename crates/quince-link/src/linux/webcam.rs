//! The iPhone as a classic webcam device (`/dev/videoN`), for programs that only look there:
//! Chrome and Chromium with their default settings, Zoom, Discord, Teams.
//!
//! The device is v4l2loopback's, made once by the person in Settings (one admin prompt; see
//! src-tauri/src/iphone.rs) and named "iPhone Camera". Skykeep writes each picture into it as
//! YUYV, the format every webcam program reads, and PipeWire lists it for the rest, so there is
//! one iPhone camera everywhere. `exclusive_caps` makes it a capture device only while a
//! picture format is set, which is what Chrome asks of a camera.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use super::decode::Picture;

/// The name the device is made with, and how Skykeep finds it again.
pub const CARD: &str = "iPhone Camera";

const VIDIOC_QUERYCAP: libc::c_ulong = 0x8068_5600;
const VIDIOC_S_FMT: libc::c_ulong = 0xc0d0_5605;
const VIDIOC_S_PARM: libc::c_ulong = 0xc0cc_5616;
const BUF_TYPE_VIDEO_OUTPUT: u32 = 2;
const FIELD_NONE: u32 = 1;
const COLORSPACE_REC709: u32 = 3;
const YUYV: u32 = u32::from_le_bytes(*b"YUYV");

#[repr(C)]
struct Capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus_info: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
}

#[repr(C)]
struct PixFormat {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    bytesperline: u32,
    sizeimage: u32,
    colorspace: u32,
    private: u32,
    flags: u32,
    ycbcr_enc: u32,
    quantization: u32,
    xfer_func: u32,
}

/// `struct v4l2_format`: the type, then a 200-byte union aligned to 8.
#[repr(C)]
struct Format {
    kind: u32,
    _pad: u32,
    pix: PixFormat,
    _rest: [u8; 200 - std::mem::size_of::<PixFormat>()],
}

/// `struct v4l2_streamparm` for an output: the type, then `v4l2_outputparm` in a 200-byte union.
#[repr(C)]
struct StreamParm {
    kind: u32,
    capability: u32,
    outputmode: u32,
    numerator: u32,
    denominator: u32,
    extendedmode: u32,
    writebuffers: u32,
    reserved: [u32; 4],
    _rest: [u8; 200 - 40],
}

/// The loopback device made for Skykeep, if there is one.
pub fn find() -> Option<PathBuf> {
    let entries = std::fs::read_dir("/sys/class/video4linux").ok()?;
    entries.flatten().find_map(|e| {
        let name = std::fs::read_to_string(e.path().join("name")).ok()?;
        (name.trim() == CARD).then(|| PathBuf::from("/dev").join(e.file_name()))
    })
}

pub struct Webcam {
    file: File,
    width: usize,
    height: usize,
    yuyv: Vec<u8>,
}

impl Webcam {
    /// Opens the device and sets the picture size: from here on, programs see a camera.
    pub fn open(path: &std::path::Path, width: u32, height: u32, fps: u32) -> Result<Webcam> {
        let file = OpenOptions::new()
            .write(true)
            .open(path)
            .with_context(|| format!("can't open {}", path.display()))?;
        let fd = file.as_raw_fd();
        // SAFETY: both structs match the kernel's layout and live for the call.
        unsafe {
            let mut cap: Capability = std::mem::zeroed();
            if libc::ioctl(fd, VIDIOC_QUERYCAP, &mut cap) < 0 {
                bail!("{} is not a video device", path.display());
            }
            let mut f: Format = std::mem::zeroed();
            f.kind = BUF_TYPE_VIDEO_OUTPUT;
            f.pix = PixFormat {
                width,
                height,
                pixelformat: YUYV,
                field: FIELD_NONE,
                bytesperline: width * 2,
                sizeimage: width * height * 2,
                colorspace: COLORSPACE_REC709,
                private: 0,
                flags: 0,
                ycbcr_enc: 0,
                quantization: 0,
                xfer_func: 0,
            };
            if libc::ioctl(fd, VIDIOC_S_FMT, &mut f) < 0 {
                bail!(
                    "{} refused a {width}x{height} picture: {}",
                    path.display(),
                    std::io::Error::last_os_error()
                );
            }
            // The rate programs are offered: the phone's own, not the device's default 30.
            let mut p: StreamParm = std::mem::zeroed();
            p.kind = BUF_TYPE_VIDEO_OUTPUT;
            p.numerator = 1;
            p.denominator = fps;
            let _ = libc::ioctl(fd, VIDIOC_S_PARM, &mut p);
        }
        let (w, h) = (width as usize, height as usize);
        Ok(Webcam {
            file,
            width: w,
            height: h,
            yuyv: vec![0; w * h * 2],
        })
    }

    pub fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Writes one picture. A program reading slower than pictures come gets the newest.
    pub fn show(&mut self, picture: &Picture) -> Result<()> {
        if picture.width as usize != self.width || picture.height as usize != self.height {
            return Ok(());
        }
        nv12_to_yuyv(&picture.nv12, self.width, self.height, &mut self.yuyv);
        self.file.write_all(&self.yuyv)?;
        Ok(())
    }
}

/// NV12 (full brightness plane, half-size interleaved colour plane) to YUYV (two pixels'
/// brightness sharing one colour pair, row by row).
fn nv12_to_yuyv(nv12: &[u8], w: usize, h: usize, out: &mut [u8]) {
    let (luma, chroma) = nv12.split_at(w * h);
    for y in 0..h {
        let row_y = &luma[y * w..y * w + w];
        let row_c = &chroma[(y / 2) * w..(y / 2) * w + w];
        let row_o = &mut out[y * w * 2..y * w * 2 + w * 2];
        for x in (0..w).step_by(2) {
            let o = &mut row_o[x * 2..x * 2 + 4];
            o[0] = row_y[x];
            o[1] = row_c[x];
            o[2] = row_y[x + 1];
            o[3] = row_c[x + 1];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nv12_becomes_yuyv_with_each_pair_of_pixels_sharing_its_colour() {
        // 2x2: brightness 10 20 / 30 40, one colour pair (u 100, v 200).
        let nv12 = [10, 20, 30, 40, 100, 200];
        let mut out = [0u8; 8];
        nv12_to_yuyv(&nv12, 2, 2, &mut out);
        assert_eq!(out, [10, 100, 20, 200, 30, 100, 40, 200]);
    }

    #[test]
    fn the_kernel_structs_have_the_kernels_sizes() {
        assert_eq!(std::mem::size_of::<Capability>(), 104);
        assert_eq!(std::mem::size_of::<Format>(), 208);
        assert_eq!(std::mem::size_of::<StreamParm>(), 204);
    }
}
