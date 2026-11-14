//! USB phones in the launcher (#92): selection, the live screen, input and
//! the adb tools.
//!
//! The screen is `adb exec-out screencap -p` in a loop, about 1–3 frames a
//! second on a real phone. It's the simplest thing plain adb can do without
//! shipping a binary to the phone (scrcpy's server, for one): `screencap`
//! is on every Android, needs no root and no permission prompt. Like the
//! emulator player, which streams from the emulator itself (ADR-0001), the
//! screen comes from adb directly (`physical.connect_info`) instead of
//! through the daemon; every action (tap, key, install, record…) is a
//! `physical.*` command.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use jidr_core::{PhoneConnectInfo, PhoneState, PhysicalDevice};
use serde_json::json;
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer, Weak};
use tokio::task::JoinHandle;

use crate::AppWindow;
use crate::app::{Ctx, report, toast};

/// Fastest the screen refreshes; screencap itself usually takes longer.
const MIN_FRAME: Duration = Duration::from_millis(250);

#[derive(Default)]
struct Screen {
    /// The phone the frame loop follows.
    serial: Option<String>,
    task: Option<JoinHandle<()>>,
    /// Where and when the pointer went down (fractions of the screen).
    press: Option<(f32, f32, Instant)>,
    /// The phone's screen in pixels, from the last frame.
    size: (u32, u32),
    /// The phone being recorded.
    recording: Option<String>,
}

thread_local! {
    static SCREEN: RefCell<Screen> = RefCell::default();
}

fn selected(ctx: &Rc<Ctx>) -> Option<PhysicalDevice> {
    let st = ctx.state.borrow();
    let serial = st.selected_phone.as_ref()?;
    st.data.phones.iter().find(|p| &p.serial == serial).cloned()
}

pub fn wire(ctx: &Rc<Ctx>) {
    let app = &ctx.app;

    let c = ctx.clone();
    app.on_select_phone(move |i| {
        let serial = {
            let st = c.state.borrow();
            usize::try_from(i)
                .ok()
                .and_then(|i| st.data.phones.get(i))
                .map(|p| p.serial.clone())
        };
        {
            let mut st = c.state.borrow_mut();
            if st.selected_phone != serial {
                st.phone_open = false;
            }
            st.selected_phone = serial;
        }
        c.rerender();
        sync(&c);
    });

    let c = ctx.clone();
    app.on_phone_action(move |a| action(&c, a.as_str()));

    let c = ctx.clone();
    app.on_phone_pointer(move |kind, x, y| pointer(&c, kind, x, y));
}

/// Starts or stops the frame loop to match the view (UI thread).
pub fn sync(ctx: &Rc<Ctx>) {
    let want = {
        let st = ctx.state.borrow();
        selected(ctx)
            .filter(|p| st.phone_open && p.state == PhoneState::Ready)
            .map(|p| p.serial)
    };
    let recording = SCREEN.with(|s| {
        let s = s.borrow();
        s.recording.is_some() && s.recording == ctx.state.borrow().selected_phone
    });
    ctx.app.set_phone_recording(recording);
    let same = SCREEN.with(|s| s.borrow().serial == want);
    if same {
        return;
    }
    SCREEN.with(|s| {
        let mut s = s.borrow_mut();
        if let Some(t) = s.task.take() {
            t.abort();
        }
        s.serial = want.clone();
        s.press = None;
    });
    ctx.app.set_phone_has_frame(false);
    ctx.app.set_phone_rate("".into());
    let Some(serial) = want else { return };
    let Some(client) = ctx.bridge.client() else {
        return;
    };
    let weak = ctx.app.as_weak();
    let task = ctx.bridge.handle().spawn(async move {
        let info: PhoneConnectInfo = match client
            .call_as("physical.connect_info", json!({ "device": serial }))
            .await
        {
            Ok(i) => i,
            Err(e) => return show_rate(&weak, e.message),
        };
        frames(info, weak).await;
    });
    SCREEN.with(|s| s.borrow_mut().task = Some(task));
}

