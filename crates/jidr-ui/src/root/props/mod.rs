//! Build identity (Phase 9f): the dev-tools form for a Full-root device's
//! build props (`device.props.get` / `.set`). The Slint side is
//! `props.slint`'s `PropsBlock`, bound to `RootUi` in `root.slint`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use jidr_core::{DeviceStatus, JidrError, JidrResult, RootMode};
use serde_json::{Map, Value, json};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use uuid::Uuid;

use crate::RootUi;
use crate::app::{Ctx, toast};
use crate::model::DeviceInfo;

/// The props' wire names, in form order.
pub const FIELDS: [&str; 6] = [
    "model",
    "brand",
    "manufacturer",
    "device",
    "fingerprint",
    "security_patch",
];

/// The six values in form order; empty = stock.
pub type Values = [String; 6];

/// Reads `device.props.*`'s `props` object.
pub fn values_of(props: &Value) -> Values {
    FIELDS.map(|f| {
        props
            .get(f)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    })
}

/// The `props` object for `device.props.set`: empty fields are left out.
pub fn props_of(values: &Values) -> Value {
    let mut m = Map::new();
    for (f, v) in FIELDS.iter().zip(values) {
        if !v.trim().is_empty() {
            m.insert((*f).to_string(), json!(v.trim()));
        }
    }
    Value::Object(m)
}

#[derive(Debug, Default)]
struct State {
    /// What the daemon has stored, per device.
    stored: HashMap<Uuid, Values>,
    /// The device whose form is open, and its draft.
    editing: Option<Uuid>,
    draft: Values,
    saving: bool,
    error: Option<(i32, String)>,
    /// `device.props.get` sent, waiting for the answer.
    asked: Option<Uuid>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

/// What the block shows for one device.
#[derive(Debug, Clone, PartialEq)]
pub struct PropsVm {
    pub summary: String,
    pub custom: bool,
    pub editing: bool,
    pub values: Values,
    pub error_field: i32,
    pub error: String,
    pub pending: bool,
    pub saving: bool,
}

/// "Stock values", or the identity at a glance: model · brand (· device) ·
/// patch date. A fingerprint alone reads "Custom fingerprint".
pub fn summary(values: &Values) -> String {
    let [model, brand, manufacturer, device, fingerprint, patch] = values;
    let mut parts: Vec<String> = Vec::new();
    for v in [model, brand, manufacturer, device] {
        let v = v.trim();
        if !v.is_empty() && !parts.iter().any(|p| p.eq_ignore_ascii_case(v)) {
            parts.push(v.to_string());
        }
    }
    if !patch.trim().is_empty() {
        parts.push(format!("patch {}", patch.trim()));
    }
    if parts.is_empty() {
        if fingerprint.trim().is_empty() {
            "Stock values".into()
        } else {
            "Custom fingerprint".into()
        }
    } else {
        parts.join(" · ")
    }
}

/// The open form: its draft, whether it is saving, and the refused field.
#[derive(Debug, Clone, Copy)]
pub struct Form<'a> {
    pub draft: &'a Values,
    pub saving: bool,
    pub error: Option<&'a (i32, String)>,
}

/// The view model from what is stored and the open form, if any.
pub fn props_vm(info: &DeviceInfo, stored: &Values, form: Option<Form<'_>>) -> PropsVm {
    let custom = stored.iter().any(|v| !v.is_empty());
    let (editing, values, saving, error) = match form {
        Some(f) => (true, f.draft.clone(), f.saving, f.error.cloned()),
        None => (false, stored.clone(), false, None),
    };
    let (error_field, error) = error.unwrap_or((-1, String::new()));
    PropsVm {
        summary: summary(stored),
        custom,
        editing,
        values,
        error_field,
        error,
        // A start clears `force_cold_boot`: still set while running means
        // the props were saved after this boot.
        pending: matches!(info.device.status, DeviceStatus::Running)
            && info.device.launch.force_cold_boot,
        saving,
    }
}

