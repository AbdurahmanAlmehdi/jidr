//! The device window (Phase 7c): a session per active device tab, input,
//! toolbar, drag-drop, shared clipboard, logcat and the debug overlay.

pub mod bench;
pub mod drop;
pub mod input;
pub mod logcat;
pub mod session;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use jidr_core::{ConnectInfo, DeviceStatus, RootMode};
use jidr_proto::control as pb;
use serde_json::json;
use slint::{ComponentHandle, ModelRc, VecModel};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::LogLine;
use crate::app::{Ctx, report, toast};
use input::{Gesture, Screen};
use session::{Session, lock, oriented, request_size};

#[derive(Default)]
pub struct Player {
    pub session: Option<Session>,
    opening: Option<Uuid>,
    gesture: Gesture,
    quarter: i32,
    logical: (f32, f32),
    overlay: Option<slint::Timer>,
    log_device: Option<Uuid>,
    log_task: Option<JoinHandle<()>>,
    pub log_lines: Vec<LogLine>,
    log_dirty: bool,
    log_timer: Option<slint::Timer>,
    bench_started: bool,
}

impl Drop for Player {
    fn drop(&mut self) {
        if let Some(t) = self.log_task.take() {
            t.abort();
        }
    }
}

/// The active tab's device id and whether it is running.
fn active_device(ctx: &Ctx) -> Option<(Uuid, bool, Screen, RootMode)> {
    let st = ctx.state.borrow();
    let id = st.active.and_then(|i| st.tabs.get(i)).copied()?;
    let info = st.data.devices.iter().find(|d| d.device.id == id)?;
    let running = matches!(info.device.status, DeviceStatus::Running);
    let screen = Screen {
        width: info.width.max(1),
        height: info.height.max(1),
    };
    Some((id, running, screen, info.device.root.mode))
}

/// Makes the session match the active tab: open for a running device,
/// close otherwise. Called after every tab or data change.
pub fn sync(ctx: &Rc<Ctx>) {
    let active = active_device(ctx);
    let mut p = ctx.player.borrow_mut();
    let want = active.filter(|a| a.1).map(|a| a.0);
    if p.session.as_ref().map(|s| s.device) != want {
        p.session = None;
        ctx.app.set_player_has_frame(false);
        ctx.app.set_player_recording(false);
        ctx.app.set_player_clipboard(false);
    }
    if p.log_device != want {
        if let Some(t) = p.log_task.take() {
            t.abort();
        }
        p.log_lines.clear();
        p.log_dirty = true;
        p.log_device = None;
    }
    let Some((id, _, screen, _)) = active.filter(|a| a.1) else {
        return;
    };
    if p.log_device.is_none() {
        p.log_device = Some(id);
        p.log_task = Some(follow_logcat(ctx, id));
    }
    if p.session.is_some() || p.opening == Some(id) {
        return;
    }
    p.opening = Some(id);
    let scale = ctx.app.window().scale_factor();
    ctx.app.set_player_scale(scale);
    let want_px = request_size(p.logical, scale, screen);
    drop(p);
    let weak = ctx.app.as_weak();
    let handle = ctx.bridge.handle();
    let c = ctx.clone();
    let token = crate::app::defer(move |r: jidr_core::JidrResult<Session>| {
        let still_active = |id| active_device(&c).filter(|a| a.1).map(|a| a.0) == Some(id);
        match r {
            Ok(s) => {
                let keep = still_active(s.device);
                let mut p = c.player.borrow_mut();
                p.opening = None;
                if keep {
                    p.session = Some(s);
                }
                drop(p);
                if keep {
                    crate::widgets::load(&c);
                }
            }
            Err(e) => {
                c.player.borrow_mut().opening = None;
                toast(
                    &c.app.as_weak(),
                    format!("Can't show the screen: {}", e.message),
                );
            }
        }
        drop_bench_hooks(&c);
    });
    ctx.bridge
        .call("device.connect_info", json!({ "device": id }), move |r| {
            let r = r.and_then(|v| {
                serde_json::from_value::<ConnectInfo>(v)
                    .map_err(|e| jidr_core::JidrError::unexpected(e.to_string()))
            });
            handle.spawn(async move {
                let s = match r {
                    Ok(info) => Session::open(id, info, screen, want_px, weak).await,
                    Err(e) => Err(e),
                };
                crate::app::fire(token, s);
            });
        });
}

