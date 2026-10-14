//! Android controls (hardware keys), developer toggles (show taps, pointer
//! location, layout bounds, animation scales) and the shared clipboard.

use jidr_core::provider::{CallCtx, CommandSpec, DeviceParams, parse_params};
use jidr_core::{JidrError, JidrResult};
use jidr_proto::control as pb;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::device::{grpc_err, target};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyParams {
    pub device: String,
    /// `back`, `home`, `recents`, `volume_up`, `volume_down`, `mute`,
    /// `power`, `menu`, `dpad_up|down|left|right|center`, `play_pause`,
    /// `next`, `previous`, `enter`, `delete`, or any W3C key name.
    pub key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DevParams {
    pub device: String,
    /// Show touches (`show_touches`).
    #[serde(default)]
    pub show_taps: Option<bool>,
    /// Pointer location overlay.
    #[serde(default)]
    pub pointer_location: Option<bool>,
    /// Show layout bounds (`debug.layout`).
    #[serde(default)]
    pub layout_bounds: Option<bool>,
    /// Window, transition and animator scales (0, 0.5, 1, 1.5, 2, 5, 10).
    #[serde(default)]
    pub animation_scale: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipParams {
    pub device: String,
    pub text: String,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new::<KeyParams>(
            "key.press",
            "Press a hardware key (back, home, volume, D-pad, media…)",
        ),
        CommandSpec::new::<DeviceParams>(
            "dev.get",
            "Developer toggles: show taps, pointer location, layout bounds, animation scale",
        ),
        CommandSpec::new::<DevParams>("dev.set", "Set developer toggles"),
        CommandSpec::new::<DeviceParams>("clipboard.get", "The device clipboard text"),
        CommandSpec::new::<ClipParams>("clipboard.set", "Set the device clipboard text"),
    ]
}

/// Our key names → W3C `KeyboardEvent.key` values the emulator accepts.
pub fn key_name(k: &str) -> JidrResult<String> {
    let w3c = match k {
        "back" => "GoBack",
        "home" => "GoHome",
        "recents" | "app_switch" => "AppSwitch",
        "volume_up" => "AudioVolumeUp",
        "volume_down" => "AudioVolumeDown",
        "mute" => "AudioVolumeMute",
        "power" => "Power",
        "menu" => "ContextMenu",
        "dpad_up" => "ArrowUp",
        "dpad_down" => "ArrowDown",
        "dpad_left" => "ArrowLeft",
        "dpad_right" => "ArrowRight",
        "dpad_center" | "enter" => "Enter",
        "delete" => "Backspace",
        "play_pause" => "MediaPlayPause",
        "next" => "MediaTrackNext",
        "previous" => "MediaTrackPrevious",
        other if !other.is_empty() && other.chars().all(|c| c.is_ascii_alphanumeric()) => other,
        other => return Err(JidrError::usage(format!("unknown key {other:?}"))),
    };
    Ok(w3c.to_string())
}

/// Hardware buttons go in as Linux evdev key codes (down, then up), the way
/// the emulator's own side buttons do; W3C names don't move the volume.
pub fn evdev_code(k: &str) -> Option<i32> {
    Some(match k {
        "mute" => 113,
        "volume_down" => 114,
        "volume_up" => 115,
        "power" => 116,
        "menu" => 139,
        "back" => 158,
        "recents" | "app_switch" => 580,
        "play_pause" => 164,
        "next" => 163,
        "previous" => 165,
        _ => return None,
    })
}

/// The keyboard events for a key name.
pub fn key_events(k: &str) -> JidrResult<Vec<pb::KeyboardEvent>> {
    use pb::keyboard_event::{KeyCodeType, KeyEventType};
    if let Some(code) = evdev_code(k) {
        return Ok([KeyEventType::Keydown, KeyEventType::Keyup]
            .into_iter()
            .map(|t| pb::KeyboardEvent {
                code_type: KeyCodeType::Evdev as i32,
                event_type: t as i32,
                key_code: code,
                ..Default::default()
            })
            .collect());
    }
    Ok(vec![pb::KeyboardEvent {
        code_type: KeyCodeType::Usb as i32,
        event_type: KeyEventType::Keypress as i32,
        key: key_name(k)?,
        ..Default::default()
    }])
}

pub const SCALES: [f32; 7] = [0.0, 0.5, 1.0, 1.5, 2.0, 5.0, 10.0];

fn flag(v: bool) -> &'static str {
    if v { "1" } else { "0" }
}

