//! First-run setup: find an SDK, show Google's license unchanged (accept
//! only after reading to the end, D8), download, create the first device.

use std::rc::Rc;

use jidr_core::{HardwareProfile, Support, SystemImage};
use serde::Deserialize;
use serde_json::{Value, json};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::app::Ctx;
use crate::model;
use crate::{DetectRow, JobTone, JobVm, PlanRow, RootChoice, SetupWindow, Theme};

/// One license still to accept (`sdk.licenses`).
#[derive(Debug, Clone, Deserialize)]
pub struct PendingLicense {
    pub id: String,
    pub sha256: String,
    pub text: String,
    #[serde(default)]
    pub packages: Vec<String>,
}

/// Where Jidr looked for an SDK, from `sdk.status` and the environment.
pub fn detect_rows(
    status: &Value,
    android_home: Option<&str>,
    sdk_root: Option<&str>,
) -> (Vec<DetectRow>, bool) {
    let studio = status.get("studio_sdk").and_then(Value::as_str);
    let tools = status.get("tools_sdk").and_then(Value::as_str);
    let row = |path: &str, state: &str, found: bool| DetectRow {
        path: path.into(),
        state: state.into(),
        found,
    };
    let rows = vec![
        match studio {
            Some(p) => row(&tilde(p), "Found · used read-only", true),
            None => row("~/Library/Android/sdk", "Not found", false),
        },
        match android_home {
            Some(p) if !p.is_empty() => row("$ANDROID_HOME", &tilde(p), true),
            _ => row("$ANDROID_HOME", "Not set", false),
        },
        match sdk_root {
            Some(p) if !p.is_empty() => row("$ANDROID_SDK_ROOT", &tilde(p), true),
            _ => row("$ANDROID_SDK_ROOT", "Not set", false),
        },
    ];
    (rows, tools.is_some())
}

fn tilde(p: &str) -> String {
    match dirs::home_dir().and_then(|h| h.to_str().map(str::to_string)) {
        Some(h) if p.starts_with(&h) => format!("~{}", &p[h.len()..]),
        _ => p.to_string(),
    }
}

/// The first image to offer: API 35 Google APIs, else the newest supported
/// Google APIs image, else the newest supported one.
pub fn pick_image(images: &[SystemImage]) -> Option<SystemImage> {
    let supported: Vec<&SystemImage> = images
        .iter()
        .filter(|i| i.support == Support::Supported)
        .collect();
    supported
        .iter()
        .find(|i| i.image.api == 35 && i.image.tag == "google_apis")
        .or_else(|| {
            supported
                .iter()
                .filter(|i| i.image.tag == "google_apis")
                .max_by_key(|i| i.image.api)
        })
        .or_else(|| supported.iter().max_by_key(|i| i.image.api))
        .map(|i| (*i).clone())
}

/// What will be downloaded, with real catalog sizes where known.
pub fn plan(needs_emulator: bool, image: Option<&SystemImage>) -> (Vec<PlanRow>, String) {
    let mut rows = Vec::new();
    let mut total = 0u64;
    let mut unknown = false;
    if needs_emulator {
        rows.push(PlanRow {
            label: "Android Emulator and platform tools".into(),
            size: "size shown when it starts".into(),
        });
        unknown = true;
    }
    if let Some(img) = image.filter(|i| i.installed_in.is_none()) {
        rows.push(PlanRow {
            label: format!(
                "{} · {} · arm64",
                model::android_version(img.image.api) + &format!(" (API {})", img.image.api),
                model::image_tag(&img.image)
            )
            .into(),
            size: if img.size > 0 {
                model::gb(img.size)
            } else {
                "—".into()
            }
            .into(),
        });
        total += img.size;
    }
    let total = match (total, unknown) {
        (0, true) => "emulator only".to_string(),
        (t, true) => format!("{} + emulator", model::gb(t)),
        (t, false) => model::gb(t),
    };
    (rows, total)
}

fn models<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(v))
}

