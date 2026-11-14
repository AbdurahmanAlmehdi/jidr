//! What `adb` says about physical phones (#92): `devices -l`, the
//! `track-devices` stream, `getprop`, `dumpsys battery`, key codes. Pure
//! parsing, so it's tested without a phone.

use std::collections::BTreeMap;

/// One line of `adb devices -l`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LongEntry {
    pub serial: String,
    /// adb's state, possibly several words (`no permissions (…); see […]`).
    pub state: String,
    /// `usb`, `product`, `model`, `device`, `transport_id`, …
    pub fields: BTreeMap<String, String>,
}

impl LongEntry {
    pub fn field(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }
}

const FIELD_KEYS: [&str; 6] = [
    "usb",
    "product",
    "model",
    "device",
    "transport_id",
    "features",
];

fn as_field(token: &str) -> Option<(&str, &str)> {
    let (k, v) = token.split_once(':')?;
    FIELD_KEYS.contains(&k).then_some((k, v))
}

/// Parses the device lines of `adb devices -l`, with or without the
/// `List of devices attached` header (`track-devices -l` frames have none).
pub fn parse_devices_long(text: &str) -> Vec<LongEntry> {
    text.lines()
        .filter(|l| !l.starts_with("List of devices") && !l.starts_with('*'))
        .filter_map(|l| {
            let mut tokens = l.split_whitespace();
            let serial = tokens.next()?.to_string();
            let mut state = Vec::new();
            let mut fields = BTreeMap::new();
            for t in tokens {
                match as_field(t) {
                    Some((k, v)) if !state.is_empty() => {
                        fields.insert(k.to_string(), v.to_string());
                    }
                    _ if fields.is_empty() => state.push(t),
                    _ => {}
                }
            }
            if state.is_empty() {
                return None;
            }
            Some(LongEntry {
                serial,
                state: state.join(" "),
                fields,
            })
        })
        .collect()
}

/// Takes the complete frames out of a `track-devices` buffer: each is four
/// hex digits of length, then that many bytes of `devices -l` lines. What's
/// left is an incomplete frame. `None` for a corrupt length.
pub fn take_frames(buf: &mut Vec<u8>) -> Option<Vec<String>> {
    let mut frames = Vec::new();
    loop {
        let Some(head) = buf.get(..4) else {
            return Some(frames);
        };
        let len = usize::from_str_radix(std::str::from_utf8(head).ok()?, 16).ok()?;
        if buf.len() < 4 + len {
            return Some(frames);
        }
        let body: Vec<u8> = buf.drain(..4 + len).skip(4).collect();
        frames.push(String::from_utf8_lossy(&body).into_owned());
    }
}

/// An emulator, not a phone: `emulator-5554`, or a localhost TCP serial
/// (Jidr's own `adb connect` past ~16 instances, D9).
pub fn is_emulator_serial(serial: &str) -> bool {
    serial
        .strip_prefix("emulator-")
        .is_some_and(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        || serial.starts_with("127.0.0.1:")
        || serial.starts_with("localhost:")
        || serial.starts_with("[::1]:")
}

/// The emulator's own props, for TCP serials that aren't obviously one.
pub fn props_say_emulator(props: &BTreeMap<String, String>) -> bool {
    let is = |k: &str, v: &str| props.get(k).is_some_and(|x| x == v);
    is("ro.kernel.qemu", "1")
        || is("ro.boot.qemu", "1")
        || is("ro.hardware", "ranchu")
        || is("ro.hardware", "goldfish")
}

/// Wireless debugging: `ip:port` or an mDNS service name.
pub fn is_wifi_serial(serial: &str) -> bool {
    serial.contains("._adb-tls-connect.")
        || serial
            .rsplit_once(':')
            .is_some_and(|(_, p)| p.parse::<u16>().is_ok())
}

/// `getprop` output: `[key]: [value]` lines.
pub fn parse_getprop(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let (k, v) = l.strip_prefix('[')?.split_once("]: [")?;
            Some((k.to_string(), v.strip_suffix(']')?.to_string()))
        })
        .collect()
}

