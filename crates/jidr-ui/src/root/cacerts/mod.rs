//! System CA certificates: the dev-tools block for a Full-root device's
//! `root.cacert.*` commands (list, add from a file, remove). The Slint side
//! is `cacerts.slint`'s `CertsBlock`, bound to `RootUi` in `root.slint`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use jidr_core::{DeviceStatus, JidrResult, Job, RootMode};
use serde_json::{Value, json};
use slint::{ComponentHandle, ModelRc, VecModel};
use uuid::Uuid;

use crate::app::{Ctx, toast};
use crate::i18n::tr;
use crate::model::DeviceInfo;
use crate::{CertRow, RootUi};

/// Reads `root.cacert.list`'s `certs`.
pub fn rows_of(v: &Value) -> Vec<CertRow> {
    v.get("certs")
        .and_then(Value::as_array)
        .map(|certs| {
            certs
                .iter()
                .map(|c| CertRow {
                    file: c["file"].as_str().unwrap_or_default().into(),
                    subject: c["subject"].as_str().unwrap_or_default().into(),
                    trusted: c["trusted"].as_bool().unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Default)]
struct State {
    /// The last list per device.
    listed: HashMap<Uuid, Vec<CertRow>>,
    /// `root.cacert.list` sent, waiting for the answer.
    asked: Option<Uuid>,
    /// The device an add/remove runs on, and what it is doing.
    working: Option<Uuid>,
    status: String,
    failed: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

/// What the block shows for one device.
#[derive(Debug, Clone, PartialEq)]
pub struct CertsVm {
    pub certs: Vec<CertRow>,
    pub running: bool,
    pub working: bool,
    pub status: String,
    pub failed: bool,
}

fn vm(info: &DeviceInfo, s: &State) -> CertsVm {
    let id = info.device.id;
    let mine = s.working == Some(id);
    CertsVm {
        certs: s.listed.get(&id).cloned().unwrap_or_default(),
        running: matches!(info.device.status, DeviceStatus::Running),
        working: mine,
        status: if mine || s.failed {
            s.status.clone()
        } else {
            String::new()
        },
        failed: s.failed && !mine,
    }
}

pub fn apply(ui: &RootUi, vm: &CertsVm) {
    ui.set_certs(ModelRc::new(VecModel::from(vm.certs.clone())));
    ui.set_certs_running(vm.running);
    ui.set_certs_working(vm.working);
    ui.set_certs_status(vm.status.clone().into());
    ui.set_certs_failed(vm.failed);
}

pub fn wire(ctx: &Rc<Ctx>) {
    let ui = ctx.app.global::<RootUi>();
    let c = ctx.clone();
    ui.on_certs_action(move |a, v| action(&c, a.as_str(), v.as_str()));
}

/// Re-renders the block for the active device, listing its certificates the
/// first time it shows running. Called from `root::sync`.
pub fn sync(ctx: &Rc<Ctx>, info: Option<&DeviceInfo>) {
    let ui = ctx.app.global::<RootUi>();
    let Some(info) = info.filter(|i| i.device.root.mode == RootMode::Magisk) else {
        return;
    };
    let id = info.device.id;
    let running = matches!(info.device.status, DeviceStatus::Running);
    let vm = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if running
            && !s.listed.contains_key(&id)
            && s.asked != Some(id)
            && s.working.is_none()
            && ui.get_certs_supported()
        {
            s.asked = Some(id);
            fetch(ctx, id);
        }
        vm(info, &s)
    });
    apply(&ui, &vm);
}

fn fetch(ctx: &Ctx, id: Uuid) {
    ctx.bridge
        .call("root.cacert.list", json!({ "device": id }), move |r| {
            let rows = r.ok().map(|v| rows_of(&v));
            let _ = slint::invoke_from_event_loop(move || {
                STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    s.asked = None;
                    if let Some(rows) = rows {
                        s.listed.insert(id, rows);
                    }
                });
                rerender();
            });
        });
}

fn rerender() {
    if let Some(c) = crate::app::ctx() {
        crate::root::sync(&c);
    }
}