/// Opens the setup window and drives it.
pub fn open(ctx: &Rc<Ctx>) {
    let Ok(win) = SetupWindow::new() else { return };
    let theme = ctx.app.global::<Theme>();
    let wt = win.global::<Theme>();
    wt.set_dark(theme.get_dark());
    wt.set_rtl(theme.get_rtl());
    wt.set_reduce_motion(theme.get_reduce_motion());
    win.set_draw_lights(false);

    let weak = win.as_weak();
    let state: St = St::default();

    // Step 1: detection and plan.
    {
        let weak = weak.clone();
        let state = state.clone();
        let bridge = ctx.bridge.clone();
        let b2 = bridge.clone();
        bridge.call("sdk.status", json!({}), move |status| {
            let status = status.unwrap_or(Value::Null);
            b2.call("image.list", json!({}), move |images| {
                let images: Vec<SystemImage> = images
                    .ok()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(win) = weak.upgrade() else { return };
                    let home = std::env::var("ANDROID_HOME").ok();
                    let root = std::env::var("ANDROID_SDK_ROOT").ok();
                    let (rows, found) = detect_rows(&status, home.as_deref(), root.as_deref());
                    let image = pick_image(&images);
                    let (plan_rows, total) = plan(!found, image.as_ref());
                    win.set_detect(models(rows));
                    win.set_sdk_found(found);
                    win.set_plan(models(plan_rows));
                    win.set_plan_total(total.into());
                    win.set_plan_dir(
                        status
                            .get("private_sdk")
                            .and_then(Value::as_str)
                            .map(tilde)
                            .unwrap_or_default()
                            .into(),
                    );
                    let mut st = lock(&state);
                    st.image = image;
                    st.needs_install =
                        !found || st.image.as_ref().is_some_and(|i| i.installed_in.is_none());
                });
            });
        });
    }

    let c = ctx.clone();
    let st = state.clone();
    let w = weak.clone();
    win.on_next(move || {
        let Some(win) = w.upgrade() else { return };
        match win.get_step() {
            1 if lock(&st).needs_install => load_licenses(&c, &win, &st),
            1 | 3 => to_first_device(&c, &win, &st),
            _ => {}
        }
    });

    let w = weak.clone();
    win.on_back(move || {
        if let Some(win) = w.upgrade() {
            win.set_step(1);
        }
    });
    let w = weak.clone();
    win.on_decline(move || {
        if let Some(win) = w.upgrade() {
            let _ = win.hide();
        }
    });
    let w = weak.clone();
    win.on_quit(move || {
        if let Some(win) = w.upgrade() {
            let _ = win.hide();
        }
    });
    let w = weak.clone();
    win.on_run_in_background(move || {
        if let Some(win) = w.upgrade() {
            let _ = win.hide();
        }
    });

    let c = ctx.clone();
    let st = state.clone();
    let w = weak.clone();
    win.on_accept(move || {
        let Some(win) = w.upgrade() else { return };
        accept_and_download(&c, &win, &st);
    });

    let c = ctx.clone();
    let st = state.clone();
    win.on_cancel_download(move || {
        if let Some(job) = lock(&st).job {
            c.bridge.call("jobs.cancel", json!({ "job": job }), |_| {});
        }
    });

    let c = ctx.clone();
    let st = state.clone();
    let w = weak.clone();
    win.on_create(move |form| {
        let Some(win) = w.upgrade() else { return };
        let st = lock(&st);
        let images: Vec<SystemImage> = st.image.iter().cloned().collect();
        let mut form = form;
        form.image = 0;
        match crate::device_form::create_params(&form, &st.profiles, &images) {
            Err(e) => win.set_error(e.into()),
            Ok(params) => {
                win.set_error("".into());
                let start = form.start_after;
                let w2 = w.clone();
                let bridge = c.bridge.clone();
                let app = c.app.as_weak();
                c.bridge.call("device.create", params, move |r| match r {
                    Ok(dev) => {
                        if start && let Some(id) = dev.get("id").cloned() {
                            bridge.run("device.start", json!({ "device": id }), |_| {}, |_| {});
                        }
                        crate::app::toast(&app, "Created your first device".into());
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(win) = w2.upgrade() {
                                let _ = win.hide();
                            }
                        });
                    }
                    Err(e) => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(win) = w2.upgrade() {
                                win.set_error(e.message.into());
                            }
                        });
                    }
                });
            }
        }
    });

    let _ = win.show();
    // Keep it alive while shown.
    ctx.windows.borrow_mut().push(Box::new(win));
}

type St = std::sync::Arc<std::sync::Mutex<SetupState>>;

fn lock(st: &St) -> std::sync::MutexGuard<'_, SetupState> {
    st.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Default)]
struct SetupState {
    image: Option<SystemImage>,
    needs_install: bool,
    licenses: Vec<PendingLicense>,
    job: Option<uuid::Uuid>,
    profiles: Vec<HardwareProfile>,
}

fn packages(st: &SetupState) -> Vec<String> {
    let mut p = Vec::new();
    if let Some(i) = st.image.as_ref().filter(|i| i.installed_in.is_none()) {
        p.push(i.image.package());
    }
    if p.is_empty() {
        p.push("emulator".into());
    }
    p
}