/// The frame loop: screencap, decode, show; until aborted.
async fn frames(info: PhoneConnectInfo, weak: Weak<AppWindow>) {
    let adb = jidr_adb::Adb::new(&info.adb_path)
        .device(&info.serial)
        .with_timeout(Duration::from_secs(10));
    let mut times: Vec<Instant> = Vec::new();
    loop {
        let start = Instant::now();
        match adb.screencap_png().await.and_then(|png| decode(&png)) {
            Ok((rgba, w, h)) => {
                times.push(Instant::now());
                times.retain(|t| t.elapsed() < Duration::from_secs(5));
                let fps =
                    times.len() as f64 / 5.0_f64.min(times[0].elapsed().as_secs_f64().max(0.5));
                let rate = format!("Live screen over adb · {fps:.1} frames/s");
                let weak = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(app) = weak.upgrade() else { return };
                    let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&rgba, w, h);
                    app.set_phone_frame(Image::from_rgba8(buf));
                    app.set_phone_frame_w(w as i32);
                    app.set_phone_frame_h(h as i32);
                    app.set_phone_has_frame(true);
                    app.set_phone_rate(rate.into());
                    SCREEN.with(|s| s.borrow_mut().size = (w, h));
                });
            }
            Err(e) => {
                show_rate(&weak, format!("No screen: {}", e.message));
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        if let Some(rest) = MIN_FRAME.checked_sub(start.elapsed()) {
            tokio::time::sleep(rest).await;
        }
    }
}

fn show_rate(weak: &Weak<AppWindow>, text: String) {
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = weak.upgrade() {
            app.set_phone_rate(text.into());
        }
    });
}

/// A PNG as RGBA8 pixels.
pub fn decode(png: &[u8]) -> jidr_core::JidrResult<(Vec<u8>, u32, u32)> {
    let bad =
        |e: &dyn std::fmt::Display| jidr_core::JidrError::unexpected(format!("screen PNG: {e}"));
    let mut dec = png::Decoder::new(std::io::Cursor::new(png));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().map_err(|e| bad(&e))?;
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| bad(&"too large"))?
    ];
    let info = reader.next_frame(&mut buf).map_err(|e| bad(&e))?;
    let (w, h) = (info.width, info.height);
    let n = (w * h) as usize;
    let rgba = match info.color_type.samples() {
        4 => {
            buf.truncate(n * 4);
            buf
        }
        3 => buf
            .as_chunks::<3>()
            .0
            .iter()
            .take(n)
            .flat_map(|&[r, g, b]| [r, g, b, 255])
            .collect(),
        2 => buf
            .as_chunks::<2>()
            .0
            .iter()
            .take(n)
            .flat_map(|&[g, a]| [g, g, g, a])
            .collect(),
        _ => buf.iter().take(n).flat_map(|g| [*g, *g, *g, 255]).collect(),
    };
    Ok((rgba, w, h))
}

/// A press and release on the screen: a tap, or a swipe when it moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    Tap(u32, u32),
    Swipe {
        from: (u32, u32),
        to: (u32, u32),
        ms: u32,
    },
}

/// Turns a press and a release (fractions of the screen) into a gesture in
/// screen pixels.
pub fn gesture(down: (f32, f32), up: (f32, f32), held: Duration, size: (u32, u32)) -> Gesture {
    let px = |f: (f32, f32)| {
        let clamp = |v: f32, max: u32| {
            (v.clamp(0.0, 1.0) * max as f32)
                .round()
                .min(max.saturating_sub(1) as f32) as u32
        };
        (clamp(f.0, size.0), clamp(f.1, size.1))
    };
    let moved = ((up.0 - down.0).powi(2) + (up.1 - down.1).powi(2)).sqrt();
    if moved < 0.015 {
        let (x, y) = px(down);
        return Gesture::Tap(x, y);
    }
    Gesture::Swipe {
        from: px(down),
        to: px(up),
        ms: held.as_millis().clamp(80, 2000) as u32,
    }
}

fn pointer(ctx: &Rc<Ctx>, kind: i32, x: f32, y: f32) {
    let Some(phone) = selected(ctx) else { return };
    if kind == 0 {
        SCREEN.with(|s| s.borrow_mut().press = Some((x, y, Instant::now())));
        return;
    }
    let Some((dx, dy, at)) = SCREEN.with(|s| s.borrow_mut().press.take()) else {
        return;
    };
    let size = SCREEN.with(|s| s.borrow().size);
    if size.0 == 0 {
        return;
    }
    let w = ctx.app.as_weak();
    let (method, params) = match gesture((dx, dy), (x, y), at.elapsed(), size) {
        Gesture::Tap(x, y) => (
            "physical.tap",
            json!({ "device": phone.serial, "x": x, "y": y }),
        ),
        Gesture::Swipe { from, to, ms } => (
            "physical.swipe",
            json!({ "device": phone.serial, "x1": from.0, "y1": from.1, "x2": to.0, "y2": to.1, "duration_ms": ms }),
        ),
    };
    ctx.bridge.call(method, params, report(&w, ""));
}

