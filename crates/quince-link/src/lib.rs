//! The iPhone as this computer's camera and microphone.
//!
//! The phone app sends what its camera and microphone capture over the USB cable (or Wi-Fi);
//! this crate receives it ([`receiver`]), decodes the pictures and publishes both as devices
//! every program can choose: on Linux, a PipeWire camera and a PipeWire microphone.

pub mod protocol;
pub mod sps;
pub mod stats;
pub mod usbmux;

#[cfg(all(target_os = "linux", feature = "fakephone"))]
pub mod fakephone;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub mod receiver;
#[cfg(target_os = "linux")]
pub mod service;