fn load_licenses(ctx: &Rc<Ctx>, win: &SetupWindow, st: &St) {
    let pkgs = packages(&lock(st));
    let weak = win.as_weak();
    let st = st.clone();
    ctx.bridge
        .call("sdk.licenses", json!({ "packages": pkgs }), move |r| {
            let _ = slint::invoke_from_event_loop(move || {
                let Some(win) = weak.upgrade() else { return };
                match r.and_then(|v| {
                    serde_json::from_value::<Vec<PendingLicense>>(v)
                        .map_err(|e| jidr_core::JidrError::unexpected(e.to_string()))
                }) {
                    Ok(lics) if lics.is_empty() => {
                        lock(&st).licenses.clear();
                        win.set_license_read(true);
                        win.set_step(2);
                    }
                    Ok(lics) => {
                        win.set_license_title(license_title(&lics[0].id).into());
                        win.set_license_text(
                            lics.iter()
                                .map(|l| l.text.as_str())
                                .collect::<Vec<_>>()
                                .join("\n\n")
                                .into(),
                        );
                        win.set_license_read(false);
                        lock(&st).licenses = lics;
                        win.set_step(2);
                    }
                    Err(e) => win.set_error(e.message.into()),
                }
            });
        });
}

pub fn license_title(id: &str) -> String {
    match id {
        "android-sdk-license" => "Android Software Development Kit License Agreement".into(),
        "android-sdk-preview-license" => "Android SDK Preview License Agreement".into(),
        other => other.replace('-', " "),
    }
}

fn accept_and_download(ctx: &Rc<Ctx>, win: &SetupWindow, st: &St) {
    let lics = lock(st).licenses.clone();
    let pkgs = packages(&lock(st));
    let image = lock(st).image.clone();
    let accepted = chrono::Local::now().format("%b %-d, %H:%M").to_string();
    win.set_accepted_note(
        format!(
            "License accepted {accepted}. You can close this window; downloads keep going in Jobs."
        )
        .into(),
    );
    win.set_step(3);
    let title = image
        .as_ref()
        .map(|i| {
            format!(
                "{} (API {}) image",
                model::android_version(i.image.api),
                i.image.api
            )
        })
        .unwrap_or_else(|| "Android Emulator".into());
    let size = image.as_ref().map(|i| i.size).unwrap_or(0);
    set_download(win, &title, JobTone::Running, 0.0, size, "");
    let bridge = ctx.bridge.clone();
    let weak = win.as_weak();
    let st2 = st.clone();
    // Consent first (per license hash), then the install job.
    let b = bridge.clone();
    ctx.bridge.handle().spawn(async move {
        for l in &lics {
            let (tx, rx) = tokio::sync::oneshot::channel();
            b.call(
                "sdk.accept_license",
                json!({ "id": l.id, "sha256": l.sha256 }),
                move |r| {
                    let _ = tx.send(r);
                },
            );
            if let Ok(Err(e)) = rx.await {
                let msg = e.message;
                let weak = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = weak.upgrade() {
                        win.set_error(msg.into());
                    }
                });
                return;
            }
        }
        let method = if image.as_ref().is_some_and(|i| i.installed_in.is_none()) {
            "image.install"
        } else {
            "sdk.install"
        };
        let params = match (&image, method) {
            (Some(i), "image.install") => json!({ "image": i.image.package() }),
            _ => json!({ "packages": ["emulator", "platform-tools"] }),
        };
        let _ = pkgs;
        let (wp, wd) = (weak.clone(), weak.clone());
        let (tp, td) = (title.clone(), title.clone());
        let started = std::time::Instant::now();
        b.run(
            method,
            params,
            move |job| {
                let p = job.progress.unwrap_or(0.0);
                let eta = eta_text(started.elapsed().as_secs_f32(), p);
                let tp = tp.clone();
                let wp = wp.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = wp.upgrade() {
                        set_download(&win, &tp, JobTone::Running, p, size, &eta);
                    }
                });
            },
            move |r| {
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(win) = wd.upgrade() else { return };
                    match r {
                        Ok(_) => {
                            set_download(&win, &td, JobTone::Done, 1.0, size, "Done");
                            win.set_downloads_done(true);
                            if let Some(i) = lock(&st2).image.as_mut() {
                                i.installed_in = Some("installed".into());
                            }
                        }
                        Err(e) => {
                            set_download(&win, &td, JobTone::Failed, 0.0, size, "");
                            win.set_error(e.message.into());
                        }
                    }
                });
            },
        );
    });
}

fn eta_text(elapsed: f32, progress: f32) -> String {
    if progress <= 0.02 || elapsed < 2.0 {
        return String::new();
    }
    let left = elapsed * (1.0 - progress) / progress;
    if left >= 60.0 {
        format!("About {} min left", (left / 60.0).ceil() as i64)
    } else {
        format!("About {} s left", left.ceil() as i64)
    }
}