fn action(ctx: &Rc<Ctx>, action: &str, value: &str) {
    let Some(id) = crate::player::active_id(ctx) else {
        return;
    };
    match action {
        "add" => {
            let token = crate::app::defer(move |path: Option<PathBuf>| {
                if let (Some(p), Some(c)) = (path, crate::app::ctx()) {
                    run(
                        &c,
                        id,
                        "root.cacert.install",
                        json!({ "device": id, "path": p }),
                    );
                }
            });
            std::thread::spawn(move || crate::app::fire(token, choose_file()));
        }
        "remove" => run(
            ctx,
            id,
            "root.cacert.remove",
            json!({ "device": id, "cert": value }),
        ),
        _ => {}
    }
}

/// Runs an add or remove (a job that restarts the device) and lists again.
fn run(ctx: &Rc<Ctx>, id: Uuid, method: &'static str, params: Value) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.working = Some(id);
        s.failed = false;
        s.status = tr!("Starting…");
    });
    rerender();
    let w = ctx.app.as_weak();
    ctx.bridge.run(
        method,
        params,
        |job: &Job| {
            let msg = job.message.clone();
            let _ = slint::invoke_from_event_loop(move || {
                STATE.with(|s| s.borrow_mut().status = format!("{msg}…"));
                rerender();
            });
        },
        move |r: JidrResult<Value>| {
            let note = match &r {
                Ok(v) if method == "root.cacert.install" && v["changed"] == false => {
                    tr!("That certificate is already trusted")
                }
                Ok(_) if method == "root.cacert.install" => {
                    match r.as_ref().ok().and_then(|v| v["cert"]["subject"].as_str()) {
                        Some(subject) => tr!("{} is now a trusted system certificate", subject),
                        None => tr!("The certificate is now a trusted system certificate"),
                    }
                }
                Ok(_) => tr!("Certificate removed"),
                Err(e) => e.message.clone(),
            };
            toast(&w, note.clone());
            let _ = slint::invoke_from_event_loop(move || {
                STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    s.working = None;
                    s.listed.remove(&id);
                    s.failed = r.is_err();
                    s.status = if r.is_err() { note } else { String::new() };
                });
                rerender();
            });
        },
    );
}

/// The macOS file picker for a certificate.
fn choose_file() -> Option<PathBuf> {
    let prompt = crate::i18n::applescript_string(&tr!(
        "Choose a CA certificate (PEM or DER), e.g. Burp's cacert.der"
    ));
    let out = std::process::Command::new("osascript")
        .args([
            "-e",
            &format!("POSIX path of (choose file with prompt {prompt})"),
        ])
        .output()
        .ok()?;
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!p.is_empty()).then(|| PathBuf::from(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    #[test]
    fn list_answers_become_rows() {
        let rows = rows_of(&json!({ "certs": [
            { "file": "a4ac454c.0", "subject": "Jidr Test CA", "trusted": true, "sha256": "x" },
            { "file": "9a5ba575.0", "subject": "PortSwigger CA", "trusted": false, "sha256": "y" },
        ]}));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].file, "a4ac454c.0");
        assert!(rows[0].trusted && !rows[1].trusted);
        assert!(rows_of(&json!({})).is_empty());
    }

    #[test]
    fn status_shows_on_the_working_device_or_after_a_failure() {
        let info = fixtures::device_infos().remove(0);
        let id = info.device.id;
        let mut s = State {
            working: Some(id),
            status: "Rebooting to apply…".into(),
            ..State::default()
        };
        let v = vm(&info, &s);
        assert!(v.working && !v.failed);
        assert_eq!(v.status, "Rebooting to apply…");
        s.working = Some(Uuid::new_v4());
        assert_eq!(vm(&info, &s).status, "", "another device's job");
        s.working = None;
        s.failed = true;
        s.status = "not in the store".into();
        let v = vm(&info, &s);
        assert!(v.failed && !v.working);
        assert_eq!(v.status, "not in the store");
    }
}
