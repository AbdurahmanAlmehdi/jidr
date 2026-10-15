//! Screenshots and screen recording from the device window (Phase 10a).
//! The daemon does the work (`media.screenshot`, `media.record`); the GUI
//! copies screenshots to the pasteboard and shows the recording clock.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use serde_json::{Value, json};
use slint::ComponentHandle;
use uuid::Uuid;

use crate::app::{Ctx, toast};

/// Recordings in progress, per device: the clock text each one shows.
#[derive(Debug, Default)]
pub struct Active(HashMap<Uuid, String>);

impl Active {
    pub fn is_recording(&self, device: Uuid) -> bool {
        self.0.contains_key(&device)
    }

    pub fn set(&mut self, device: Uuid, text: Option<String>) {
        match text {
            Some(t) => {
                self.0.insert(device, t);
            }
            None => {
                self.0.remove(&device);
            }
        }
    }

    /// What the toolbar shows for the active tab's device.
    pub fn view(&self, active: Option<Uuid>) -> (bool, String) {
        match active.and_then(|id| self.0.get(&id)) {
            Some(t) => (true, t.clone()),
            None => (false, String::new()),
        }
    }
}

thread_local! {
    static ACTIVE: RefCell<Active> = RefCell::default();
}

/// Shows the recording state of `active` (the active tab's device).
pub fn show(ctx: &Ctx, active: Option<Uuid>) {
    let (on, text) = ACTIVE.with(|a| a.borrow().view(active));
    ctx.app.set_player_recording(on);
    ctx.app.set_player_record_text(text.into());
}

/// Updates one device's state from any thread and refreshes the toolbar.
fn update(device: Uuid, text: Option<String>) {
    let _ = slint::invoke_from_event_loop(move || {
        ACTIVE.with(|a| a.borrow_mut().set(device, text));
        if let Some(ctx) = crate::app::ctx() {
            show(&ctx, crate::player::active_id(&ctx));
        }
    });
}

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

/// `osascript` arguments that put the PNG at `path` on the pasteboard. The
/// path goes in as `argv`, never into the script text, so no file name can
/// change what the script does.
pub fn copy_image_args(path: &str) -> [&str; 7] {
    [
        "-e",
        "on run argv",
        "-e",
        "set the clipboard to (read (POSIX file (item 1 of argv)) as «class PNGf»)",
        "-e",
        "end run",
        path,
    ]
}

/// Puts a PNG on the macOS pasteboard.
pub fn copy_image(path: &str) {
    if path.is_empty() {
        return;
    }
    let _ = std::process::Command::new("osascript")
        .args(copy_image_args(path))
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
    if ACTIVE.with(|a| a.borrow().is_recording(device)) {
        // The job ends and reports the file.
        ctx.bridge
            .call("media.record_stop", json!({ "device": device }), move |r| {
                if let Err(e) = r {
                    toast(&w, e.message);
                }
            });
        return;
    }
    ACTIVE.with(|a| a.borrow_mut().set(device, Some("Recording 0:00".into())));
    show(ctx, Some(device));
    ctx.bridge.run(
        "media.record",
        json!({ "device": device }),
        move |job| update(device, Some(job.message.clone())),
        move |r| {
            update(device, None);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression (review of PR #3): one global flag meant that after
    /// switching from device A's tab to B and back, A's recording showed as
    /// stopped and the button tried to start a second one.
    #[test]
    fn recording_state_is_per_device() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let mut st = Active::default();
        st.set(a, Some("Recording 0:03".into()));
        assert_eq!(st.view(Some(b)), (false, String::new()));
        assert!(!st.is_recording(b));
        assert_eq!(st.view(Some(a)), (true, "Recording 0:03".into()));
        assert!(st.is_recording(a));
        st.set(a, None);
        assert_eq!(st.view(Some(a)), (false, String::new()));
    }

    /// Regression (review of PR #3): the path was spliced into the
    /// AppleScript with only `"` escaped, so a device name with `\` could
    /// close the string and run `do shell script` on the host.
    #[test]
    fn the_image_path_is_an_argument_not_script() {
        let evil = r#"/tmp/a\"&(do shell script "touch /tmp/pwned")--.png"#;
        let args = copy_image_args(evil);
        assert_eq!(args.last(), Some(&evil));
        assert!(args[..args.len() - 1].iter().all(|a| !a.contains("pwned")));
    }

    /// The script really reads its path from argv (macOS only).
    #[cfg(target_os = "macos")]
    #[test]
    fn osascript_takes_the_path_from_argv() {
        let tmp = tempfile::tempdir().unwrap();
        let marker = tmp.path().join("pwned");
        let evil = format!(
            r#"{}/a\"&(do shell script "touch {}")--.png"#,
            tmp.path().display(),
            marker.display()
        );
        let mut args = copy_image_args(&evil);
        // Same script, but echo the path back instead of touching the pasteboard.
        args[3] = "return item 1 of argv";
        let out = std::process::Command::new("osascript")
            .args(args)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), evil);
        assert!(!marker.exists());
    }
}