/// `dumpsys battery` → (level 0–100, charging).
pub fn parse_battery(text: &str) -> Option<(u8, bool)> {
    let mut level = None;
    let mut scale = 100u32;
    let mut status = None;
    let mut powered = false;
    for l in text.lines() {
        let Some((k, v)) = l.trim().split_once(':') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "level" => level = v.parse::<u32>().ok(),
            "scale" => scale = v.parse().unwrap_or(100).max(1),
            "status" => status = v.parse::<u32>().ok(),
            "AC powered" | "USB powered" | "Wireless powered" | "Dock powered" => {
                powered |= v == "true";
            }
            _ => {}
        }
    }
    let pct = (level? * 100 / scale).min(100) as u8;
    // BatteryManager: 2 = charging, 5 = full (and still plugged in).
    let charging = match status {
        Some(2) => true,
        Some(5) => powered,
        Some(_) => false,
        None => powered,
    };
    Some((pct, charging))
}

/// An Android key code for a key name: Jidr's (`back`, `home`, `recents`,
/// `volume_up`, … as `key.press` takes them), W3C `KeyboardEvent.key`
/// values (`GoBack`, `Enter`, `ArrowUp`, …), `KEYCODE_*` names, or a bare
/// number.
pub fn android_keycode(name: &str) -> Option<u32> {
    if let Ok(n) = name.parse::<u32>() {
        return Some(n);
    }
    let code = match name {
        "home" | "GoHome" | "Home" | "KEYCODE_HOME" => 3,
        "back" | "GoBack" | "BrowserBack" | "KEYCODE_BACK" => 4,
        "dpad_up" | "ArrowUp" => 19,
        "dpad_down" | "ArrowDown" => 20,
        "dpad_left" | "ArrowLeft" => 21,
        "dpad_right" | "ArrowRight" => 22,
        "dpad_center" => 23,
        "volume_up" | "AudioVolumeUp" => 24,
        "volume_down" | "AudioVolumeDown" => 25,
        "power" | "Power" | "KEYCODE_POWER" => 26,
        "camera" | "Camera" => 27,
        "tab" | "Tab" => 61,
        "space" | " " => 62,
        "enter" | "Enter" => 66,
        "delete" | "Backspace" => 67,
        "menu" | "ContextMenu" => 82,
        "search" | "BrowserSearch" => 84,
        "play_pause" | "MediaPlayPause" => 85,
        "next" | "MediaTrackNext" => 87,
        "previous" | "MediaTrackPrevious" => 88,
        "escape" | "Escape" => 111,
        "forward_delete" | "Delete" => 112,
        "move_home" => 122,
        "move_end" | "End" => 123,
        "mute" | "AudioVolumeMute" => 164,
        "recents" | "app_switch" | "AppSwitch" => 187,
        "wakeup" | "WakeUp" => 224,
        "sleep" => 223,
        _ => {
            if let Some(code) = name.strip_prefix("KEYCODE_") {
                return keycode_by_name(code);
            }
            return single_char_keycode(name);
        }
    };
    Some(code)
}

/// The few `KEYCODE_*` names people type beyond the table above.
fn keycode_by_name(name: &str) -> Option<u32> {
    match name {
        "ENTER" => Some(66),
        "DEL" => Some(67),
        "APP_SWITCH" => Some(187),
        "VOLUME_UP" => Some(24),
        "VOLUME_DOWN" => Some(25),
        "MENU" => Some(82),
        _ => single_char_keycode(name),
    }
}

