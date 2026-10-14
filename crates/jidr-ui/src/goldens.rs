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

/// The device tab for `device` (0 = Pixel-ish 6.1) with the mock screen.
fn device_tab(v: Variant, device: usize) -> Result<AppWindow, String> {
    let app = populated(v)?;
    let devices = fixtures::devices();
    app.set_tabs(model(vec![
        model::tab_vm(&devices[0]),
        model::tab_vm(&devices[2]),
        model::tab_vm(&devices[1]),
    ]));
    app.set_active_tab(match device {
        0 => 0,
        2 => 1,
        _ => 2,
    });
    app.set_active_device(device as i32);
    app.set_player_mock(true);
    let lines = fixtures::log_lines();
    app.set_log_errors(1);
    app.set_log_last_error(crate::player::logcat::short(&lines[3]).into());
    app.set_log_summary("5 of 12,408 lines · 1 error".into());
    app.set_log_lines(model(lines));
    let mut w = crate::widgets::default_vm();
    w.battery_level = 64;
    w.wifi = true;
    w.mobile = true;
    w.signal = 3;
    w.phone_number = "+218 91 234 5678".into();
    w.sms_text = "Your courier is 4 min away".into();
    w.clipboard = "ORDER-4821".into();
    app.set_widgets(w);
    Ok(app)
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
            name: "snapshots-manager",
            render: |v| {
                let app = populated(v)?;
                app.set_snapshots_open(true);
                app.set_snap_banner("Every snapshot remembers the root mode it was taken with. Pixel-ish 6.1 is Full root now, so all 2 can be restored. 4.5 GB in total.".into());
                shot(&app, 1440, 900)
            },
        },
        Case {
            name: "snapshots-refusal",
            render: |v| {
                let app = populated(v)?;
                app.set_snapshots(model(fixtures::snapshots(RootMode::None)));
                app.set_snapshots_open(true);
                app.set_snap_banner_warning(true);
                app.set_snap_banner("Pixel-ish 6.1 is Not rooted now. Snapshots taken with Full root can't be restored until Full root is set up again.".into());
                app.set_refuse_name("Clean install".into());
                app.set_refuse_snapshot_line("Taken with: Full root · Magisk 30.7".into());
                app.set_refuse_device_line("Not rooted".into());
                app.set_refuse_body("It was taken with Full root. Pixel-ish 6.1 is Not rooted now, so restoring it would bring back a root setup Jidr hasn't verified on this device.".into());
                app.set_refuse_hint("Set up Full root first, then restore. Snapshots taken while Not rooted restore as usual.".into());
                app.set_refuse_action("Set up Full root…".into());
                app.set_refuse_open(true);
                shot(&app, 1440, 900)
            },
        },
        Case {
            name: "snapshots-save",
            render: |v| {
                let app = populated(v)?;
                app.set_snapshots_open(true);
                app.set_save_name("Before checkout test".into());
                app.set_save_root_kind(crate::RootKind::Full);
                app.set_save_root_tag("Taken with: Full root".into());
                app.set_save_hint("Its other snapshots are 2.1 and 2.4 GB.".into());
                app.set_save_open(true);
                shot(&app, 1440, 900)
            },
        },
        Case {
            name: "player-running",
            render: |v| shot(&device_tab(v, 0)?, 1440, 900),
        },
        Case {
            name: "player-running-1280",
            render: |v| shot(&device_tab(v, 0)?, 1280, 800),
        },
        Case {
            name: "player-pixel-perfect",
            render: |v| {
                let app = device_tab(v, 0)?;
                app.set_player_pixel_perfect(true);
                app.set_player_scale(2.0);
                shot(&app, 1440, 900)
            },
        },
        Case {
            name: "player-apk-drop",
            render: |v| {
                let app = device_tab(v, 0)?;
                let (t, b) = crate::player::drop::hover_text(
                    &crate::player::drop::DropAction::InstallApk("/x/shop-debug.apk".into()),
                    "Pixel-ish 6.1",
                );
                app.set_drop_title(t.into());
                app.set_drop_body(b.into());
                app.set_drop_hover(true);
                shot(&app, 1440, 900)
            },
        },
        Case {
            name: "player-logcat",
            render: |v| {
                let app = device_tab(v, 0)?;
                app.set_logcat_open(true);
                app.set_log_level(0);
                shot(&app, 1440, 900)
            },
        },
        Case {
            name: "player-overlay",
            render: |v| {
                let app = device_tab(v, 0)?;
                app.set_player_overlay(true);
                app.set_player_overlay_text(
                    "recv 60 fps · shown 60 fps · active 60 fps\nframe 800×1778 · age 5.4 ms · touch→frame 13.5 ms".into(),
                );
                app.set_player_panel_open(false);
                shot(&app, 1440, 900)
            },
        },
        Case {
            name: "widgets-gps",
            render: |v| {
                let app = device_tab(v, 0)?;
                let mut w = app.get_widgets();
                w.gps_lat = "32.8954".into();
                w.gps_lon = "13.1802".into();
                w.gps_alt = "21".into();
                w.gps_bearing = "168".into();
                w.route_name = "tripoli-airport-run.gpx".into();
                w.route_active = true;
                w.route_progress = 0.38;
                w.route_speed = 1;
                w.route_text = "4.1 / 10.8 km · 6 min left at 2× · bearing 168°".into();
                app.set_widgets(w);
                let c = crate::widgets::gps::world_px(32.8954, 13.1802, crate::widgets::gps::ZOOM);
                app.set_gps_route_path(
                    crate::widgets::gps::route_path(
                        &[
                            (32.8990, 13.1720),
                            (32.8954, 13.1802),
                            (32.8925, 13.1830),
                            (32.8890, 13.1842),
                            (32.8850, 13.1880),
                        ],
                        c,
                    )
                    .into(),
                );
                app.set_player_panel_tab(1);
                shot(&app, 1440, 900)
            },
        },
        Case {
            name: "widgets-network",
            render: |v| {
                let app = device_tab(v, 0)?;
                app.set_player_panel_tab(2);
                shot(&app, 1440, 900)
            },
        },
        Case {
            name: "widgets-sensors",
            render: |v| {
                let app = device_tab(v, 0)?;
                app.set_player_panel_tab(3);
                shot(&app, 1440, 900)
            },
        },
        Case {
            name: "widgets-more",
            render: |v| {
                let app = device_tab(v, 0)?;
                app.set_player_panel_tab(4);
                shot(&app, 1440, 900)
            },
        },
        Case {
            name: "player-recording",
            render: |v| {
                let app = device_tab(v, 0)?;
                app.set_player_recording(true);
                app.set_player_record_text("Recording 1:24".into());
                shot(&app, 1440, 900)
            },
        },
        Case {
            name: "player-booting",
            render: |v| {
                let app = device_tab(v, 2)?;
                app.set_player_mock(false);
                shot(&app, 1440, 900)
            },
        },
        Case {
            name: "player-stopped",
            render: |v| {
                let app = device_tab(v, 1)?;
                app.set_player_mock(false);
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
