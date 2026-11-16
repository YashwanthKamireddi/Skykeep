//! Whether PipeWire already lists a webcam device as a camera of its own (the session
//! manager adds a v4l2 node for it once it has seen it carry pictures, and keeps it). Then
//! Skykeep's own PipeWire camera would be a second "iPhone Camera" in the same list.

use std::cell::Cell;
use std::rc::Rc;

use pipewire as pw;

/// Asks PipeWire's registry once, up to its sync point: true when a node's path names the
/// device (`api.v4l2.path` or `object.path` = "v4l2:/dev/videoN").
pub fn lists(device: &std::path::Path) -> bool {
    let wanted = device.to_string_lossy().into_owned();
    let run = || -> Result<bool, pw::Error> {
        pw::init();
        let mainloop = pw::main_loop::MainLoopRc::new(None)?;
        let context = pw::context::ContextRc::new(&mainloop, None)?;
        let core = context.connect_rc(None)?;
        let registry = core.get_registry_rc()?;
        let found = Rc::new(Cell::new(false));
        let f = found.clone();
        let _listener = registry
            .add_listener_local()
            .global(move |global| {
                if let Some(props) = global.props {
                    let path = props.get("api.v4l2.path").map(str::to_owned).or_else(|| {
                        props
                            .get("object.path")
                            .and_then(|p| p.strip_prefix("v4l2:"))
                            .map(str::to_owned)
                    });
                    if props.get("media.class") == Some("Video/Source")
                        && path.as_deref() == Some(wanted.as_str())
                    {
                        f.set(true);
                    }
                }
            })
            .register();
        let pending = core.sync(0)?;
        let done = mainloop.clone();
        let _core_listener = core
            .add_listener_local()
            .done(move |id, seq| {
                if id == pw::core::PW_ID_CORE && seq == pending {
                    done.quit();
                }
            })
            .register();
        mainloop.run();
        Ok(found.get())
    };
    run().unwrap_or(false)
}