/// `a`–`z` and `0`–`9` (`KEYCODE_A` = 29, `KEYCODE_0` = 7).
fn single_char_keycode(name: &str) -> Option<u32> {
    let mut chars = name.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    match c.to_ascii_lowercase() {
        c @ 'a'..='z' => Some(29 + (c as u32 - 'a' as u32)),
        c @ '0'..='9' => Some(7 + (c as u32 - '0' as u32)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_with_fields_and_multi_word_states() {
        let text = "List of devices attached\n\
            R5CT1234ABC            device usb:1-1 product:shiba model:Pixel_8 device:shiba transport_id:3\n\
            0A1B2C3D               unauthorized usb:2-1 transport_id:4\n\
            emulator-5554          device product:sdk_gphone64_arm64 model:sdk_gphone64_arm64 device:emu64a transport_id:1\n\
            XYZ                    no permissions (missing udev rules? user is in the plugdev group); see [http://developer.android.com/tools/device.html] usb:1-2 transport_id:5\n\
            192.168.1.5:5555       offline transport_id:6\n\n";
        let e = parse_devices_long(text);
        assert_eq!(e.len(), 5);
        assert_eq!(e[0].serial, "R5CT1234ABC");
        assert_eq!(e[0].state, "device");
        assert_eq!(e[0].field("model"), Some("Pixel_8"));
        assert_eq!(e[0].field("usb"), Some("1-1"));
        assert_eq!(e[1].state, "unauthorized");
        assert_eq!(e[1].field("model"), None);
        assert!(e[3].state.starts_with("no permissions"));
        assert_eq!(e[3].field("usb"), Some("1-2"));
        assert_eq!(e[4].state, "offline");
        assert!(is_emulator_serial(&e[2].serial));
        assert!(!is_emulator_serial(&e[0].serial));
        assert!(is_emulator_serial("127.0.0.1:5585"));
        assert!(!is_emulator_serial("emulator-x"));
        assert!(is_wifi_serial(&e[4].serial));
        assert!(is_wifi_serial("adb-R5CT-abc._adb-tls-connect._tcp"));
        assert!(!is_wifi_serial("R5CT1234ABC"));
    }

    #[test]
    fn track_frames_split_on_their_length_prefix() {
        let a = "R5CT\tdevice usb:1-1\n";
        let mut buf = format!("{:04x}{a}0000{:04x}", a.len(), 30).into_bytes();
        buf.extend_from_slice(b"partial");
        let frames = take_frames(&mut buf).unwrap();
        assert_eq!(frames, vec![a.to_string(), String::new()]);
        assert_eq!(buf, format!("{:04x}partial", 30).into_bytes());
        let mut bad = b"zzzz".to_vec();
        assert!(take_frames(&mut bad).is_none());
    }

    #[test]
    fn getprop_and_battery() {
        let p = parse_getprop(
            "[ro.product.model]: [Pixel 8]\n[ro.build.version.sdk]: [34]\n[empty]: []\nnoise\n",
        );
        assert_eq!(p["ro.product.model"], "Pixel 8");
        assert_eq!(p["ro.build.version.sdk"], "34");
        assert_eq!(p["empty"], "");
        assert!(!props_say_emulator(&p));
        let mut emu = p.clone();
        emu.insert("ro.hardware".into(), "ranchu".into());
        assert!(props_say_emulator(&emu));

        let b = "Current Battery Service state:\n  AC powered: false\n  USB powered: true\n  status: 2\n  level: 87\n  scale: 100\n";
        assert_eq!(parse_battery(b), Some((87, true)));
        let b = "  USB powered: false\n  status: 3\n  level: 40\n  scale: 100\n";
        assert_eq!(parse_battery(b), Some((40, false)));
        assert_eq!(parse_battery("nothing"), None);
    }

    #[test]
    fn key_names_map_to_android_key_codes() {
        assert_eq!(android_keycode("home"), Some(3));
        assert_eq!(android_keycode("GoBack"), Some(4));
        assert_eq!(android_keycode("recents"), Some(187));
        assert_eq!(android_keycode("AppSwitch"), Some(187));
        assert_eq!(android_keycode("Enter"), Some(66));
        assert_eq!(android_keycode("KEYCODE_POWER"), Some(26));
        assert_eq!(android_keycode("KEYCODE_DEL"), Some(67));
        assert_eq!(android_keycode("a"), Some(29));
        assert_eq!(android_keycode("Z"), Some(54));
        assert_eq!(android_keycode("5"), Some(5), "a number is a key code");
        assert_eq!(android_keycode("F13"), None);
    }
}
