//! Widget panels → `jidr-widgets` commands. Controls update the view model
//! at once, then call the daemon; a failure shows a toast and reloads.

use std::rc::Rc;

use serde_json::{Value, json};
use slint::ComponentHandle;
use uuid::Uuid;

use crate::WidgetsVm;
use crate::app::{Ctx, toast};

pub mod gps;

pub const PROFILES: [&str; 7] = ["full", "lte", "hsdpa", "umts", "edge", "gprs", "gsm"];
pub const SCALES: [f32; 7] = [0.0, 0.5, 1.0, 1.5, 2.0, 5.0, 10.0];
pub const CAMERAS: [&str; 4] = ["emulated", "webcam0", "virtualscene", "none"];

/// Defaults before the device answers.
pub fn default_vm() -> WidgetsVm {
    WidgetsVm {
        battery_level: 100,
        animation: 2,
        light: "400".into(),
        temperature: "25".into(),
        pressure: "1013".into(),
        humidity: "40".into(),
        ..Default::default()
    }
}

/// A panel action → (method, params); `None` for view-only changes, which
/// are applied to `vm` directly.
pub fn command(vm: &mut WidgetsVm, action: &str, value: &str) -> Option<(&'static str, Value)> {
    let on = value == "true";
    let num = || value.trim().parse::<f64>().ok();
    let idx = || value.trim().parse::<usize>().ok();
    Some(match action {
        "battery.level" => {
            let l = num()?.clamp(0.0, 100.0).round() as i32;
            vm.battery_level = l;
            ("battery.set", json!({ "level": l }))
        }
        "battery.charging" => {
            vm.charging = on;
            ("battery.set", json!({ "charging": on }))
        }
        "network.wifi" => {
            vm.wifi = on;
            ("network.set", json!({ "wifi": on }))
        }
        "network.mobile" => {
            vm.mobile = on;
            ("network.set", json!({ "mobile": on }))
        }
        "network.profile" => {
            let i = idx()?;
            vm.profile = i as i32;
            ("network.set", json!({ "profile": PROFILES.get(i)? }))
        }
        "phone.signal" => {
            let i = idx()?.min(4);
            vm.signal = i as i32;
            ("phone.gsm", json!({ "signal": i }))
        }
        "sensor.rotate" => {
            let a = num()? as u16;
            vm.rotation = i32::from(a / 90);
            ("sensor.rotate", json!({ "angle": a }))
        }
        "sensor.motion" => {
            let g = 9.80665;
            let v = match value {
                "upright" => [0.0, g, 0.0],
                "flat" => [0.0, 0.0, g],
                "upside-down" => [0.0, -g, 0.0],
                _ => return None,
            };
            (
                "sensor.set",
                json!({ "sensor": "acceleration", "values": v }),
            )
        }
        "sensor.light" | "sensor.temperature" | "sensor.pressure" | "sensor.humidity" => {
            let v = num()?;
            let s = action.trim_start_matches("sensor.");
            match s {
                "light" => vm.light = value.into(),
                "temperature" => vm.temperature = value.into(),
                "pressure" => vm.pressure = value.into(),
                _ => vm.humidity = value.into(),
            }
            ("sensor.set", json!({ "sensor": s, "values": [v] }))
        }
        "sensor.proximity" => {
            vm.proximity_near = value == "near";
            let cm = if vm.proximity_near { 0.0 } else { 5.0 };
            (
                "sensor.set",
                json!({ "sensor": "proximity", "values": [cm] }),
            )
        }
        "phone.number" => {
            vm.phone_number = value.into();
            return None;
        }
        "phone.sms-text" => {
            vm.sms_text = value.into();
            return None;
        }
        "phone.call" => (
            "phone.call",
            json!({ "number": vm.phone_number.as_str(), "action": value }),
        ),
        "phone.sms" => (
            "phone.sms",
            json!({ "from": vm.phone_number.as_str(), "text": vm.sms_text.as_str() }),
        ),
        "key.press" => ("key.press", json!({ "key": value })),
        "dev.show_taps" => {
            vm.show_taps = on;
            ("dev.set", json!({ "show_taps": on }))
        }
        "dev.pointer_location" => {
            vm.pointer_location = on;
            ("dev.set", json!({ "pointer_location": on }))
        }
        "dev.layout_bounds" => {
            vm.layout_bounds = on;
            ("dev.set", json!({ "layout_bounds": on }))
        }
        "dev.animation" => {
            let i = idx()?;
            vm.animation = i as i32;
            ("dev.set", json!({ "animation_scale": SCALES.get(i)? }))
        }
        "clipboard.text" => {
            vm.clipboard = value.into();
            return None;
        }
        "clipboard.set" => ("clipboard.set", json!({ "text": vm.clipboard.as_str() })),
        "clipboard.get" => ("clipboard.get", json!({})),
        "fingerprint.touch" => ("fingerprint.touch", json!({ "finger": idx()? })),
        "fingerprint.remove" => ("fingerprint.remove", json!({})),
        "camera.back" => {
            let i = idx()?;
            vm.camera_back = i as i32;
            ("camera.set", json!({ "back": CAMERAS.get(i)? }))
        }
        _ => return None,
    })
}

