//! The running app: wires `AppWindow` callbacks to daemon calls and pushes
//! daemon data back into the view models. View state (selection, tabs,
//! theme) stays here (plan: UI-only state is not a protocol command).

use std::cell::RefCell;
use std::rc::Rc;

use jidr_core::{JidrPaths, JidrResult};
use serde_json::Value;
use slint::{ComponentHandle, ModelRc, VecModel, Weak};
use uuid::Uuid;

use crate::bridge::{Bridge, Conn, Data, Update};
use crate::model::{self, Clock};
use crate::{AppWindow, ConnState, Theme};

/// UI-thread state.
#[derive(Default)]
pub struct State {
    pub data: Data,
    pub selected: Option<Uuid>,
    pub tabs: Vec<Uuid>,
    pub active: Option<usize>,
}

pub type Shared = Rc<RefCell<State>>;

/// System appearance (macOS `AppleInterfaceStyle`).
pub fn system_dark() -> bool {
    std::process::Command::new("defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "Dark")
        .unwrap_or(true)
}

/// macOS Reduce Motion.
pub fn system_reduce_motion() -> bool {
    std::process::Command::new("defaults")
        .args(["read", "com.apple.universalaccess", "reduceMotion"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
        .unwrap_or(false)
}

/// Pushes `state` into the window's models.
pub fn render(app: &AppWindow, state: &State) {
    let clock = Clock::system();
    let vms: Vec<_> = state
        .data
        .devices
        .iter()
        .map(|i| model::device_vm(i, &clock))
        .collect();
    let selected = state
        .selected
        .and_then(|id| state.data.devices.iter().position(|d| d.device.id == id))
        .or(if vms.is_empty() { None } else { Some(0) });
    let tabs: Vec<_> = state
        .tabs
        .iter()
        .filter_map(|id| vms.iter().find(|v| v.id == id.to_string().as_str()))
        .map(model::tab_vm)
        .collect();
    let jobs: Vec<_> = state
        .data
        .jobs
        .iter()
        .filter(|j| {
            j.kind == "image.install"
                || j.kind == "sdk.install"
                || j.kind == "base.create"
                || j.kind.starts_with("root.")
        })
        .map(|j| {
            // B5: real bytes while the daemon reports a download.
            let bytes = j.bytes.map(|b| (b.done, b.total));
            model::job_vm(j, &job_title(j), bytes, None)
        })
        .collect();
    app.set_devices(ModelRc::new(VecModel::from(vms)));
    app.set_jobs(ModelRc::new(VecModel::from(jobs)));
    app.set_tabs(ModelRc::new(VecModel::from(tabs)));
    app.set_selected(selected.map(|i| i as i32).unwrap_or(-1));
    app.set_active_tab(state.active.map(|i| i as i32).unwrap_or(-1));
    let active_device = state
        .active
        .and_then(|i| state.tabs.get(i))
        .and_then(|id| state.data.devices.iter().position(|d| d.device.id == *id));
    app.set_active_device(active_device.map(|i| i as i32).unwrap_or(-1));
    app.set_sdk_ready(state.data.sdk_ready);
    app.set_budget(
        state
            .data
            .budget
            .as_ref()
            .map(|b| model::budget_vm(b, model::typical_mb(&state.data.devices)))
            .unwrap_or_default(),
    );
}

fn job_title(j: &jidr_core::Job) -> String {
    match j.kind.as_str() {
        "image.install" | "sdk.install" => {
            if j.message.is_empty() {
                "Downloading".into()
            } else {
                j.message.clone()
            }
        }
        k if k.starts_with("root.") => "Setting up root".into(),
        "base.create" => "Saving a base".into(),
        k => k.to_string(),
    }
}

/// Shows a toast for a few seconds.
pub fn toast(weak: &Weak<AppWindow>, text: String) {
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = weak.upgrade() {
            app.set_toast(text.clone().into());
            let w = app.as_weak();
            slint::Timer::single_shot(std::time::Duration::from_secs(4), move || {
                if let Some(app) = w.upgrade()
                    && app.get_toast() == text.as_str()
                {
                    app.set_toast("".into());
                }
            });
        }
    });
}

/// Toast an error, or a confirmation on success.
pub fn report(
    weak: &Weak<AppWindow>,
    ok: impl Into<String>,
) -> impl FnOnce(JidrResult<Value>) + Send + 'static {
    let weak = weak.clone();
    let ok = ok.into();
    move |r| match r {
        Ok(_) if !ok.is_empty() => toast(&weak, ok),
        Ok(_) => {}
        Err(e) => toast(&weak, e.message),
    }
}

