//! Screenshots and screen recording from the device window (Phase 10a).
//! The daemon does the work (`media.screenshot`, `media.record`); the GUI
//! copies screenshots to the pasteboard and shows the recording clock.

use std::path::Path;
use std::rc::Rc;

use serde_json::{Value, json};
use slint::ComponentHandle;
use uuid::Uuid;

use crate::app::{Ctx, toast};

pub fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

fn path_of(v: &Value) -> String {
    v.get("path")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Puts a PNG on the macOS pasteboard.
pub fn copy_image(path: &str) {
    if path.is_empty() {
        return;
    }
    let script = format!(
        "set the clipboard to (read (POSIX file \"{}\") as «class PNGf»)",
        path.replace('"', "\\\"")
    );
    let _ = std::process::Command::new("osascript")
        .args(["-e", &script])
        .spawn();
}

/// `media.screenshot`: saved to ~/Pictures/Jidr and copied.
pub fn screenshot(ctx: &Rc<Ctx>, device: Uuid) {
    let w = ctx.app.as_weak();
    ctx.bridge.call(
        "media.screenshot",
        json!({ "device": device }),
        move |r| match r {
            Ok(v) => {
                let path = path_of(&v);
                copy_image(&path);
                toast(&w, format!("Saved {} and copied it", file_name(&path)));
            }
            Err(e) => toast(&w, e.message),
        },
    );
}

/// Starts or stops `media.record`; the job's progress drives the clock.
pub fn toggle(ctx: &Rc<Ctx>, device: Uuid) {
    let w = ctx.app.as_weak();
    if ctx.app.get_player_recording() {
        // The job ends and reports the file.
        ctx.bridge
            .call("media.record_stop", json!({ "device": device }), move |r| {
                if let Err(e) = r {
                    toast(&w, e.message);
                }
            });
        return;
    }
    ctx.app.set_player_recording(true);
    ctx.app.set_player_record_text("Recording 0:00".into());
    let wp = w.clone();
    ctx.bridge.run(
        "media.record",
        json!({ "device": device }),
        move |job| {
            let msg = job.message.clone();
            let weak = wp.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = weak.upgrade() {
                    app.set_player_record_text(msg.into());
                }
            });
        },
        move |r| {
            let weak = w.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = weak.upgrade() {
                    app.set_player_recording(false);
                }
            });
            match r {
                Ok(v) => {
                    let secs = v.get("duration_s").and_then(Value::as_f64).unwrap_or(0.0);
                    toast(
                        &w,
                        format!(
                            "Saved {} ({})",
                            file_name(&path_of(&v)),
                            crate::model::duration(secs.round() as i64)
                        ),
                    );
                }
                Err(e) if !jidr_core::cancel::is_cancelled(&e) => toast(&w, e.message),
                Err(_) => {}
            }
        },
    );
}
