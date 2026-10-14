//! The running app: wires `AppWindow` callbacks to daemon calls and pushes
//! daemon data back into the view models. View state (selection, tabs,
//! theme) stays here (plan: UI-only state is not a protocol command).

use std::cell::RefCell;
use std::rc::Rc;

use jidr_core::{JidrPaths, JidrResult};
use serde_json::{Value, json};
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
            j.kind == "image.install" || j.kind == "sdk.install" || j.kind.starts_with("root.")
        })
        .map(|j| model::job_vm(j, &job_title(j), None, None))
        .collect();
    app.set_devices(ModelRc::new(VecModel::from(vms)));
    app.set_jobs(ModelRc::new(VecModel::from(jobs)));
    app.set_tabs(ModelRc::new(VecModel::from(tabs)));
    app.set_selected(selected.map(|i| i as i32).unwrap_or(-1));
    app.set_active_tab(state.active.map(|i| i as i32).unwrap_or(-1));
    app.set_sdk_ready(state.data.sdk_ready);
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

pub struct Ctx {
    pub app: AppWindow,
    pub bridge: std::sync::Arc<Bridge>,
    pub state: Shared,
}

impl Ctx {
    fn device_id(&self, index: i32) -> Option<Uuid> {
        let st = self.state.borrow();
        usize::try_from(index)
            .ok()
            .and_then(|i| st.data.devices.get(i))
            .map(|d| d.device.id)
    }

    fn rerender(&self) {
        render(&self.app, &self.state.borrow());
    }
}

/// Wires the launcher callbacks.
pub fn wire(ctx: &Rc<Ctx>) {
    let app = &ctx.app;
    let weak = app.as_weak();

    let c = ctx.clone();
    app.on_select_device(move |i| {
        let id = c.device_id(i);
        c.state.borrow_mut().selected = id;
        c.rerender();
    });

    let c = ctx.clone();
    app.on_open_device(move |i| {
        let Some(id) = c.device_id(i) else { return };
        {
            let mut st = c.state.borrow_mut();
            st.selected = Some(id);
            let pos = match st.tabs.iter().position(|t| *t == id) {
                Some(p) => p,
                None => {
                    st.tabs.push(id);
                    st.tabs.len() - 1
                }
            };
            st.active = Some(pos);
        }
        c.rerender();
    });

    let c = ctx.clone();
    app.on_select_tab(move |i| {
        c.state.borrow_mut().active = usize::try_from(i).ok();
        c.rerender();
    });

    let c = ctx.clone();
    app.on_close_tab(move |i| {
        {
            let mut st = c.state.borrow_mut();
            if let Ok(i) = usize::try_from(i)
                && i < st.tabs.len()
            {
                st.tabs.remove(i);
            }
            st.active = None;
        }
        c.rerender();
    });

    let c = ctx.clone();
    let w = weak.clone();
    app.on_start_device(move |i| {
        let Some(id) = c.device_id(i) else { return };
        c.bridge.run(
            "device.start",
            json!({ "device": id }),
            |_| {},
            report(&w, ""),
        );
    });

    let c = ctx.clone();
    let w = weak.clone();
    app.on_stop_device(move |i| {
        let Some(id) = c.device_id(i) else { return };
        c.bridge
            .call("device.stop", json!({ "device": id }), report(&w, ""));
    });

    let c = ctx.clone();
    let w = weak.clone();
    app.on_cancel_job(move |i| {
        let job = {
            let st = c.state.borrow();
            usize::try_from(i)
                .ok()
                .and_then(|i| st.data.jobs.get(i))
                .map(|j| j.id)
        };
        if let Some(job) = job {
            c.bridge.call(
                "jobs.cancel",
                json!({ "job": job }),
                report(&w, "Cancelled"),
            );
        }
    });

    let w = weak.clone();
    app.on_drag_window(move || {
        if let Some(app) = w.upgrade() {
            use slint::winit_030::WinitWindowAccessor;
            app.window().with_winit_window(|win| {
                let _ = win.drag_window();
            });
        }
    });
}

thread_local! {
    /// The UI thread's state; IO-thread updates reach it via the event loop.
    static STATE: Shared = Rc::default();
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
                Update::Data(data) => {
                    let state = state();
                    let mut st = state.borrow_mut();
                    let ids: Vec<Uuid> = data.devices.iter().map(|d| d.device.id).collect();
                    st.tabs.retain(|t| ids.contains(t));
                    if st.active.is_some_and(|a| a >= st.tabs.len()) {
                        st.active = None;
                    }
                    st.data = data;
                    render(&app, &st);
                }
                Update::Event(_) => {}
            }
        });
    }
}

/// Starts the GUI: native window, theme from the system, daemon connection.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    select_backend()?;
    let app = AppWindow::new()?;
    app.set_draw_lights(!cfg!(target_os = "macos"));
    let theme = app.global::<Theme>();
    theme.set_dark(system_dark());
    theme.set_reduce_motion(system_reduce_motion());

    let paths = JidrPaths::discover()?;
    let bridge = Bridge::new()?;
    let ctx = Rc::new(Ctx {
        app: app.clone_strong(),
        bridge: bridge.clone(),
        state: state(),
    });
    wire(&ctx);
    bridge.connect(paths, sink(app.as_weak()));
    app.run()?;
    Ok(())
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