fn action(ctx: &Rc<Ctx>, a: &str) {
    let Some(phone) = selected(ctx) else { return };
    let serial = phone.serial.clone();
    let name = phone.display_name().to_string();
    let w = ctx.app.as_weak();
    match a {
        "phone-open" | "phone-close" => {
            ctx.state.borrow_mut().phone_open =
                a == "phone-open" && phone.state == PhoneState::Ready;
            ctx.rerender();
            sync(ctx);
        }
        "back" | "home" | "recents" | "power" | "volume_up" | "volume_down" => {
            ctx.bridge.call(
                "physical.key",
                json!({ "device": serial, "key": a }),
                report(&w, ""),
            );
        }
        "screenshot" => {
            let w2 = w.clone();
            ctx.bridge.call(
                "physical.screenshot",
                json!({ "device": serial }),
                move |r| match r {
                    Ok(v) => toast(
                        &w2,
                        format!("Saved {}", v["path"].as_str().unwrap_or("the screenshot")),
                    ),
                    Err(e) => toast(&w2, e.message),
                },
            );
        }
        "record" => record(ctx, &serial),
        "install" => {
            if let Some(apk) = choose("Choose an APK to install") {
                install(ctx, &serial, &name, apk);
            }
        }
        "push" => {
            if let Some(file) = choose("Choose a file to copy to the phone's Download folder") {
                push(ctx, &serial, file);
            }
        }
        "pull" => pull(ctx, &serial),
        "logcat" => in_terminal(ctx, &serial, "logcat -v color"),
        "shell" => in_terminal(ctx, &serial, "shell"),
        "reboot" => {
            let c2 = ctx.clone();
            crate::launcher::confirm(
                ctx,
                &format!("Reboot {name}?"),
                "The phone restarts. Unsaved work in its apps is lost, and it shows up again once Android has booted.",
                "Reboot",
                move || {
                    let w = c2.app.as_weak();
                    c2.bridge.call(
                        "physical.reboot",
                        json!({ "device": serial }),
                        report(&w, &format!("Rebooting {name}")),
                    );
                },
            );
        }
        _ => {}
    }
}

fn record(ctx: &Rc<Ctx>, serial: &str) {
    let w = ctx.app.as_weak();
    let busy = SCREEN.with(|s| s.borrow().recording.is_some());
    if busy {
        ctx.bridge.call(
            "physical.record_stop",
            json!({ "device": serial }),
            report(&w, "Saving the recording…"),
        );
        return;
    }
    SCREEN.with(|s| s.borrow_mut().recording = Some(serial.to_string()));
    ctx.app.set_phone_recording(true);
    let w2 = w.clone();
    ctx.bridge.run(
        "physical.record",
        json!({ "device": serial }),
        |_| {},
        move |r| {
            let text = match r {
                Ok(v) => format!("Saved {}", v["path"].as_str().unwrap_or("the recording")),
                Err(e) => e.message,
            };
            let _ = slint::invoke_from_event_loop(move || {
                SCREEN.with(|s| s.borrow_mut().recording = None);
                if let Some(c) = crate::app::ctx() {
                    sync(&c);
                }
            });
            toast(&w2, text);
        },
    );
}

fn install(ctx: &Rc<Ctx>, serial: &str, name: &str, apk: PathBuf) {
    let w = ctx.app.as_weak();
    let file = file_name(&apk);
    toast(&w, format!("Installing {file}…"));
    ctx.bridge.call(
        "physical.install",
        json!({ "device": serial, "apk": apk, "replace": true }),
        report(&w, &format!("Installed {file} on {name}")),
    );
}

fn push(ctx: &Rc<Ctx>, serial: &str, file: PathBuf) {
    let w = ctx.app.as_weak();
    let name = file_name(&file);
    ctx.bridge.call(
        "physical.push",
        json!({ "device": serial, "local": file }),
        report(&w, &format!("Copied {name} to /sdcard/Download")),
    );
}

fn pull(ctx: &Rc<Ctx>, serial: &str) {
    let Some(remote) = ask("Path of the file on the phone", "/sdcard/Download/") else {
        return;
    };
    let base = remote
        .rsplit('/')
        .next()
        .filter(|b| !b.is_empty())
        .unwrap_or("file");
    let Some(local) = save_as(base) else { return };
    let w = ctx.app.as_weak();
    ctx.bridge.call(
        "physical.pull",
        json!({ "device": serial, "remote": remote, "local": local }),
        report(&w, &format!("Saved {}", local.display())),
    );
}