fn set_download(win: &SetupWindow, title: &str, tone: JobTone, p: f32, size: u64, eta: &str) {
    let detail = if size > 0 {
        format!(
            "{:.1} / {}",
            f64::from(p) * size as f64 / 1e9,
            model::gb(size)
        )
    } else {
        format!("{}%", (p * 100.0).round() as i64)
    };
    let row = JobVm {
        id: "".into(),
        title: title.into(),
        tone,
        progress: p,
        indeterminate: false,
        detail: detail.into(),
        eta: eta.into(),
        cancellable: tone == JobTone::Running,
    };
    let verify = JobVm {
        id: "".into(),
        title: "Verify checksums".into(),
        tone: if tone == JobTone::Done {
            JobTone::Done
        } else {
            JobTone::Queued
        },
        progress: 0.0,
        indeterminate: false,
        detail: if tone == JobTone::Done {
            "Done"
        } else {
            "Queued"
        }
        .into(),
        eta: if tone == JobTone::Done {
            ""
        } else {
            "After download"
        }
        .into(),
        cancellable: false,
    };
    win.set_downloads(models(vec![row, verify]));
}

fn to_first_device(ctx: &Rc<Ctx>, win: &SetupWindow, st: &St) {
    let weak = win.as_weak();
    let st = st.clone();
    let taken: Vec<String> = ctx
        .state
        .borrow()
        .data
        .devices
        .iter()
        .map(|d| d.device.name.clone())
        .collect();
    let preferred = ctx.prefs.borrow().root_choice();
    ctx.bridge.call("profile.list", json!({}), move |r| {
        let profiles: Vec<HardwareProfile> = r
            .ok()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(win) = weak.upgrade() else { return };
            let mut s = lock(&st);
            let images: Vec<SystemImage> = s.image.iter().cloned().collect();
            let mut form = crate::device_form::new_form(&profiles, &images, &taken, preferred);
            if form.root == RootChoice::None && preferred == RootChoice::Shell {
                form.root = crate::device_form::default_root(images.first(), RootChoice::Shell);
            }
            win.set_profiles(models(crate::device_form::profile_options(&profiles)));
            if let Some(i) = images.first() {
                win.set_image_label(crate::device_form::image_label(i).into());
                win.set_image_status(format!("Installed · {}", model::gb(i.size)).into());
                win.set_image_play_store(!i.image.allows_adb_root());
            }
            win.set_form(form);
            s.profiles = profiles;
            win.set_step(4);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use jidr_core::ImageRef;

    fn img(api: u32, tag: &str, installed: bool) -> SystemImage {
        SystemImage {
            image: ImageRef {
                api,
                tag: tag.into(),
                abi: "arm64-v8a".into(),
            },
            display: String::new(),
            revision: "1".into(),
            support: Support::for_api(api),
            size: 2_000_000_000,
            license: Some("android-sdk-license".into()),
            installed_in: installed.then(|| "/sdk".into()),
            pristine_ramdisk_sha256: None,
        }
    }

    #[test]
    fn detection_reports_each_place() {
        let (rows, found) = detect_rows(
            &json!({"studio_sdk": null, "tools_sdk": null}),
            None,
            Some("/opt/sdk"),
        );
        assert!(!found);
        assert_eq!(rows[0].state, "Not found");
        assert_eq!(rows[1].state, "Not set");
        assert_eq!(rows[2].state, "/opt/sdk");
        let (_, found) = detect_rows(&json!({"studio_sdk": "/x", "tools_sdk": "/x"}), None, None);
        assert!(found);
    }

    #[test]
    fn picks_api_35_google_apis_first() {
        let images = [
            img(36, "google_apis", false),
            img(35, "google_apis_playstore", false),
            img(35, "google_apis", false),
        ];
        let p = pick_image(&images).unwrap();
        assert_eq!((p.image.api, p.image.tag.as_str()), (35, "google_apis"));
        let p =
            pick_image(&[img(34, "google_apis", false), img(36, "google_apis", false)]).unwrap();
        assert_eq!(p.image.api, 36);
    }

    #[test]
    fn plan_lists_only_what_is_missing() {
        let (rows, total) = plan(false, Some(&img(35, "google_apis", false)));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "Android 15 (API 35) · Google APIs · arm64");
        assert_eq!(total, "2.0 GB");
        let (rows, total) = plan(true, Some(&img(35, "google_apis", true)));
        assert_eq!(rows.len(), 1);
        assert_eq!(total, "emulator only");
    }

    #[test]
    fn eta_is_blank_until_progress_is_measurable() {
        assert_eq!(eta_text(1.0, 0.5), "");
        assert_eq!(eta_text(30.0, 0.5), "About 30 s left");
        assert_eq!(eta_text(60.0, 0.25), "About 3 min left");
    }
}