fn with_session(ctx: &Ctx, f: impl FnOnce(&Session)) {
    if let Some(s) = ctx.player.borrow().session.as_ref() {
        f(s);
    }
}

fn current_screen(s: &Session) -> Screen {
    let frame = *lock(&s.stats.frame);
    if frame.0 == 0 {
        s.native
    } else {
        oriented(s.native, frame)
    }
}

fn spawn_key(ctx: &Ctx, ev: pb::KeyboardEvent) {
    with_session(ctx, |s| {
        let mut c = s.ctl.clone();
        ctx.bridge.handle().spawn(async move {
            let _ = c.send_key(ev).await;
        });
    });
}

pub fn wire(ctx: &Rc<Ctx>) {
    let app = &ctx.app;

    let c = ctx.clone();
    app.on_player_pointer(move |kind, nx, ny, alt, shift| {
        let mut p = c.player.borrow_mut();
        if kind == 0 {
            p.gesture = Gesture::from_mods(alt, shift);
        }
        let g = p.gesture;
        let Some(s) = p.session.as_ref() else { return };
        let screen = current_screen(s);
        let pts = input::fingers(g, screen, screen.to_px(nx, ny));
        s.send(input::touch_event(&pts, kind != 2), kind != 2);
    });

    let c = ctx.clone();
    app.on_player_wheel(move |dx, dy| {
        with_session(&c, |s| {
            let mut ctl = s.ctl.clone();
            let ev = input::wheel(dx, dy);
            c.bridge.handle().spawn(async move {
                let _ = ctl.inject_wheel(tokio_stream::iter(vec![ev])).await;
            });
        });
    });

    let c = ctx.clone();
    app.on_player_key(move |text, down| {
        if let Some(ev) = input::key_event(&text, down) {
            spawn_key(&c, ev);
        }
    });

    let c = ctx.clone();
    app.on_player_tool(move |tool| tool_action(&c, &tool));

    let c = ctx.clone();
    app.on_player_start(move || {
        if let Some((id, ..)) = active_device(&c) {
            crate::launcher::start(&c, id);
        }
    });

    let c = ctx.clone();
    app.on_player_resized(move |w, h| {
        tracing::info!(w, h, "player screen resized");
        let Ok(mut p) = c.player.try_borrow_mut() else {
            return;
        };
        p.logical = (w, h);
        let scale = c.app.window().scale_factor();
        if let Some(s) = p.session.as_ref() {
            s.resize(request_size((w, h), scale, current_screen(s)));
        }
    });

    // Logcat panel.
    let c = ctx.clone();
    app.on_log_level_changed(move |_| refresh_log(&c));
    let c = ctx.clone();
    app.on_log_filter_changed(move |_| refresh_log(&c));
    let c = ctx.clone();
    app.on_log_clear(move || {
        c.player.borrow_mut().log_lines.clear();
        refresh_log(&c);
        if let Some((id, ..)) = active_device(&c) {
            c.bridge
                .call("logcat.clear", json!({ "device": id }), |_| {});
        }
    });
    let c = ctx.clone();
    app.on_log_toggle_follow(move || {
        let f = !c.app.get_log_following();
        c.app.set_log_following(f);
    });
    app.on_log_open(|_| {});

    // Refresh the logcat model at most 5× a second.
    let c = ctx.clone();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(200),
        move || {
            let dirty = std::mem::take(&mut c.player.borrow_mut().log_dirty);
            if dirty {
                refresh_log(&c);
            }
        },
    );
    ctx.player.borrow_mut().log_timer = Some(timer);

    // Drag and drop from Finder (winit window events).
    let c = ctx.clone();
    {
        use slint::winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent};
        app.window().on_winit_window_event(move |_, ev| {
            match ev {
                WindowEvent::HoveredFile(p) => hover(&c, Some(p.clone())),
                WindowEvent::HoveredFileCancelled => hover(&c, None),
                WindowEvent::DroppedFile(p) => {
                    hover(&c, None);
                    handle_drop(&c, p);
                }
                _ => {}
            }
            EventResult::Propagate
        });
    }
}

