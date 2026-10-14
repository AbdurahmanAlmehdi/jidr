//! The golden snapshot cases: every screen and state, filled with the
//! PRODUCT.md §7 fixtures. Rendered by `examples/snapshot.rs`.

use jidr_core::RootMode;
use slint::{ComponentHandle, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

use crate::{
    AppWindow, ConnState, DetectRow, JobTone, JobVm, PlanRow, RootChoice, SettingsWindow,
    SetupWindow, Theme, device_form, fixtures, model, offscreen, palette, settings,
};

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

fn shot<C: ComponentHandle>(
    c: &C,
    w: u32,
    h: u32,
) -> Result<SharedPixelBuffer<Rgba8Pixel>, String> {
    offscreen::render(c.window(), w, h).map_err(|e| e.to_string())
}

fn create_sheet(
    v: Variant,
    play_store: bool,
    edit: bool,
) -> Result<SharedPixelBuffer<Rgba8Pixel>, String> {
    let app = populated(v)?;
    let profiles = fixtures::profiles();
    let images = fixtures::system_images();
    let jobs = vec![fixtures::download_job()];
    app.set_profiles(model(device_form::profile_options(&profiles)));
    app.set_images(model(device_form::image_rows(&images, &jobs)));
    let mut form = device_form::new_form(&profiles, &images, &[], RootChoice::Shell);
    if edit {
        form = device_form::edit_form(&fixtures::device_infos()[1], &profiles);
    } else if play_store {
        form.name = "Play Store test 2".into();
        form.image = images
            .iter()
            .position(|i| i.image.api == 33)
            .map(|i| i as i32)
            .unwrap_or(0);
        form.root = RootChoice::None;
    } else {
        form.name = "Checkout test".into();
    }
    app.set_form(form);
    app.set_sheet_open(true);
    shot(&app, 1440, 900)
}

fn setup(v: Variant, step: i32) -> Result<SharedPixelBuffer<Rgba8Pixel>, String> {
    let w = SetupWindow::new().map_err(|e| e.to_string())?;
    apply(&w, v);
    w.set_step(step);
    w.set_detect(model(vec![
        DetectRow {
            path: "~/Library/Android/sdk".into(),
            state: "Not found".into(),
            found: false,
        },
        DetectRow {
            path: "$ANDROID_HOME".into(),
            state: "Not set".into(),
            found: false,
        },
        DetectRow {
            path: "$ANDROID_SDK_ROOT".into(),
            state: "Not set".into(),
            found: false,
        },
    ]));
    w.set_plan(model(vec![
        PlanRow {
            label: "Android Emulator".into(),
            size: "0.3 GB".into(),
        },
        PlanRow {
            label: "Android 15 (API 35) · Google APIs · arm64".into(),
            size: "2.0 GB".into(),
        },
    ]));
    w.set_plan_dir("~/Library/Application Support/Jidr/sdk".into());
    w.set_plan_total("2.3 GB".into());
    w.set_license_title("Android Software Development Kit License Agreement".into());
    w.set_license_text(fixtures::LICENSE_EXCERPT.into());
    w.set_accepted_note(
        "License accepted Oct 13, 14:01. You can close this window; downloads keep going in Jobs."
            .into(),
    );
    let row = |title: &str, tone, p: f32, detail: &str, eta: &str| JobVm {
        id: "".into(),
        title: title.into(),
        tone,
        progress: p,
        indeterminate: false,
        detail: detail.into(),
        eta: eta.into(),
        cancellable: false,
    };
    w.set_downloads(model(vec![
        row(
            "Android Emulator",
            JobTone::Done,
            1.0,
            "0.3 / 0.3 GB",
            "Done",
        ),
        row(
            "Android 15 (API 35) image",
            JobTone::Running,
            0.6,
            "1.2 / 2.0 GB · 18 MB/s",
            "About 1 min left",
        ),
        row(
            "Verify checksums",
            JobTone::Queued,
            0.0,
            "Queued",
            "After download",
        ),
    ]));
    let profiles = fixtures::profiles();
    let images: Vec<_> = fixtures::system_images()
        .into_iter()
        .filter(|i| i.image.api == 35 && i.installed_in.is_some())
        .collect();
    let mut form = device_form::new_form(&profiles, &images, &[], RootChoice::Shell);
    form.name = "Pixel-ish 6.1".into();
    w.set_form(form);
    w.set_profiles(model(device_form::profile_options(&profiles)));
    w.set_image_label("Android 15 · API 35 · Google APIs".into());
    w.set_image_status("Installed · 2.0 GB".into());
    shot(&w, 760, 580)
}

fn settings_tab(v: Variant, tab: i32) -> Result<SharedPixelBuffer<Rgba8Pixel>, String> {
    let w = SettingsWindow::new().map_err(|e| e.to_string())?;
    apply(&w, v);
    w.set_tab(tab);
    w.set_version("0.1.0".into());
    w.set_tile_options(model(settings::tile_options()));
    w.set_sdk_path("~/Library/Application Support/Jidr/sdk".into());
    w.set_sdk_note("License accepted Oct 13, 14:01".into());
    let infos = fixtures::device_infos();
    w.set_device_usage(model(settings::device_usage(
        &infos,
        &[(uuid::Uuid::from_u128(1), 3)],
    )));
    w.set_image_usage(model(settings::image_usage(
        &fixtures::system_images(),
        &infos,
    )));
    w.set_cache_size("0.4 GB".into());
    shot(&w, 820, 640)
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
            name: "launcher-empty",
            render: |v| shot(&app(v)?, 1440, 900),
        },
        Case {
            name: "launcher-error",
            render: |v| {
                let app = app(v)?;
                let mut infos = fixtures::device_infos();
                infos[1].device.status = jidr_core::DeviceStatus::Crashed {
                    reason: "emulator exited with code 1".into(),
                    log: None,
                };
                infos[0].device.root.health = jidr_core::RootHealth::Failed;
                let devices: Vec<_> = infos
                    .iter()
                    .map(|i| model::device_vm(i, &fixtures::clock()))
                    .collect();
                app.set_devices(model(devices));
                app.set_selected(0);
                app.set_toast("Root failed: patch step timed out. The device is unchanged.".into());
                shot(&app, 1440, 900)
            },
        },
        Case {
            name: "create-google-apis",
            render: |v| create_sheet(v, false, false),
        },
        Case {
            name: "create-play-store",
            render: |v| create_sheet(v, true, false),
        },
        Case {
            name: "edit-device",
            render: |v| create_sheet(v, false, true),
        },
        Case {
            name: "palette",
            render: |v| {
                let app = populated(v)?;
                let items = palette::items(&fixtures::devices(), true);
                app.set_palette_items(model(items));
                app.set_palette_open(true);
                shot(&app, 1440, 900)
            },
        },
        Case {
            name: "palette-filtered",
            render: |v| {
                let app = populated(v)?;
                let items = palette::filter(&palette::items(&fixtures::devices(), true), "tab");
                app.set_palette_query("tab".into());
                app.set_palette_items(model(items));
                app.set_palette_open(true);
                shot(&app, 1440, 900)
            },
        },
        Case {
            name: "setup-1-find-sdk",
            render: |v| setup(v, 1),
        },
        Case {
            name: "setup-2-license",
            render: |v| setup(v, 2),
        },
        Case {
            name: "setup-3-download",
            render: |v| setup(v, 3),
        },
        Case {
            name: "setup-4-first-device",
            render: |v| setup(v, 4),
        },
        Case {
            name: "settings-general",
            render: |v| settings_tab(v, 0),
        },
        Case {
            name: "settings-storage",
            render: |v| settings_tab(v, 1),
        },
        Case {
            name: "settings-agents",
            render: |v| settings_tab(v, 2),
        },
        Case {
            name: "settings-about",
            render: |v| settings_tab(v, 3),
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
