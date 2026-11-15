//! The USB cable to the iPhone, through usbmuxd: the service every Linux distribution ships
//! for iPhones (and Apple's own on Windows, on port 27015), which tunnels a TCP connection to
//! a port on the phone's loopback address.
//!
//! Its protocol is a 16-byte little-endian header (total length, version 1, message type 8 =
//! property list, tag) and an XML property list. After a successful `Connect`, the same socket
//! carries the phone's bytes.

use std::io::{Read, Write};

use anyhow::{bail, Context, Result};
use plist::{Dictionary, Value};

#[cfg(unix)]
pub type Socket = std::os::unix::net::UnixStream;
#[cfg(windows)]
pub type Socket = std::net::TcpStream;

fn open() -> Result<Socket> {
    #[cfg(unix)]
    {
        let path =
            std::env::var("USBMUXD_SOCKET_ADDRESS").unwrap_or_else(|_| "/var/run/usbmuxd".into());
        Socket::connect(&path).with_context(|| {
            format!("usbmuxd isn't running ({path}); it comes with the usbmuxd package")
        })
    }
    #[cfg(windows)]
    {
        Socket::connect(("127.0.0.1", 27015)).context(
            "Apple's device service isn't running: install Apple Devices from the Microsoft Store",
        )
    }
}

fn send(socket: &mut Socket, message: Dictionary, tag: u32) -> Result<()> {
    let mut body = Vec::new();
    plist::to_writer_xml(&mut body, &Value::Dictionary(message))?;
    let mut head = [0u8; 16];
    head[0..4].copy_from_slice(&((16 + body.len()) as u32).to_le_bytes());
    head[4..8].copy_from_slice(&1u32.to_le_bytes());
    head[8..12].copy_from_slice(&8u32.to_le_bytes());
    head[12..16].copy_from_slice(&tag.to_le_bytes());
    socket.write_all(&head)?;
    socket.write_all(&body)?;
    Ok(())
}

fn receive(socket: &mut Socket) -> Result<Dictionary> {
    let mut head = [0u8; 16];
    socket.read_exact(&mut head)?;
    let total = u32::from_le_bytes(head[0..4].try_into().unwrap()) as usize;
    if !(16..=1 << 20).contains(&total) {
        bail!("usbmuxd sent a {total}-byte message");
    }
    let mut body = vec![0u8; total - 16];
    socket.read_exact(&mut body)?;
    match plist::from_bytes::<Value>(&body)? {
        Value::Dictionary(d) => Ok(d),
        _ => bail!("usbmuxd sent something other than a dictionary"),
    }
}

fn request(kind: &str) -> Dictionary {
    let mut d = Dictionary::new();
    d.insert("MessageType".into(), kind.into());
    d.insert("ProgName".into(), "Skykeep".into());
    d.insert("ClientVersionString".into(), "skykeep".into());
    d
}

/// The iPhones on a cable now, by usbmuxd's device id. Phones on Wi-Fi sync are left out:
/// only the cable is a private link.
pub fn phones() -> Result<Vec<u64>> {
    let mut s = open()?;
    send(&mut s, request("ListDevices"), 1)?;
    let reply = receive(&mut s)?;
    let list = reply
        .get("DeviceList")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(list
        .iter()
        .filter_map(Value::as_dictionary)
        .filter(|d| {
            d.get("Properties")
                .and_then(Value::as_dictionary)
                .and_then(|p| p.get("ConnectionType"))
                .and_then(Value::as_string)
                == Some("USB")
        })
        .filter_map(|d| d.get("DeviceID").and_then(Value::as_unsigned_integer))
        .collect())
}

/// The phone is on the cable but nothing answers on the port: Skykeep isn't open on it.
/// Kept apart from every other failure, which means the phone (or usbmuxd) is gone.
#[derive(Debug)]
pub struct AppNotOpen;

impl std::fmt::Display for AppNotOpen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Skykeep isn't open on the iPhone")
    }
}

impl std::error::Error for AppNotOpen {}

/// A connection to `port` on the phone's loopback address, through the cable.
pub fn connect(device: u64, port: u16) -> Result<Socket> {
    let mut s = open()?;
    let mut d = request("Connect");
    d.insert("DeviceID".into(), device.into());
    // usbmuxd takes the port in network byte order inside a host-order number.
    d.insert("PortNumber".into(), (port.swap_bytes() as u64).into());
    send(&mut s, d, 2)?;
    let reply = receive(&mut s)?;
    match reply.get("Number").and_then(Value::as_unsigned_integer) {
        Some(0) => Ok(s),
        // 3: nothing is listening there: the app isn't open on the phone.
        Some(3) => Err(AppNotOpen.into()),
        other => bail!("usbmuxd refused the connection ({other:?})"),
    }
}

/// Phones being plugged in and out, as they happen: calls `attached` with each new phone's
/// id. Returns when usbmuxd goes away.
pub fn watch(
    listening: &std::sync::Mutex<Option<Socket>>,
    mut attached: impl FnMut(u64),
) -> Result<()> {
    let mut s = open()?;
    send(&mut s, request("Listen"), 3)?;
    // Kept where the owner can shut it, which ends this loop at once.
    *listening.lock().unwrap() = Some(s.try_clone()?);
    loop {
        let event = receive(&mut s)?;
        if event.get("MessageType").and_then(Value::as_string) == Some("Attached") {
            let usb = event
                .get("Properties")
                .and_then(Value::as_dictionary)
                .and_then(|p| p.get("ConnectionType"))
                .and_then(Value::as_string)
                == Some("USB");
            if let (true, Some(id)) = (
                usb,
                event.get("DeviceID").and_then(Value::as_unsigned_integer),
            ) {
                attached(id);
            }
        }
    }
}