/// Merges `*.get` answers into the view model.
pub fn merge(vm: &mut WidgetsVm, method: &str, v: &Value) {
    match method {
        "battery.get" => {
            if let Some(l) = v.get("level").and_then(Value::as_i64) {
                vm.battery_level = l as i32;
            }
            vm.charging = v.get("charging").and_then(Value::as_bool).unwrap_or(false);
            vm.battery_health = v
                .get("health")
                .and_then(Value::as_str)
                .unwrap_or("good")
                .into();
        }
        "network.get" => {
            vm.wifi = v.get("wifi").and_then(Value::as_bool).unwrap_or(true);
            vm.mobile = v.get("mobile").and_then(Value::as_bool).unwrap_or(true);
        }
        "dev.get" => {
            vm.show_taps = v.get("show_taps").and_then(Value::as_bool).unwrap_or(false);
            vm.pointer_location = v
                .get("pointer_location")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            vm.layout_bounds = v
                .get("layout_bounds")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if let Some(s) = v.get("animation_scale").and_then(Value::as_f64) {
                vm.animation = SCALES
                    .iter()
                    .position(|x| (f64::from(*x) - s).abs() < 0.01)
                    .unwrap_or(2) as i32;
            }
        }
        "camera.get" => {
            let back = v.get("back").and_then(Value::as_str).unwrap_or("default");
            vm.camera_back = CAMERAS.iter().position(|c| *c == back).unwrap_or(0) as i32;
        }
        "clipboard.get" => {
            vm.clipboard = v
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into();
        }
        _ => {}
    }
}

fn active(ctx: &Ctx) -> Option<Uuid> {
    let st = ctx.state.borrow();
    st.active.and_then(|i| st.tabs.get(i)).copied()
}

fn call_and_merge(ctx: &Rc<Ctx>, device: Uuid, method: &'static str, mut params: Value) {
    params["device"] = json!(device);
    let token = crate::app::defer(move |r: jidr_core::JidrResult<Value>| {
        let Some(c) = crate::app::ctx() else { return };
        match r {
            Ok(v) => {
                let mut vm = c.app.get_widgets();
                merge(&mut vm, method, &v);
                c.app.set_widgets(vm);
                if method == "gps.get" {
                    gps::refresh_map(&c);
                }
            }
            Err(e) if method.ends_with(".get") => tracing::debug!("{method}: {}", e.message),
            Err(e) => {
                toast(&c.app.as_weak(), e.message);
                load(&c);
            }
        }
    });
    ctx.bridge
        .call(method, params, move |r| crate::app::fire(token, r));
}

/// Loads the panels' values for the active device.
pub fn load(ctx: &Rc<Ctx>) {
    let Some(id) = active(ctx) else { return };
    for m in [
        "battery.get",
        "network.get",
        "dev.get",
        "camera.get",
        "gps.get",
    ] {
        call_and_merge(ctx, id, m, json!({}));
    }
}

