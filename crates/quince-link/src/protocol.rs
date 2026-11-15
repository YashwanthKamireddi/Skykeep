//! The wire between the iPhone and this computer.
//!
//! One ordered byte stream (the USB tunnel, or a stream on the Wi-Fi link) carries packets, each a
//! fixed 16-byte header and a body. The header is little-endian, as both ends are:
//!
//! | bytes | field |
//! |---|---|
//! | 1 | kind ([`Kind`]) |
//! | 1 | flags ([`KEYFRAME`]) |
//! | 2 | reserved, zero |
//! | 4 | body length (at most [`MAX_BODY`]) |
//! | 8 | when the phone captured it, in microseconds of the phone's own clock (0 for messages) |
//!
//! Video bodies are one H.264 or HEVC access unit in Annex B form, parameter sets in front of
//! every keyframe, so a receiver can start at any keyframe. Audio bodies are 48 kHz mono signed
//! 16-bit samples. Messages are JSON, so either end can add a field without breaking the other.

use std::io::{self, Read, Write};

/// No single packet is larger: a 4K keyframe at the highest quality is well under it, and a
/// length past it means the stream is out of step, which is answered by closing it.
pub const MAX_BODY: usize = 8 << 20;
pub const HEADER: usize = 16;

/// The picture can be decoded on its own: a receiver joining or recovering starts here.
pub const KEYFRAME: u8 = 1;

/// Every audio body: one channel of 48 kHz signed 16-bit little-endian samples. Over a cable
/// this is 0.77 Mbit/s, so it goes as it is: no codec, no codec delay.
pub const AUDIO_RATE: u32 = 48_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    /// JSON: who is speaking and what it can do ([`Hello`]).
    Hello = 1,
    /// JSON: the picture that follows ([`VideoFormat`]).
    VideoFormat = 2,
    Video = 3,
    Audio = 4,
    /// JSON: a request from either side (a keyframe, a lens, the camera on or off).
    Control = 5,
}

impl Kind {
    fn from_byte(b: u8) -> Option<Kind> {
        Some(match b {
            1 => Kind::Hello,
            2 => Kind::VideoFormat,
            3 => Kind::Video,
            4 => Kind::Audio,
            5 => Kind::Control,
            _ => return None,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Packet {
    pub kind: Kind,
    pub flags: u8,
    pub captured_us: i64,
    pub body: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    H264,
    Hevc,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VideoFormat {
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Hello {
    /// The protocol both ends speak; a phone with a newer one still sends what this reads.
    pub version: u32,
    /// What the person calls the phone ("Yash's iPhone"): the camera and microphone are named after it.
    pub name: String,
}

pub const VERSION: u32 = 1;

impl Packet {
    pub fn message(kind: Kind, value: &impl serde::Serialize) -> Packet {
        Packet {
            kind,
            flags: 0,
            captured_us: 0,
            body: serde_json::to_vec(value).expect("a message always serializes"),
        }
    }

    pub fn write_to(&self, out: &mut impl Write) -> io::Result<()> {
        let mut head = [0u8; HEADER];
        head[0] = self.kind as u8;
        head[1] = self.flags;
        head[4..8].copy_from_slice(&(self.body.len() as u32).to_le_bytes());
        head[8..16].copy_from_slice(&self.captured_us.to_le_bytes());
        out.write_all(&head)?;
        out.write_all(&self.body)
    }

    /// The next packet, or `None` when the other side closed the stream cleanly between packets.
    /// A kind this side doesn't know is read past, so a newer phone can add one.
    pub fn read_from(input: &mut impl Read) -> io::Result<Option<Packet>> {
        loop {
            let mut head = [0u8; HEADER];
            match input.read(&mut head[..1])? {
                0 => return Ok(None),
                _ => input.read_exact(&mut head[1..])?,
            }
            let len = u32::from_le_bytes(head[4..8].try_into().unwrap()) as usize;
            if len > MAX_BODY {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("a {len}-byte packet: the stream is out of step"),
                ));
            }
            let mut body = vec![0u8; len];
            input.read_exact(&mut body)?;
            let Some(kind) = Kind::from_byte(head[0]) else {
                continue;
            };
            return Ok(Some(Packet {
                kind,
                flags: head[1],
                captured_us: i64::from_le_bytes(head[8..16].try_into().unwrap()),
                body,
            }));
        }
    }

    pub fn json<T: serde::de::DeserializeOwned>(&self) -> io::Result<T> {
        serde_json::from_slice(&self.body)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets_read_back_as_written_and_unknown_kinds_are_skipped() {
        let mut wire = Vec::new();
        let video = Packet {
            kind: Kind::Video,
            flags: KEYFRAME,
            captured_us: -42,
            body: vec![0, 0, 0, 1, 0x65],
        };
        video.write_to(&mut wire).unwrap();
        // A kind from a newer phone, between two this side knows.
        wire.extend_from_slice(&[99, 0, 0, 0, 3, 0, 0, 0]);
        wire.extend_from_slice(&[0; 8]);
        wire.extend_from_slice(b"new");
        let hello = Packet::message(
            Kind::Hello,
            &Hello {
                version: VERSION,
                name: "Test iPhone".into(),
            },
        );
        hello.write_to(&mut wire).unwrap();

        let mut r = wire.as_slice();
        assert_eq!(Packet::read_from(&mut r).unwrap().unwrap(), video);
        let back = Packet::read_from(&mut r).unwrap().unwrap();
        assert_eq!(back.json::<Hello>().unwrap().name, "Test iPhone");
        assert!(Packet::read_from(&mut r).unwrap().is_none());
    }

    #[test]
    fn a_length_past_the_limit_closes_the_stream() {
        let mut wire = vec![Kind::Video as u8, 0, 0, 0];
        wire.extend_from_slice(&((MAX_BODY as u32) + 1).to_le_bytes());
        wire.extend_from_slice(&[0; 8]);
        assert!(Packet::read_from(&mut wire.as_slice()).is_err());
    }
}
