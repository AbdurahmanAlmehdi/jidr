//! The golden snapshot cases: every screen and state, filled with the
//! PRODUCT.md §7 fixtures. Rendered by `examples/snapshot.rs`.

use jidr_core::RootMode;
use slint::{ComponentHandle, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

use crate::{AppWindow, ConnState, Theme, fixtures, model, offscreen};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Dark,
    Light,
    /// Dark, mirrored for right-to-left (D33).
    Rtl,
}

impl Variant {
    pub const ALL: [Variant; 3] = [Variant::Dark, Variant::Light, Variant::Rtl];

    pub fn suffix(self) -> &'static str {
        match self {
            Variant::Dark => "dark",
            Variant::Light => "light",
            Variant::Rtl => "rtl",
        }
    }
}

pub type Render = fn(Variant) -> Result<SharedPixelBuffer<Rgba8Pixel>, String>;

pub struct Case {
    pub name: &'static str,
    pub render: Render,
}

/// Applies a variant to a component's `Theme` global. Snapshots freeze
/// motion (spinners, transitions) by turning Reduce Motion on.
pub fn apply<C: ComponentHandle>(c: &C, v: Variant)
where
    for<'a> Theme<'a>: slint::Global<'a, C>,
{
    let theme = c.global::<Theme>();
    theme.set_dark(v != Variant::Light);
    theme.set_rtl(v == Variant::Rtl);
    theme.set_reduce_motion(true);
}

fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(v))
}

fn app(v: Variant) -> Result<AppWindow, String> {
    let app = AppWindow::new().map_err(|e| e.to_string())?;
    apply(&app, v);
    Ok(app)
}

fn shot(app: &AppWindow, w: u32, h: u32) -> Result<SharedPixelBuffer<Rgba8Pixel>, String> {
    offscreen::render(app.window(), w, h).map_err(|e| e.to_string())
}

/// Launcher with the four sample devices, Pixel-ish 6.1 selected.
pub fn populated(v: Variant) -> Result<AppWindow, String> {
    let app = app(v)?;
    let devices = fixtures::devices();
    let tabs = vec![model::tab_vm(&devices[0])];
    app.set_devices(model(devices));
    app.set_jobs(model(fixtures::jobs()));
    app.set_tabs(model(tabs));
    app.set_selected(0);
    app.set_snapshots(model(fixtures::snapshots(RootMode::Magisk)));
    Ok(app)
}

pub fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "launcher-first-run",
            render: |v| {
                let app = app(v)?;
                app.set_sdk_ready(false);
                shot(&app, 1440, 900)
            },
        },
        Case {
            name: "launcher-populated",
            render: |v| shot(&populated(v)?, 1440, 900),
        },
        Case {
            name: "launcher-populated-1280",
            render: |v| shot(&populated(v)?, 1280, 800),
        },
        Case {
            name: "launcher-reconnecting",
            render: |v| {
                let app = populated(v)?;
                app.set_conn(ConnState::Reconnecting);
                app.set_conn_text("Reconnecting to the daemon…".into());
                shot(&app, 1440, 900)
            },
        },
    ]
}