/// Where a `usage` error from `device.props.set` points: the field index
/// (from `data.field`) and the message without its "field: " prefix.
pub fn field_error(e: &JidrError) -> (i32, String) {
    let field = e
        .data
        .as_ref()
        .and_then(|d| d.get("field"))
        .and_then(Value::as_str);
    match field.and_then(|f| FIELDS.iter().position(|x| *x == f)) {
        Some(i) => {
            let prefix = format!("{}: ", FIELDS[i]);
            let msg = e.message.strip_prefix(&prefix).unwrap_or(&e.message);
            (i as i32, capitalize(msg))
        }
        None => (-1, e.message.clone()),
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Fills `ui`'s props properties.
pub fn apply(ui: &RootUi, vm: &PropsVm) {
    ui.set_props_summary(vm.summary.clone().into());
    ui.set_props_custom(vm.custom);
    ui.set_props_editing(vm.editing);
    ui.set_props_values(ModelRc::new(VecModel::from(
        vm.values
            .iter()
            .map(|v| SharedString::from(v.as_str()))
            .collect::<Vec<_>>(),
    )));
    ui.set_props_error_field(vm.error_field);
    ui.set_props_error(vm.error.clone().into());
    ui.set_props_pending(vm.pending);
    ui.set_props_saving(vm.saving);
}

pub fn wire(ctx: &Rc<Ctx>) {
    let ui = ctx.app.global::<RootUi>();
    let c = ctx.clone();
    ui.on_props_action(move |a| action(&c, a.as_str()));
    ui.on_props_edited(|i, v| {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            if let Some(slot) = usize::try_from(i).ok().and_then(|i| s.draft.get_mut(i)) {
                *slot = v.to_string();
            }
            if s.error.as_ref().is_some_and(|(f, _)| *f == i) {
                s.error = None;
            }
        });
    });
}

/// Re-renders the block for the active device, asking the daemon for its
/// props the first time it shows. Called from `root::sync`.
pub fn sync(ctx: &Rc<Ctx>, info: Option<&DeviceInfo>) {
    let ui = ctx.app.global::<RootUi>();
    let Some(info) = info.filter(|i| i.device.root.mode == RootMode::Magisk) else {
        STATE.with(|s| s.borrow_mut().editing = None);
        ui.set_props_editing(false);
        return;
    };
    let id = info.device.id;
    let vm = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.editing.is_some_and(|e| e != id) {
            s.editing = None;
            s.error = None;
        }
        let stored = s.stored.get(&id).cloned();
        if stored.is_none() && s.asked != Some(id) && ui.get_props_supported() {
            s.asked = Some(id);
            fetch(ctx, id);
        }
        let stored = stored.unwrap_or_default();
        let form = (s.editing == Some(id)).then_some(Form {
            draft: &s.draft,
            saving: s.saving,
            error: s.error.as_ref(),
        });
        props_vm(info, &stored, form)
    });
    apply(&ui, &vm);
}

fn fetch(ctx: &Ctx, id: Uuid) {
    ctx.bridge
        .call("device.props.get", json!({ "device": id }), move |r| {
            let values = r.ok().map(|v| values_of(&v["props"]));
            let _ = slint::invoke_from_event_loop(move || {
                STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    s.asked = None;
                    s.stored.insert(id, values.unwrap_or_default());
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

fn action(ctx: &Rc<Ctx>, action: &str) {
    let Some(id) = crate::player::active_id(ctx) else {
        return;
    };
    match action {
        "edit" => STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.draft = s.stored.get(&id).cloned().unwrap_or_default();
            s.editing = Some(id);
            s.error = None;
        }),
        "cancel" => STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.editing = None;
            s.error = None;
        }),
        "save" => {
            let draft = STATE.with(|s| s.borrow().draft.clone());
            save(ctx, id, props_of(&draft));
        }
        "reset" => save(ctx, id, json!({})),
        "restart" => restart(ctx, id),
        _ => {}
    }
    rerender();
}

fn save(ctx: &Rc<Ctx>, id: Uuid, props: Value) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.saving = true;
        s.error = None;
    });
    let w = ctx.app.as_weak();
    let running = ctx
        .info(id)
        .is_some_and(|i| matches!(i.device.status, DeviceStatus::Running));
    ctx.bridge.call(
        "device.props.set",
        json!({ "device": id, "props": props }),
        move |r: JidrResult<Value>| {
            let msg = match &r {
                Ok(_) if running => Some("Build identity saved. Restart to apply it.".to_string()),
                Ok(_) => Some("Build identity saved. It applies on the next start.".to_string()),
                Err(_) => None,
            };
            if let Some(m) = msg {
                toast(&w, m);
            }
            let _ = slint::invoke_from_event_loop(move || {
                STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    s.saving = false;
                    match r {
                        Ok(v) => {
                            s.stored.insert(id, values_of(&v["props"]));
                            s.editing = None;
                        }
                        Err(e) => {
                            // Reset to stock fails outside the form too.
                            if s.editing != Some(id) {
                                s.draft = s.stored.get(&id).cloned().unwrap_or_default();
                                s.editing = Some(id);
                            }
                            s.error = Some(field_error(&e));
                        }
                    }
                });
                rerender();
            });
        },
    );
}

