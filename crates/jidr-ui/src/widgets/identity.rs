//! The Identifiers section (#19): the Android ID draft, a random value, and
//! `identity.set`, which runs as a job (it restarts Android). The IMEI is
//! shown read only.

use std::rc::Rc;

use jidr_helper::identity::check_android_id;
use serde_json::{Value, json};
use slint::ComponentHandle;
use uuid::Uuid;

use crate::WidgetsVm;
use crate::app::{Ctx, toast};

/// 16 hex digits from `seed`'s random bits.
pub fn random_android_id(seed: Uuid) -> String {
    seed.simple().to_string()[..16].to_string()
}

/// Merges an `identity.*` answer: the current values, and the draft when
/// it is untouched.
pub fn merge(vm: &mut WidgetsVm, v: &Value) {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default();
    vm.android_id_now = s("android_id").into();
    vm.imei_now = s("imei").into();
    if vm.android_id.is_empty() {
        vm.android_id = s("android_id").into();
    }
}

/// The `identity.set` params for the draft, or the error to show.
pub fn set_params(vm: &WidgetsVm) -> Result<Value, String> {
    check_android_id(vm.android_id.trim())
        .map(|id| json!({ "android_id": id }))
        .map_err(|e| e.message)
}

/// Identifiers actions; true when handled.
pub fn action(ctx: &Rc<Ctx>, id: Uuid, action: &str, value: &str) -> bool {
    let mut vm = ctx.app.get_widgets();
    match action {
        "identity.android-id" => vm.android_id = value.into(),
        "identity.random-android-id" => vm.android_id = random_android_id(Uuid::new_v4()).into(),
        "identity.set-android-id" => {
            match set_params(&vm) {
                Ok(params) => apply(ctx, id, params),
                Err(e) => toast(&ctx.app.as_weak(), e),
            }
            return true;
        }
        _ => return false,
    }
    ctx.app.set_widgets(vm);
    true
}

fn apply(ctx: &Rc<Ctx>, id: Uuid, mut params: Value) {
    params["device"] = json!(id);
    let mut vm = ctx.app.get_widgets();
    vm.identity_busy = true;
    ctx.app.set_widgets(vm);
    let token = crate::app::defer(move |r: jidr_core::JidrResult<Value>| {
        let Some(c) = crate::app::ctx() else { return };
        let mut vm = c.app.get_widgets();
        vm.identity_busy = false;
        match r {
            Ok(v) => {
                vm.android_id = Default::default();
                merge(&mut vm, &v);
            }
            Err(e) => toast(&c.app.as_weak(), e.message),
        }
        c.app.set_widgets(vm);
    });
    ctx.bridge.run(
        "identity.set",
        params,
        |_| {},
        move |r| crate::app::fire(token, r),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_ids_are_valid() {
        for _ in 0..50 {
            assert!(check_android_id(&random_android_id(Uuid::new_v4())).is_ok());
        }
    }

    #[test]
    fn the_draft_is_checked_and_filled_once() {
        let mut vm = WidgetsVm::default();
        merge(
            &mut vm,
            &json!({ "android_id": "1f3842600e7f0f09", "imei": "867400022047199" }),
        );
        assert_eq!(vm.android_id, "1f3842600e7f0f09");
        assert_eq!(vm.imei_now, "867400022047199");
        vm.android_id = "0123456789ABCDEF".into();
        merge(
            &mut vm,
            &json!({ "android_id": "1f3842600e7f0f09", "imei": null }),
        );
        assert_eq!(vm.android_id, "0123456789ABCDEF");
        assert_eq!(vm.imei_now, "");
        assert_eq!(
            set_params(&vm).unwrap(),
            json!({ "android_id": "0123456789abcdef" })
        );
        vm.android_id = "XYZ".into();
        assert!(set_params(&vm).is_err());
    }
}