/// Meta shortcuts that belong to the device tab.
pub fn shortcut(ctx: &Rc<Ctx>, s: &str) -> bool {
    match s {
        "overlay" => toggle_overlay(ctx),
        "pixel-perfect" => {
            let v = !ctx.app.get_player_pixel_perfect();
            ctx.app.set_player_pixel_perfect(v);
        }
        "logcat" => {
            let v = !ctx.app.get_logcat_open();
            ctx.app.set_logcat_open(v);
        }
        "panel" => {
            let v = !ctx.app.get_player_panel_open();
            ctx.app.set_player_panel_open(v);
        }
        "screenshot" => tool_action(ctx, "screenshot"),
        _ => return false,
    }
    true
}

fn tool_action(ctx: &Rc<Ctx>, tool: &str) {
    let Some((id, ..)) = active_device(ctx) else {
        return;
    };
    let w = ctx.app.as_weak();
    if let Some(key) = input::tool_key(tool) {
        // Same path as the CLI and MCP (`key.press`): hardware buttons go
        // in as evdev codes there.
        ctx.bridge.call(
            "key.press",
            json!({ "device": id, "key": key }),
            report(&w, ""),
        );
        return;
    }
    match tool {
        "rotate-left" | "rotate-right" => {
            let q = {
                let mut p = ctx.player.borrow_mut();
                p.quarter = (p.quarter + if tool == "rotate-left" { 1 } else { 3 }) % 4;
                p.quarter
            };
            let mut vm = ctx.app.get_widgets();
            vm.rotation = q;
            ctx.app.set_widgets(vm);
            ctx.bridge.call(
                "sensor.rotate",
                json!({ "device": id, "angle": 90 * q }),
                report(&w, ""),
            );
        }
        "screenshot" => crate::recording::screenshot(ctx, id),
        "record" => crate::recording::toggle(ctx, id),
        "clipboard" => {
            let on = {
                let mut p = ctx.player.borrow_mut();
                match p.session.as_mut() {
                    Some(s) => {
                        let on = !s.clipboard_sharing();
                        // The tasks run on the IO runtime.
                        let _guard = ctx.bridge.handle().enter();
                        s.set_clipboard_sharing(on);
                        on
                    }
                    None => false,
                }
            };
            ctx.app.set_player_clipboard(on);
            toast(
                &w,
                if on {
                    "Sharing the clipboard with the device"
                } else {
                    "Stopped sharing the clipboard"
                }
                .into(),
            );
        }
        _ => {}
    }
}

use crate::recording::file_name;

fn toggle_overlay(ctx: &Rc<Ctx>) {
    let on = !ctx.app.get_player_overlay();
    ctx.app.set_player_overlay(on);
    let mut p = ctx.player.borrow_mut();
    if !on {
        p.overlay = None;
        return;
    }
    let c = ctx.clone();
    let mut last = (std::time::Instant::now(), 0u64, 0u64);
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(500), move || {
        let text = {
            let p = c.player.borrow();
            match p.session.as_ref() {
                Some(s) => {
                    use std::sync::atomic::Ordering;
                    let st = &s.stats;
                    let (r, sh) = (st.received.load(Ordering::Relaxed), st.shown.load(Ordering::Relaxed));
                    let dt = last.0.elapsed().as_secs_f32().max(0.001);
                    let line = format!(
                        "recv {:.0} fps · shown {:.0} fps · active {:.0} fps\nframe {}×{} · age {} · touch→frame {}",
                        (r - last.1) as f32 / dt,
                        (sh - last.2) as f32 / dt,
                        st.active_fps(),
                        lock(&st.frame).0,
                        lock(&st.frame).1,
                        st.median_age().map(|a| format!("{a:.1} ms")).unwrap_or_else(|| "—".into()),
                        st.median_latency().map(|a| format!("{a:.1} ms")).unwrap_or_else(|| "—".into()),
                    );
                    last = (std::time::Instant::now(), r, sh);
                    line
                }
                None => "no frames yet".into(),
            }
        };
        c.app.set_player_overlay_text(text.into());
    });
    p.overlay = Some(timer);
}