/// Everything the UI-thread controllers share.
pub struct Ctx {
    pub app: AppWindow,
    pub bridge: std::sync::Arc<Bridge>,
    pub state: Shared,
    pub paths: JidrPaths,
    pub prefs: RefCell<crate::settings::Prefs>,
    /// Secondary windows (setup, settings) kept alive while shown.
    pub windows: RefCell<Vec<Box<dyn std::any::Any>>>,
    /// Catalog for the create sheet, loaded when it opens.
    pub catalog: RefCell<Catalog>,
    /// What the confirmation dialog will do.
    pub pending_confirm: RefCell<Option<Box<dyn Fn()>>>,
    /// The device window's live session.
    pub player: RefCell<crate::player::Player>,
    /// The device a refused start was for, and the daemon's details (D42).
    pub pending_exhausted: RefCell<Option<(Uuid, crate::model::Exhausted)>>,
}

#[derive(Default)]
pub struct Catalog {
    pub profiles: Vec<jidr_core::HardwareProfile>,
    pub images: Vec<jidr_core::SystemImage>,
    /// Device being edited.
    pub editing: Option<Uuid>,
    /// The edit sheet's auto-hibernate choices.
    pub hibernate: crate::device_form::HibernateChoices,
}

impl Ctx {
    pub fn device_id(&self, index: i32) -> Option<Uuid> {
        let st = self.state.borrow();
        usize::try_from(index)
            .ok()
            .and_then(|i| st.data.devices.get(i))
            .map(|d| d.device.id)
    }

    pub fn info(&self, id: Uuid) -> Option<crate::model::DeviceInfo> {
        self.state
            .borrow()
            .data
            .devices
            .iter()
            .find(|d| d.device.id == id)
            .cloned()
    }

    pub fn rerender(self: &Rc<Self>) {
        render(&self.app, &self.state.borrow());
        crate::player::sync(self);
        crate::root::sync(self);
    }

    /// Applies the Appearance pref (System follows macOS).
    pub fn apply_appearance(&self) {
        let dark = match self.prefs.borrow().appearance {
            1 => false,
            2 => true,
            _ => system_dark(),
        };
        self.app.global::<Theme>().set_dark(dark);
    }
}

thread_local! {
    /// The UI thread's state; IO-thread updates reach it via the event loop.
    static STATE: Shared = Rc::default();
    /// The controllers' context, once the app runs.
    static CTX: RefCell<Option<Rc<Ctx>>> = const { RefCell::new(None) };
}

/// The running app's context (UI thread only).
pub fn ctx() -> Option<Rc<Ctx>> {
    CTX.with(|c| c.borrow().clone())
}

/// The UI thread's shared state.
pub fn state() -> Shared {
    STATE.with(Rc::clone)
}

/// Feeds bridge updates into the window (called on the IO thread).
pub fn sink(weak: Weak<AppWindow>) -> impl Fn(Update) + Send + Sync + 'static {
    move |u: Update| {
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(app) = weak.upgrade() else { return };
            match u {
                Update::Conn(conn) => {
                    if matches!(conn, Conn::Online)
                        && let Some(c) = ctx()
                    {
                        crate::settings::sync_from_daemon(&c);
                    }
                    let (s, text) = match conn {
                        Conn::Connecting => (
                            ConnState::Connecting,
                            "Connecting to the daemon…".to_string(),
                        ),
                        Conn::Online => (ConnState::Online, String::new()),
                        Conn::Reconnecting(t) => (ConnState::Reconnecting, t),
                    };
                    app.set_conn(s);
                    app.set_conn_text(text.into());
                }
                Update::Data(data) => apply_data(&app, data),
                Update::Event(_) => {}
            }
        });
    }
}

/// Takes a fresh device list from the daemon (UI thread).
pub fn apply_data(app: &AppWindow, data: Data) {
    let state = state();
    let mut st = state.borrow_mut();
    let ids: Vec<Uuid> = data.devices.iter().map(|d| d.device.id).collect();
    st.tabs.retain(|t| ids.contains(t));
    if st.active.is_some_and(|a| a >= st.tabs.len()) {
        st.active = None;
    }
    st.data = data;
    render(app, &st);
    drop(st);
    if let Some(c) = ctx() {
        crate::player::sync(&c);
        // Starts a root setup queued for a device that just arrived (a new
        // Full-root device), instead of waiting for the next selection.
        crate::root::sync(&c);
    }
}