/// "Restart now": stop, then a start — the cold boot the props need.
fn restart(ctx: &Rc<Ctx>, id: Uuid) {
    let w = ctx.app.as_weak();
    ctx.bridge
        .call("device.stop", json!({ "device": id }), move |r| match r {
            Ok(_) => {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(c) = crate::app::ctx() {
                        crate::launcher::start(&c, id);
                    }
                });
            }
            Err(e) => toast(&w, e.message),
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    fn pixel() -> DeviceInfo {
        fixtures::device_infos().remove(0)
    }

    fn values(model: &str, brand: &str, patch: &str) -> Values {
        [
            model.into(),
            brand.into(),
            String::new(),
            String::new(),
            String::new(),
            patch.into(),
        ]
    }

    #[test]
    fn summaries() {
        assert_eq!(summary(&Values::default()), "Stock values");
        assert_eq!(
            summary(&values("Jidr One", "jidr", "2026-10-05")),
            "Jidr One · jidr · patch 2026-10-05"
        );
        let mut v = values("Pixel 8", "google", "");
        v[2] = "Google".into();
        assert_eq!(summary(&v), "Pixel 8 · google", "manufacturer = brand");
        let mut v = Values::default();
        v[4] = "google/husky/husky:15".into();
        assert_eq!(summary(&v), "Custom fingerprint");
    }

    #[test]
    fn props_round_trip_and_empty_fields_are_left_out() {
        let v = values(" Jidr One ", "", "2026-10-05");
        let p = props_of(&v);
        assert_eq!(
            p,
            json!({ "model": "Jidr One", "security_patch": "2026-10-05" })
        );
        assert_eq!(values_of(&p), values("Jidr One", "", "2026-10-05"));
    }

    #[test]
    fn pending_means_saved_after_this_boot() {
        let mut info = pixel();
        let stored = values("Jidr One", "", "");
        assert!(!props_vm(&info, &stored, None).pending);
        info.device.launch.force_cold_boot = true;
        let vm = props_vm(&info, &stored, None);
        assert!(vm.pending);
        assert!(vm.custom);
        info.device.status = DeviceStatus::Stopped;
        assert!(
            !props_vm(&info, &stored, None).pending,
            "stopped: no restart"
        );
    }

    #[test]
    fn a_refused_field_shows_under_that_field() {
        let e = JidrError::usage("security_patch: a date like 2026-10-05")
            .with_data(json!({ "field": "security_patch" }));
        assert_eq!(field_error(&e), (5, "A date like 2026-10-05".into()));
        let e = JidrError::usage("phone isn't Full-root (Magisk)");
        assert_eq!(field_error(&e).0, -1);
        let draft = values("Jidr One", "", "Oct");
        let err = field_error(
            &JidrError::usage("security_patch: x").with_data(json!({ "field": "security_patch" })),
        );
        let vm = props_vm(
            &pixel(),
            &Values::default(),
            Some(Form {
                draft: &draft,
                saving: false,
                error: Some(&err),
            }),
        );
        assert!(vm.editing);
        assert_eq!(vm.error_field, 5);
        assert_eq!(vm.values[5], "Oct", "the draft stays");
    }
}