fn refresh_log(ctx: &Rc<Ctx>) {
    let p = ctx.player.borrow();
    let level = usize::try_from(ctx.app.get_log_level()).unwrap_or(1);
    let filter = ctx.app.get_log_filter().to_string();
    let vis = logcat::visible(&p.log_lines, level, &filter);
    let errors: Vec<&LogLine> = p
        .log_lines
        .iter()
        .filter(|l| logcat::level_rank(&l.level) >= 4)
        .collect();
    ctx.app.set_log_errors(errors.len() as i32);
    ctx.app.set_log_last_error(
        errors
            .last()
            .map(|l| logcat::short(l))
            .unwrap_or_default()
            .into(),
    );
    ctx.app.set_log_summary(
        format!(
            "{} of {} lines · {} error{}",
            vis.len(),
            p.log_lines.len(),
            errors.len(),
            if errors.len() == 1 { "" } else { "s" }
        )
        .into(),
    );
    let shown: Vec<LogLine> = vis.into_iter().rev().take(500).rev().collect();
    ctx.app.set_log_lines(ModelRc::new(VecModel::from(shown)));
}

/// Follows the device log on a connection of its own (`logcat.follow`
/// pushes until the connection closes).
fn follow_logcat(ctx: &Rc<Ctx>, device: Uuid) -> JoinHandle<()> {
    let socket = ctx.paths.socket();
    let c = ctx.clone();
    let token_base = std::sync::Arc::new(());
    let _ = token_base;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<LogLine>>();
    // UI side: drain batches on the event loop.
    let weak = ctx.app.as_weak();
    let ui = ctx.bridge.handle().spawn(async move {
        while let Some(batch) = rx.recv().await {
            let weak = weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if weak.upgrade().is_none() {
                    return;
                }
                if let Some(ctx) = crate::app::ctx() {
                    let mut p = ctx.player.borrow_mut();
                    if p.log_device != Some(device) {
                        return;
                    }
                    p.log_lines.extend(batch);
                    let n = p.log_lines.len();
                    if n > logcat::KEEP {
                        p.log_lines.drain(..n - logcat::KEEP);
                    }
                    p.log_dirty = true;
                }
            });
        }
    });
    let _ = c;
    ctx.bridge.handle().spawn(async move {
        let _ui = AbortOnDrop(ui);
        let Ok(client) = jidr_client::Client::connect(&socket, "gui-logcat").await else {
            return;
        };
        let mut events = client.events();
        if client
            .call("logcat.follow", json!({ "device": device }))
            .await
            .is_err()
        {
            return;
        }
        let mut batch = Vec::new();
        let mut tick = tokio::time::interval(Duration::from_millis(150));
        loop {
            tokio::select! {
                ev = events.recv() => match ev {
                    Ok(ev) if ev.topic == "logcat" => {
                        if let Some(l) = ev.data.get("line").and_then(|l| l.as_str()).and_then(logcat::parse) {
                            batch.push(l);
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => break,
                },
                _ = tick.tick() => {
                    if !batch.is_empty() && tx.send(std::mem::take(&mut batch)).is_err() {
                        break;
                    }
                }
            }
        }
    })
}

struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn hover(ctx: &Rc<Ctx>, path: Option<PathBuf>) {
    let Some(path) = path else {
        ctx.app.set_drop_hover(false);
        return;
    };
    let Some((id, running, _, mode)) = active_device(ctx) else {
        return;
    };
    if !running {
        return;
    }
    let name = ctx.info(id).map(|d| d.device.name).unwrap_or_default();
    let (t, b) = drop::hover_text(&drop::classify(&path, mode == RootMode::Magisk), &name);
    ctx.app.set_drop_title(t.into());
    ctx.app.set_drop_body(b.into());
    ctx.app.set_drop_hover(true);
}

/// Installs, flashes or copies a dropped file (D35).
pub fn handle_drop(ctx: &Rc<Ctx>, path: &Path) {
    let Some((id, running, _, mode)) = active_device(ctx) else {
        return;
    };
    let w = ctx.app.as_weak();
    if !running {
        toast(
            &w,
            "Start the device first, then drop the file again".into(),
        );
        return;
    }
    let name = file_name(&path.to_string_lossy());
    match drop::classify(path, mode == RootMode::Magisk) {
        drop::DropAction::InstallApk(apk) => install_apk(ctx, id, apk, false),
        drop::DropAction::MagiskModule(zip) => {
            let b = ctx.bridge.clone();
            let w2 = w.clone();
            let zip2 = zip.clone();
            ctx.bridge.run(
                "root.module.install",
                json!({ "device": id, "path": zip }),
                |_| {},
                move |r| match r {
                    Ok(_) => toast(&w2, format!("Installed {name}; the device is rebooting")),
                    Err(e)
                        if e.message.contains("Unknown method")
                            || e.message.contains("unknown method")
                            || e.message.contains("Method not found") =>
                    {
                        // Magisk modules arrive with Stream C's 9d; until then copy it.
                        b.call(
                            "file.push",
                            json!({ "device": id, "local": zip2 }),
                            report(
                                &w2,
                                &format!(
                                    "Copied {name} to Download (module install isn't available yet)"
                                ),
                            ),
                        );
                    }
                    Err(e) => toast(&w2, e.message),
                },
            );
        }
        drop::DropAction::Push { path, note } => {
            let msg = match note {
                Some(n) => format!("Copied {name} to Download. {n}"),
                None => format!("Copied {name} to /sdcard/Download"),
            };
            ctx.bridge.call(
                "file.push",
                json!({ "device": id, "local": path }),
                report(&w, &msg),
            );
        }
    }
}

fn install_apk(ctx: &Rc<Ctx>, id: Uuid, apk: PathBuf, reinstall: bool) {
    let w = ctx.app.as_weak();
    let name = file_name(&apk.to_string_lossy());
    toast(&w, format!("Installing {name}…"));
    let b = ctx.bridge.clone();
    let apk2 = apk.clone();
    let token = crate::app::defer(move |r: jidr_core::JidrResult<()>| {
        let Some(c) = crate::app::ctx() else { return };
        match r {
            Ok(()) => {}
            Err(e) if !reinstall && drop::is_signature_mismatch(&e.message) => {
                let c2 = c.clone();
                let apk3 = apk2.clone();
                crate::launcher::confirm(
                    &c,
                    &format!("Reinstall {}?", file_name(&apk2.to_string_lossy())),
                    "The installed app was signed with a different key. Reinstalling uninstalls it first, which deletes its data.",
                    "Reinstall",
                    move || install_apk(&c2, id, apk3.clone(), true),
                );
            }
            Err(e) => toast(&c.app.as_weak(), e.message),
        }
    });
    let w2 = w.clone();
    ctx.bridge.handle().spawn(async move {
        let call = |method: &'static str, params: serde_json::Value| {
            let b = b.clone();
            async move {
                let (tx, rx) = tokio::sync::oneshot::channel();
                b.call(method, params, move |r| {
                    let _ = tx.send(r);
                });
                rx.await
                    .unwrap_or_else(|_| Err(jidr_core::JidrError::unexpected("cancelled")))
            }
        };
        if reinstall {
            let pkg = call("app.inspect", json!({ "apk": apk })).await;
            if let Ok(v) = pkg
                && let Some(p) = v.get("package").and_then(|p| p.as_str())
            {
                let _ = call("app.uninstall", json!({ "device": id, "package": p })).await;
            }
        }
        let r = call(
            "file.install",
            json!({ "device": id, "apk": apk, "replace": true }),
        )
        .await;
        let r = match r {
            Ok(_) => match call("app.launch", json!({ "device": id, "apk": apk })).await {
                Ok(_) => {
                    toast(&w2, format!("Installed and opened {name}"));
                    Ok(())
                }
                Err(_) => {
                    toast(&w2, format!("Installed {name}"));
                    Ok(())
                }
            },
            Err(e) => Err(e),
        };
        crate::app::fire(token, r);
    });
}

/// Dev hooks for the Phase 7c check (`JIDR_PLAYER_BENCH`, `JIDR_PLAYER_DROP`).
fn drop_bench_hooks(ctx: &Rc<Ctx>) {
    if ctx.player.borrow().bench_started {
        return;
    }
    if std::env::var_os("JIDR_PLAYER_BENCH").is_none()
        && std::env::var_os("JIDR_PLAYER_DROP").is_none()
    {
        return;
    }
    ctx.player.borrow_mut().bench_started = true;
    bench::start(ctx);
}