/// Starts the GUI: native window, theme from the system, daemon connection.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    select_backend()?;
    let app = AppWindow::new()?;
    app.set_draw_lights(!cfg!(target_os = "macos"));
    app.global::<Theme>()
        .set_reduce_motion(system_reduce_motion());

    let paths = JidrPaths::discover()?;
    let bridge = Bridge::new()?;
    let ctx = Rc::new(Ctx {
        app: app.clone_strong(),
        bridge: bridge.clone(),
        state: state(),
        prefs: RefCell::new(crate::settings::Prefs::load(&paths.data)),
        paths: paths.clone(),
        windows: RefCell::default(),
        catalog: RefCell::default(),
        pending_confirm: RefCell::default(),
        player: RefCell::default(),
        pending_exhausted: RefCell::default(),
    });
    CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
    ctx.apply_appearance();
    crate::launcher::wire(&ctx);
    crate::player::wire(&ctx);
    crate::widgets::wire(&ctx);
    crate::snapshots::wire(&ctx);
    crate::root::wire(&ctx);
    auto_open(&ctx);
    bridge.connect(paths, sink(app.as_weak()));
    app.run()?;
    Ok(())
}

/// Dev hook: `JIDR_PLAYER_DEVICE=<name>` opens that device's tab (starting
/// it if needed) once the daemon's device list arrives.
fn auto_open(ctx: &Rc<Ctx>) {
    let Ok(name) = std::env::var("JIDR_PLAYER_DEVICE") else {
        return;
    };
    let c = ctx.clone();
    let timer = slint::Timer::default();
    let started = std::cell::Cell::new(false);
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(500),
        move || {
            let found = c
                .state
                .borrow()
                .data
                .devices
                .iter()
                .find(|d| d.device.name.eq_ignore_ascii_case(&name))
                .map(|d| (d.device.id, d.device.status.clone()));
            let Some((id, status)) = found else { return };
            if c.state.borrow().tabs.contains(&id) {
                return;
            }
            if matches!(status, jidr_core::DeviceStatus::Stopped) && !started.get() {
                started.set(true);
                crate::launcher::start(&c, id);
            }
            crate::launcher::open_tab(&c, id);
        },
    );
    ctx.windows.borrow_mut().push(Box::new(timer));
}

fn select_backend() -> Result<(), slint::PlatformError> {
    let selector = slint::BackendSelector::new().backend_name("winit".into());
    #[cfg(target_os = "macos")]
    let selector = selector.with_winit_window_attributes_hook(|attrs| {
        use slint::winit_030::winit::platform::macos::WindowAttributesExtMacOS;
        attrs
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true)
            .with_title_hidden(true)
    });
    selector.select()
}

/// A UI-thread continuation waiting for a value from the IO thread.
pub type Token = u64;

type Continuation = Box<dyn FnOnce(Box<dyn std::any::Any + Send>)>;

thread_local! {
    static PENDING: RefCell<(Token, std::collections::HashMap<Token, Continuation>)> =
        RefCell::new((0, std::collections::HashMap::new()));
}

/// Parks `f` (which may hold UI-thread values such as `Rc<Ctx>`) and
/// returns a `Send` token; `fire(token, value)` runs it on the UI thread.
pub fn defer<T: Send + 'static>(f: impl FnOnce(T) + 'static) -> Token {
    PENDING.with(|p| {
        let mut p = p.borrow_mut();
        p.0 += 1;
        let token = p.0;
        p.1.insert(
            token,
            Box::new(move |v: Box<dyn std::any::Any + Send>| {
                if let Ok(v) = v.downcast::<T>() {
                    f(*v);
                }
            }),
        );
        token
    })
}

