//! Golden scenes for every root state (Phase 9e acceptance): the Root sheet
//! in progress, rooted, failed, Play Store first boot, guided tap and
//! unverified, the dev-tools root section, and the new root-mode cards.
//! `crate::goldens::cases()` appends these (gap B8).

use std::time::{Duration, Instant};

use jidr_core::{ErrorCode, JidrError, RootHealth};
use serde_json::json;
use slint::ComponentHandle;

use super::steps::{self, LogEntry};
use super::{Outcome, Sheet, apply, sheet_vm, tools_vm};
use crate::RootUi;
use crate::fixtures;
use crate::goldens::{Case, Variant, device_tab, shot};

/// Device fixtures: 0 is "Pixel-ish 6.1" (google_apis, rooted, running);
/// 2 is "Play Store test" (Play Store image, booting at 62%).
const PIXEL: usize = 0;
const PLAY_STORE: usize = 2;

fn log(lines: &[(&str, &'static str, &str)]) -> Vec<LogEntry> {
    lines
        .iter()
        .map(|(t, level, text)| LogEntry {
            time: (*t).into(),
            source: "jidr",
            level,
            text: (*text).into(),
        })
        .collect()
}

/// The device window of fixture `device` with the Root sheet on top.
fn sheet(
    v: Variant,
    device: usize,
    s: Sheet,
    elapsed: u64,
) -> Result<slint::SharedPixelBuffer<slint::Rgba8Pixel>, String> {
    let app = device_tab(v, device)?;
    // The window behind the sheet shows the same root state.
    let mut infos = fixtures::device_infos();
    let root = &mut infos[device].device.root;
    match &s.outcome {
        None => infos[device].root_job = true,
        Some(Outcome::Rooted { .. }) => {
            root.health = RootHealth::Ok;
            root.verified_at = Some(fixtures::now());
        }
        Some(Outcome::Failed { failure, .. }) if failure.unverified => {
            root.health = RootHealth::Unverified;
            root.verified_at = Some(fixtures::now());
        }
        Some(Outcome::Failed { .. }) => root.health = RootHealth::Failed,
    }
    if s.pending {
        infos[device].root_job = false;
    }
    let clock = fixtures::clock();
    app.set_devices(slint::ModelRc::new(slint::VecModel::from(
        infos
            .iter()
            .map(|i| crate::model::device_vm(i, &clock))
            .collect::<Vec<_>>(),
    )));
    let info = infos.swap_remove(device);
    let s = Sheet {
        device: Some(info.device.id),
        ..s
    };
    let base = s.started.unwrap_or_else(Instant::now);
    let vm = sheet_vm(&s, Some(&info), base + Duration::from_secs(elapsed));
    let ui = app.global::<RootUi>();
    apply(&ui, &vm);
    ui.set_open(true);
    shot(&app, 1440, 900)
}

/// A job on `step`, rendered `total` seconds after it started, with the
/// step's clock at `step_secs`.
fn running(step: usize, total: u64, step_secs: u64) -> Sheet {
    let t0 = Instant::now();
    Sheet {
        step,
        started: Some(t0),
        step_started: Some(t0 + Duration::from_secs(total.saturating_sub(step_secs))),
        ..Sheet::default()
    }
}

pub fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "root-progress",
            render: |v| sheet(v, PIXEL, running(steps::REBOOT, 58, 18), 58),
        },
        Case {
            name: "root-success",
            render: |v| {
                let s = Sheet {
                    outcome: Some(Outcome::Rooted {
                        secs: 58,
                        magisk: Some("30.7".into()),
                        uid: Some(0),
                    }),
                    ..running(steps::VERIFY, 58, 0)
                };
                sheet(v, PIXEL, s, 58)
            },
        },
        Case {
            name: "root-failed",
            render: |v| {
                let e = JidrError::new(
                    ErrorCode::BootTimeout,
                    "jidr-pixel-ish did not boot within 120 s (boot); see logs",
                );
                let s = Sheet {
                    log: log(&[
                        ("14:20:31", "", "Cold booting"),
                        ("14:21:02", "w", "Waiting for sys.boot_completed"),
                        ("14:22:31", "e", "boot_timeout: did not boot within 120 s"),
                        ("14:22:31", "", "root setup stopped at step 3 of 5"),
                    ]),
                    outcome: Some(Outcome::Failed {
                        failure: steps::failure(&e, steps::REBOOT),
                        secs: 160,
                        detail: super::detail("Pixel-ish 6.1", &e),
                    }),
                    ..running(steps::REBOOT, 160, 120)
                };
                sheet(v, PIXEL, s, 160)
            },
        },
        Case {
            name: "root-playstore-beta",
            render: |v| {
                sheet(
                    v,
                    PLAY_STORE,
                    Sheet {
                        pending: true,
                        ..Sheet::default()
                    },
                    0,
                )
            },
        },
        Case {
            name: "root-awaiting-grant",
            render: |v| {
                let s = running(steps::VERIFY, 98, 18);
                let s = Sheet {
                    grant_since: s.started.map(|t| t + Duration::from_secs(80)),
                    ..s
                };
                sheet(v, PLAY_STORE, s, 98)
            },
        },
        Case {
            name: "root-unverified",
            render: |v| {
                let e = JidrError::new(
                    ErrorCode::RootFailed,
                    "root grant to Shell was denied or timed out; tap Grant and try again, or revert",
                )
                .with_data(json!({ "state": "awaiting_user_grant" }));
                let s = Sheet {
                    outcome: Some(Outcome::Failed {
                        failure: steps::failure(&e, steps::VERIFY),
                        secs: 150,
                        detail: String::new(),
                    }),
                    ..running(steps::VERIFY, 150, 60)
                };
                sheet(v, PLAY_STORE, s, 150)
            },
        },
        Case {
            name: "root-devtools",
            render: |v| devtools(v, PIXEL, false, true),
        },
        Case {
            name: "root-devtools-failed",
            render: |v| devtools(v, PIXEL, true, true),
        },
        Case {
            // A daemon without `root.auto_grant` (gap G15): the toggle is
            // disabled and says why.
            name: "root-devtools-no-auto-grant",
            render: |v| devtools(v, PIXEL, false, false),
        },
        Case {
            // Phase 9f: build props saved after this boot, so a restart
            // applies them.
            name: "root-devtools-props",
            render: |v| props_shot(v, false),
        },
        Case {
            // The open form, with the daemon refusing the patch date.
            name: "root-devtools-props-edit",
            render: |v| props_shot(v, true),
        },
    ]
}