pub async fn call(ctx: &CallCtx, method: &str, params: Value) -> JidrResult<Value> {
    match method {
        "key.press" => {
            let p: KeyParams = parse_params(params)?;
            let events = key_events(&p.key)?;
            let t = target(ctx, &p.device).await?;
            let mut c = t.grpc().await?;
            for ev in events {
                c.send_key(ev).await.map_err(grpc_err("sendKey"))?;
            }
            Ok(json!({ "key": p.key }))
        }
        "dev.get" => {
            let p: DeviceParams = parse_params(params)?;
            let adb = target(ctx, &p.device).await?.adb();
            let show = adb.shell("settings get system show_touches").await?;
            let ptr = adb.shell("settings get system pointer_location").await?;
            let layout = adb.getprop("debug.layout").await.unwrap_or_default();
            let anim = adb
                .shell("settings get global window_animation_scale")
                .await?;
            Ok(json!({
                "show_taps": show.trim() == "1",
                "pointer_location": ptr.trim() == "1",
                "layout_bounds": layout.trim() == "true",
                "animation_scale": anim.trim().parse::<f32>().unwrap_or(1.0),
            }))
        }
        "dev.set" => {
            let p: DevParams = parse_params(params)?;
            if let Some(s) = p.animation_scale
                && !SCALES.contains(&s)
            {
                return Err(JidrError::usage(
                    "animation_scale must be 0, 0.5, 1, 1.5, 2, 5 or 10",
                ));
            }
            let adb = target(ctx, &p.device).await?.adb();
            if let Some(v) = p.show_taps {
                adb.shell(&format!("settings put system show_touches {}", flag(v)))
                    .await?;
            }
            if let Some(v) = p.pointer_location {
                adb.shell(&format!("settings put system pointer_location {}", flag(v)))
                    .await?;
            }
            if let Some(v) = p.layout_bounds {
                // setprop needs a nudge so running apps re-read it (SYSPROPS_TRANSACTION).
                adb.shell(&format!(
                    "setprop debug.layout {} && service call activity 1599295570 >/dev/null",
                    if v { "true" } else { "false" }
                ))
                .await?;
            }
            if let Some(s) = p.animation_scale {
                for key in [
                    "window_animation_scale",
                    "transition_animation_scale",
                    "animator_duration_scale",
                ] {
                    adb.shell(&format!("settings put global {key} {s}")).await?;
                }
            }
            Ok(json!({ "ok": true }))
        }
        "clipboard.get" => {
            let p: DeviceParams = parse_params(params)?;
            let t = target(ctx, &p.device).await?;
            let c = t
                .grpc()
                .await?
                .get_clipboard(())
                .await
                .map_err(grpc_err("getClipboard"))?;
            Ok(json!({ "text": c.into_inner().text }))
        }
        "clipboard.set" => {
            let p: ClipParams = parse_params(params)?;
            let t = target(ctx, &p.device).await?;
            t.grpc()
                .await?
                .set_clipboard(pb::ClipData { text: p.text })
                .await
                .map_err(grpc_err("setClipboard"))?;
            Ok(json!({ "ok": true }))
        }
        _ => Err(JidrError::usage(format!("unknown method {method}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_keys_are_evdev_down_up() {
        let ev = key_events("volume_up").unwrap();
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].key_code, 115);
        assert_eq!(
            ev[1].event_type,
            pb::keyboard_event::KeyEventType::Keyup as i32
        );
        assert_eq!(key_events("F5").unwrap()[0].key, "F5");
    }

    #[test]
    fn key_names_map_to_w3c() {
        assert_eq!(key_name("home").unwrap(), "GoHome");
        assert_eq!(key_name("dpad_left").unwrap(), "ArrowLeft");
        assert_eq!(key_name("F5").unwrap(), "F5");
        assert!(key_name("rm -rf").is_err());
        assert!(key_name("").is_err());
    }
}