/// Runs a deferred continuation on the UI thread (callable from any thread).
pub fn fire<T: Send + 'static>(token: Token, value: T) {
    let _ = slint::invoke_from_event_loop(move || {
        let f = PENDING.with(|p| p.borrow_mut().1.remove(&token));
        if let Some(f) = f {
            f(Box::new(value));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RootUi;
    use jidr_core::RootMode;
    use slint::platform::{Key, PointerEventButton, WindowEvent};

    #[test]
    fn a_new_full_root_device_starts_its_setup_when_the_list_brings_it_in() {
        // Regression: `root.set magisk` for a just-created device waited in
        // the queue until the person selected it (only a re-render drained
        // the queue), so the device stayed Stopped.
        crate::offscreen::init().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let app = AppWindow::new().unwrap();
        let paths = JidrPaths::under(tmp.path());
        let ctx = Rc::new(Ctx {
            app: app.clone_strong(),
            bridge: Bridge::new().unwrap(),
            state: state(),
            prefs: RefCell::default(),
            paths,
            windows: RefCell::default(),
            catalog: RefCell::default(),
            pending_confirm: RefCell::default(),
            player: RefCell::default(),
            pending_exhausted: RefCell::default(),
        });
        CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));

        let tablet = crate::fixtures::device_infos()
            .into_iter()
            .find(|i| i.device.name == "Tablet 11")
            .unwrap();
        crate::root::start(&ctx, tablet.device.id, RootMode::Magisk);
        assert!(!app.global::<RootUi>().get_open(), "queued, not started");

        apply_data(
            &app,
            Data {
                devices: vec![tablet],
                ..Data::default()
            },
        );
        assert!(
            app.global::<RootUi>().get_open(),
            "the Root sheet opens and root.set runs once the device is listed"
        );
    }

    /// Presses `text` with ⌘ held. On macOS Slint's winit backend reports ⌘
    /// as Control (it swaps ⌘ and ⌃ like Qt), so ⌘ is dispatched as
    /// `Key::Control` here, the way the real window delivers it.
    fn press_cmd(app: &AppWindow, text: &str) {
        let w = app.window();
        let ctrl: slint::SharedString = Key::Control.into();
        w.dispatch_event(WindowEvent::KeyPressed { text: ctrl.clone() });
        w.dispatch_event(WindowEvent::KeyPressed { text: text.into() });
        w.dispatch_event(WindowEvent::KeyReleased { text: text.into() });
        w.dispatch_event(WindowEvent::KeyReleased { text: ctrl });
    }

    fn record_shortcuts(app: &AppWindow) -> Rc<RefCell<Vec<String>>> {
        let fired = Rc::new(RefCell::new(Vec::new()));
        let f = fired.clone();
        app.on_shortcut(move |s| f.borrow_mut().push(s.to_string()));
        fired
    }

    #[test]
    fn cmd_shortcuts_fire_from_the_window() {
        // Regression: shortcuts checked `modifiers.meta`, which is ⌃ on macOS,
        // so ⌘L, ⌘J, ⌘W… never fired.
        crate::offscreen::init().unwrap();
        let app = crate::goldens::populated(crate::goldens::Variant::Dark).unwrap();
        app.show().unwrap();
        let fired = record_shortcuts(&app);

        press_cmd(&app, "1");
        press_cmd(&app, "j");
        assert_eq!(*fired.borrow(), ["tab-1", "panel"]);
    }

    #[test]
    fn cmd_shortcuts_fire_from_the_device_screen_and_do_not_reach_the_device() {
        crate::offscreen::init().unwrap();
        let app = crate::goldens::device_tab(crate::goldens::Variant::Dark, 0).unwrap();
        app.show().unwrap();
        crate::offscreen::render(app.window(), 1440, 900).unwrap();
        let fired = record_shortcuts(&app);
        let keys = Rc::new(RefCell::new(Vec::new()));
        let k = keys.clone();
        app.on_player_key(move |t, down| k.borrow_mut().push((t.to_string(), down)));

        // A click on the screen focuses its key scope.
        let size = app.window().size();
        let at = slint::LogicalPosition::new(size.width as f32 * 0.4, size.height as f32 * 0.4);
        let w = app.window();
        let button = PointerEventButton::Left;
        w.dispatch_event(WindowEvent::PointerPressed {
            position: at,
            button,
        });
        w.dispatch_event(WindowEvent::PointerReleased {
            position: at,
            button,
        });
        w.dispatch_event(WindowEvent::KeyPressed { text: "x".into() });
        assert_eq!(
            *keys.borrow(),
            [("x".to_string(), true)],
            "the screen has focus"
        );

        press_cmd(&app, "l");
        assert_eq!(*fired.borrow(), ["logcat"]);
        assert_eq!(keys.borrow().len(), 1, "⌘L is not sent to the device");
    }
}