pub fn wire(ctx: &Rc<Ctx>) {
    ctx.app.set_widgets(default_vm());
    let c = ctx.clone();
    ctx.app.on_widget_action(move |action, value| {
        let Some(id) = active(&c) else { return };
        if gps_action(&c, id, &action, &value) {
            return;
        }
        let mut vm = c.app.get_widgets();
        let cmd = command(&mut vm, &action, &value);
        c.app.set_widgets(vm);
        if let Some((method, params)) = cmd {
            call_and_merge(&c, id, method, params);
        }
    });
    let c = ctx.clone();
    ctx.app.on_widget_tab(move |_| load(&c));
    let c = ctx.clone();
    ctx.app.on_gps_map_click(move |x, y| {
        if let Some(id) = active(&c) {
            gps::map_click(&c, id, x, y);
        }
    });
}

/// GPS panel actions; true when handled.
fn gps_action(ctx: &Rc<Ctx>, id: Uuid, action: &str, value: &str) -> bool {
    let mut vm = ctx.app.get_widgets();
    match action {
        "gps.lat" => vm.gps_lat = value.into(),
        "gps.lon" => vm.gps_lon = value.into(),
        "gps.alt" => vm.gps_alt = value.into(),
        "gps.bearing" => vm.gps_bearing = value.into(),
        "gps.set" => {
            gps::set_location(ctx, id);
            return true;
        }
        "gps.route.choose" => {
            let token = crate::app::defer(move |path: Option<std::path::PathBuf>| {
                if let (Some(p), Some(c)) = (path, crate::app::ctx()) {
                    gps::start_route(&c, id, p);
                }
            });
            std::thread::spawn(move || crate::app::fire(token, gps::choose_file()));
            return true;
        }
        "gps.route.toggle" => {
            if vm.route_active {
                vm.route_paused = !vm.route_paused;
                let paused = vm.route_paused;
                ctx.app.set_widgets(vm);
                ctx.bridge.call(
                    "gps.route_control",
                    json!({ "device": id, "paused": paused }),
                    |_| {},
                );
            } else if let Some(p) = gps::ROUTE_FILE.with(|f| f.borrow().clone()) {
                gps::start_route(ctx, id, p);
            }
            return true;
        }
        "gps.route.speed" => {
            let i = value.parse::<usize>().unwrap_or(0).min(2);
            vm.route_speed = i as i32;
            if vm.route_active {
                ctx.bridge.call(
                    "gps.route_control",
                    json!({ "device": id, "speed": gps::SPEEDS[i] }),
                    |_| {},
                );
            }
        }
        "gps.route.loop" => {
            vm.route_loop = value == "true";
            if vm.route_active {
                ctx.bridge.call(
                    "gps.route_control",
                    json!({ "device": id, "loop": vm.route_loop }),
                    |_| {},
                );
            }
        }
        _ => return false,
    }
    ctx.app.set_widgets(vm);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_become_commands() {
        let mut vm = default_vm();
        let (m, p) = command(&mut vm, "battery.level", "64").unwrap();
        assert_eq!((m, p["level"].as_i64()), ("battery.set", Some(64)));
        assert_eq!(vm.battery_level, 64);
        let (_, p) = command(&mut vm, "network.profile", "4").unwrap();
        assert_eq!(p["profile"], "edge");
        let (_, p) = command(&mut vm, "dev.animation", "1").unwrap();
        assert_eq!(p["animation_scale"], 0.5);
        assert!(command(&mut vm, "phone.number", "+218 91").is_none());
        let (m, p) = command(&mut vm, "phone.call", "incoming").unwrap();
        assert_eq!((m, p["number"].as_str()), ("phone.call", Some("+218 91")));
        assert!(command(&mut vm, "battery.level", "x").is_none());
        let (_, p) = command(&mut vm, "camera.back", "1").unwrap();
        assert_eq!(p["back"], "webcam0");
    }

    #[test]
    fn get_answers_merge() {
        let mut vm = default_vm();
        merge(
            &mut vm,
            "battery.get",
            &json!({"level": 42, "charging": true, "health": "good"}),
        );
        merge(
            &mut vm,
            "dev.get",
            &json!({"show_taps": true, "animation_scale": 0.5}),
        );
        assert_eq!(
            (vm.battery_level, vm.charging, vm.show_taps, vm.animation),
            (42, true, true, 1)
        );
    }
}
