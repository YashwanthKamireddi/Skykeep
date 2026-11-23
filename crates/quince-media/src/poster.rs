//! A video's picture when Apple's is blank.
//!
//! Apple makes a video's poster from its first frame, and a screen recording's first frame is
//! black: in his library a 1:02 recording showed as a black square in the grid (its thumbnail
//! 0.0 light to the last pixel) though its picture starts half a second in. Here the first
//! frame that shows something is read from the video itself — upright, as the phone recorded
//! it — and made the video's thumbnail and screen-sized picture in their place.

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

/// Whether a frame can be read from a video here at all (see the app's stand-in elsewhere).
pub const READS_FRAMES: bool = true;

/// Where the poster is looked for: the start, then a little way in, then further.
const LOOKS: [u64; 7] = [0, 250, 500, 1000, 2000, 4000, 8000];

/// Average lightness (0–255) above which a frame shows something.
const SEEN: f64 = 8.0;

/// The first frame of the video at `path` that is not black, as a JPEG. `None` when every
/// look finds black (a video of the dark is shown as it is).
///
/// # Errors
/// When GStreamer cannot read the file.
pub fn first_seen_frame(path: &std::path::Path) -> Result<Option<Vec<u8>>, String> {
    gst::init().map_err(|e| e.to_string())?;
    // videoflip's automatic direction turns the frame upright, as the phone's orientation tag
    // says; the player does the same.
    let pipeline = gst::parse::launch(
        "filesrc name=src ! decodebin ! videoconvert ! videoflip video-direction=auto ! \
         videoconvert ! video/x-raw,format=RGB ! appsink name=sink sync=false",
    )
    .map_err(|e| e.to_string())?
    .downcast::<gst::Pipeline>()
    .map_err(|_| "not a pipeline")?;
    let src = pipeline.by_name("src").ok_or("no source")?;
    src.set_property("location", path.to_string_lossy().as_ref());
    let sink = pipeline
        .by_name("sink")
        .ok_or("no sink")?
        .downcast::<gst_app::AppSink>()
        .map_err(|_| "not an appsink")?;
    let found = (|| {
        pipeline
            .set_state(gst::State::Paused)
            .map_err(|e| e.to_string())?;
        let (done, ..) = pipeline.state(gst::ClockTime::from_seconds(10));
        done.map_err(|e| e.to_string())?;
        for at in LOOKS {
            if at > 0
                && pipeline
                    .seek_simple(
                        gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                        gst::ClockTime::from_mseconds(at),
                    )
                    .is_err()
            {
                break;
            }
            let Some(sample) = sink.try_pull_preroll(gst::ClockTime::from_seconds(5)) else {
                continue;
            };
            let (Some(buffer), Some(caps)) = (sample.buffer(), sample.caps()) else {
                continue;
            };
            let info = gstreamer_video::VideoInfo::from_caps(caps).map_err(|e| e.to_string())?;
            let map = buffer.map_readable().map_err(|e| e.to_string())?;
            let (w, h) = (info.width(), info.height());
            let stride = usize::try_from(info.stride()[0]).unwrap_or(0);
            if crate::rgb_lightness(&map, w, h, stride) > SEEN {
                return crate::rgb_to_jpeg(&map, w, h, stride).map(Some);
            }
        }
        Ok(None)
    })();
    let _ = pipeline.set_state(gst::State::Null);
    found
}

#[cfg(test)]
mod tests {
    /// A real video, given by path (QUINCE_POSTER_VIDEO): its poster's size and lightness.
    #[test]
    #[ignore = "needs a video on disk"]
    fn a_real_video() {
        let Ok(path) = std::env::var("QUINCE_POSTER_VIDEO") else {
            return;
        };
        let jpeg = super::first_seen_frame(std::path::Path::new(&path))
            .unwrap()
            .expect("a frame");
        let picture = image::load_from_memory(&jpeg).unwrap();
        let gray = picture.to_luma8();
        let mean = gray.pixels().map(|p| f64::from(p.0[0])).sum::<f64>()
            / f64::from(gray.width() * gray.height());
        println!(
            "poster {}x{}, lightness {mean:.1}, blank {}",
            picture.width(),
            picture.height(),
            crate::is_blank(&jpeg)
        );
    }

    /// Half a second of black, then a picture, recorded turned a quarter (as a phone held
    /// upright records): the poster is the picture, not the black, and stands upright.
    #[test]
    fn the_first_frame_that_shows_something_upright() {
        let film =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/black-start.mp4");
        let jpeg = super::first_seen_frame(&film)
            .unwrap()
            .expect("a frame that shows something");
        assert!(!crate::is_blank(&jpeg), "not the black first frame");
        let picture = image::load_from_memory(&jpeg).unwrap();
        assert_eq!(
            (picture.width(), picture.height()),
            (48, 64),
            "turned upright, as the phone recorded it"
        );
    }

    /// An iPhone's kind of video: HEVC in a QuickTime file (`hvc1`), a made-up test pattern.
    /// Read through gst-libav's H.265 decoder, as the AppImage carries it.
    #[test]
    fn an_hevc_video() {
        let film = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hevc.mov");
        let jpeg = super::first_seen_frame(&film)
            .unwrap()
            .expect("a frame of the test pattern");
        let picture = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((picture.width(), picture.height()), (128, 96));
    }
}