/// The dev-tools section with the build-identity block (Phase 9f).
fn props_shot(
    v: Variant,
    editing: bool,
) -> Result<slint::SharedPixelBuffer<slint::Rgba8Pixel>, String> {
    use super::props::{Form, apply, field_error, props_vm};
    let app = device_tab(v, PIXEL)?;
    app.set_player_panel_tab(4);
    let mut info = fixtures::device_infos().remove(PIXEL);
    info.device.launch.force_cold_boot = true;
    let fill = |ui: &RootUi| {
        let vm = tools_vm(Some(&info), None, fixtures::now());
        ui.set_tools_visible(vm.visible);
        ui.set_tools_kind(vm.kind);
        ui.set_tools_badge(vm.badge);
        ui.set_tools_beta(vm.beta);
        ui.set_tools_full(vm.full);
        ui.set_tools_running(vm.running);
        ui.set_tools_busy(vm.busy);
        ui.set_tools_verified(vm.verified);
        ui.set_tools_failed(vm.failed);
        ui.set_tools_auto_grant(vm.auto_grant);
        ui.set_tools_auto_grant_supported(true);
        ui.set_props_supported(true);
    };
    let ui = app.global::<RootUi>();
    fill(&ui);
    let stored = [
        "Jidr One".to_string(),
        "jidr".into(),
        "Jidr".into(),
        String::new(),
        String::new(),
        "2026-10-05".into(),
    ];
    let vm = if editing {
        let mut draft = stored.clone();
        draft[3] = "jidr_one".into();
        draft[4] = "jidr/jidr_one/jidr_one:12/SP2A.220505/1:user/release-keys".into();
        draft[5] = "2026-13-05".into();
        let err = field_error(
            &JidrError::usage("security_patch: a date like 2026-10-05")
                .with_data(json!({ "field": "security_patch" })),
        );
        info.device.launch.force_cold_boot = false;
        props_vm(
            &info,
            &stored,
            Some(Form {
                draft: &draft,
                saving: false,
                error: Some(&err),
            }),
        )
    } else {
        props_vm(&info, &stored, None)
    };
    apply(&ui, &vm);
    // The open form runs below the fold at 900 px (the panel scrolls), so
    // the edit frame is taller to show all of it.
    shot(&app, 1440, if editing { 1240 } else { 900 })
}

/// The widget panel's "More" tab with the root section, for a rooted device
/// or one whose setup failed.
fn devtools(
    v: Variant,
    device: usize,
    failed: bool,
    auto_grant: bool,
) -> Result<slint::SharedPixelBuffer<slint::Rgba8Pixel>, String> {
    let app = device_tab(v, device)?;
    app.set_player_panel_tab(4);
    let mut info = fixtures::device_infos().remove(device);
    let failure = failed.then(|| {
        info.device.root.health = RootHealth::Failed;
        steps::failure(&JidrError::new(ErrorCode::BootTimeout, "x"), steps::REBOOT)
    });
    if failed {
        let mut infos = fixtures::device_infos();
        infos[device] = info.clone();
        let clock = fixtures::clock();
        app.set_devices(slint::ModelRc::new(slint::VecModel::from(
            infos
                .iter()
                .map(|i| crate::model::device_vm(i, &clock))
                .collect::<Vec<_>>(),
        )));
    }
    let vm = tools_vm(Some(&info), failure.as_ref(), fixtures::now());
    let ui = app.global::<RootUi>();
    ui.set_tools_visible(vm.visible);
    ui.set_tools_kind(vm.kind);
    ui.set_tools_badge(vm.badge);
    ui.set_tools_beta(vm.beta);
    ui.set_tools_full(vm.full);
    ui.set_tools_running(vm.running);
    ui.set_tools_busy(vm.busy);
    ui.set_tools_verified(vm.verified);
    ui.set_tools_failed(vm.failed);
    ui.set_tools_auto_grant(vm.auto_grant);
    ui.set_tools_auto_grant_supported(auto_grant);
    shot(&app, 1440, 900)
}