/// Opens Terminal running adb on the phone (`logcat`, `shell`): a
/// terminal session, not a device action.
fn in_terminal(ctx: &Rc<Ctx>, serial: &str, args: &str) {
    let w = ctx.app.as_weak();
    let args = args.to_string();
    ctx.bridge.call(
        "physical.connect_info",
        json!({ "device": serial }),
        move |r| match r.and_then(|v| {
            serde_json::from_value::<PhoneConnectInfo>(v)
                .map_err(|e| jidr_core::JidrError::unexpected(e.to_string()))
        }) {
            Ok(info) => {
                let cmd = format!(
                    "{} -s {} {args}",
                    sh_quote(&info.adb_path.to_string_lossy()),
                    sh_quote(&info.serial)
                );
                let ok = std::process::Command::new("osascript")
                    .args([
                        "-e",
                        "on run argv",
                        "-e",
                        "tell application \"Terminal\"",
                        "-e",
                        "activate",
                        "-e",
                        "do script (item 1 of argv)",
                        "-e",
                        "end tell",
                        "-e",
                        "end run",
                        &cmd,
                    ])
                    .status()
                    .is_ok_and(|s| s.success());
                if !ok {
                    toast(&w, format!("Couldn't open Terminal; run: {cmd}"));
                }
            }
            Err(e) => toast(&w, e.message),
        },
    );
}

/// Single-quotes `s` for a POSIX shell.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn osascript(lines: &[&str], args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new("osascript");
    for l in lines {
        cmd.args(["-e", l]);
    }
    let out = cmd.args(args).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}

fn choose(prompt: &str) -> Option<PathBuf> {
    osascript(
        &[
            "on run argv",
            "POSIX path of (choose file with prompt (item 1 of argv))",
            "end run",
        ],
        &[prompt],
    )
    .map(PathBuf::from)
}

fn ask(prompt: &str, default: &str) -> Option<String> {
    osascript(
        &[
            "on run argv",
            "text returned of (display dialog (item 1 of argv) default answer (item 2 of argv))",
            "end run",
        ],
        &[prompt, default],
    )
}

fn save_as(name: &str) -> Option<PathBuf> {
    osascript(
        &[
            "on run argv",
            "POSIX path of (choose file name with prompt \"Save the phone's file as\" default name (item 1 of argv))",
            "end run",
        ],
        &[name],
    )
    .map(PathBuf::from)
}

/// True when the open phone screen takes dropped files (no device tab in
/// front).
pub fn takes_drop(ctx: &Rc<Ctx>) -> bool {
    let st = ctx.state.borrow();
    st.active.is_none()
        && st.phone_open
        && selected(ctx).is_some_and(|p| p.state == PhoneState::Ready)
}

/// The drop label while a file hovers the phone screen.
pub fn hover(ctx: &Rc<Ctx>, path: Option<&Path>) {
    let Some(path) = path else {
        ctx.app.set_drop_hover(false);
        return;
    };
    let Some(phone) = selected(ctx) else { return };
    let (t, b) = if is_apk(path) {
        (
            format!("Release to install {}", file_name(path)),
            format!(
                "adb install -r on {}. Replaces the installed version and keeps its data.",
                phone.display_name()
            ),
        )
    } else {
        (
            format!("Release to copy {}", file_name(path)),
            format!("Goes to /sdcard/Download on {}.", phone.display_name()),
        )
    };
    ctx.app.set_drop_title(t.into());
    ctx.app.set_drop_body(b.into());
    ctx.app.set_drop_hover(true);
}

/// Installs a dropped APK, or copies any other file to Download.
pub fn drop_file(ctx: &Rc<Ctx>, path: &Path) {
    ctx.app.set_drop_hover(false);
    let Some(phone) = selected(ctx) else { return };
    if is_apk(path) {
        install(ctx, &phone.serial, phone.display_name(), path.to_path_buf());
    } else {
        push(ctx, &phone.serial, path.to_path_buf());
    }
}

fn is_apk(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("apk"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_still_press_is_a_tap_and_a_drag_is_a_swipe() {
        let size = (1080, 2400);
        assert_eq!(
            gesture((0.5, 0.5), (0.505, 0.5), Duration::from_millis(90), size),
            Gesture::Tap(540, 1200)
        );
        assert_eq!(
            gesture((0.5, 0.8), (0.5, 0.2), Duration::from_millis(300), size),
            Gesture::Swipe {
                from: (540, 1920),
                to: (540, 480),
                ms: 300
            }
        );
        assert_eq!(
            gesture((1.2, -0.1), (1.2, -0.1), Duration::ZERO, size),
            Gesture::Tap(1079, 0),
            "clamped to the screen"
        );
    }

    #[test]
    fn shell_quoting() {
        assert_eq!(sh_quote("/Users/a b/adb"), "'/Users/a b/adb'");
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn screen_pngs_decode_to_rgba() {
        let mut png = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png, 2, 1);
            enc.set_color(png::ColorType::Rgb);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[1, 2, 3, 4, 5, 6]).unwrap();
        }
        let (rgba, w, h) = decode(&png).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(rgba, vec![1, 2, 3, 255, 4, 5, 6, 255]);
    }
}
