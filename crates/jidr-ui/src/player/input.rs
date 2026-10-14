//! Window input → emulator input: touch points in device pixels, gestures
//! with modifiers, keys as W3C key names, wheel.

use jidr_proto::control as pb;

/// Device size in pixels in its current orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Screen {
    pub width: u32,
    pub height: u32,
}

impl Screen {
    /// Normalized (0..1) → device pixels, clamped to the screen.
    pub fn to_px(&self, nx: f32, ny: f32) -> (i32, i32) {
        let x = (nx.clamp(0.0, 1.0) * self.width as f32).min(self.width.saturating_sub(1) as f32);
        let y = (ny.clamp(0.0, 1.0) * self.height as f32).min(self.height.saturating_sub(1) as f32);
        (x as i32, y as i32)
    }
}

/// Multi-touch with modifiers (our own bindings; Genymotion's aren't public).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Gesture {
    #[default]
    Single,
    /// ⌥ drag: a second finger mirrors the first around the screen centre
    /// (pinch / zoom / rotate).
    Pinch,
    /// ⇧⌥ drag: a second finger follows at a fixed offset (two-finger pan).
    Pan,
}

impl Gesture {
    pub fn from_mods(alt: bool, shift: bool) -> Self {
        match (alt, shift) {
            (true, true) => Gesture::Pan,
            (true, false) => Gesture::Pinch,
            _ => Gesture::Single,
        }
    }
}

/// Touch points (id, x, y) for a pointer at `p` under `g`.
pub fn fingers(g: Gesture, screen: Screen, p: (i32, i32)) -> Vec<(i32, i32, i32)> {
    let (w, h) = (screen.width as i32, screen.height as i32);
    let clamp = |x: i32, max: i32| x.clamp(0, (max - 1).max(0));
    match g {
        Gesture::Single => vec![(1, p.0, p.1)],
        Gesture::Pinch => vec![(1, p.0, p.1), (2, clamp(w - p.0, w), clamp(h - p.1, h))],
        Gesture::Pan => {
            let dx = (w / 8).max(40);
            vec![(1, p.0, p.1), (2, clamp(p.0 + dx, w), p.1)]
        }
    }
}

/// A touch event; `down == false` lifts every finger (pressure 0).
pub fn touch_event(points: &[(i32, i32, i32)], down: bool) -> pb::InputEvent {
    pb::InputEvent {
        r#type: Some(pb::input_event::Type::TouchEvent(pb::TouchEvent {
            touches: points
                .iter()
                .map(|&(id, x, y)| pb::Touch {
                    x,
                    y,
                    identifier: id,
                    pressure: if down { 100 } else { 0 },
                    touch_major: if down { 5 } else { 0 },
                    expiration: pb::touch::EventExpiration::NeverExpire as i32,
                    ..Default::default()
                })
                .collect(),
            display: 0,
        })),
    }
}

/// Slint's key text → W3C `KeyboardEvent.key` for non-printing keys.
pub fn special_key(text: &str) -> Option<&'static str> {
    Some(match text {
        "\u{8}" => "Backspace",
        "\t" => "Tab",
        "\n" | "\r" => "Enter",
        "\u{1b}" => "Escape",
        "\u{7f}" => "Delete",
        "\u{f700}" => "ArrowUp",
        "\u{f701}" => "ArrowDown",
        "\u{f702}" => "ArrowLeft",
        "\u{f703}" => "ArrowRight",
        "\u{f729}" => "Home",
        "\u{f72b}" => "End",
        "\u{f72c}" => "PageUp",
        "\u{f72d}" => "PageDown",
        _ => return None,
    })
}

/// Modifier-only key presses that shouldn't reach the device.
fn is_modifier(text: &str) -> bool {
    matches!(
        text,
        "\u{10}"
            | "\u{11}"
            | "\u{12}"
            | "\u{13}"
            | "\u{14}"
            | "\u{15}"
            | "\u{16}"
            | "\u{17}"
            | "\u{18}"
    )
}

/// A key press or release → emulator keyboard event (printable text is
/// sent once, on press).
pub fn key_event(text: &str, down: bool) -> Option<pb::KeyboardEvent> {
    if text.is_empty() || is_modifier(text) {
        return None;
    }
    if let Some(key) = special_key(text) {
        return Some(pb::KeyboardEvent {
            code_type: pb::keyboard_event::KeyCodeType::Usb as i32,
            event_type: if down {
                pb::keyboard_event::KeyEventType::Keydown as i32
            } else {
                pb::keyboard_event::KeyEventType::Keyup as i32
            },
            key: key.into(),
            ..Default::default()
        });
    }
    // Private-use codes are other function keys we don't map.
    if text
        .chars()
        .any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c) || c.is_control())
        || !down
    {
        return None;
    }
    Some(pb::KeyboardEvent {
        code_type: pb::keyboard_event::KeyCodeType::Usb as i32,
        event_type: pb::keyboard_event::KeyEventType::Keypress as i32,
        text: text.into(),
        ..Default::default()
    })
}

/// Toolbar buttons that are hardware keys (W3C key names the emulator knows).
pub fn tool_key(tool: &str) -> Option<&'static str> {
    Some(match tool {
        "back" => "GoBack",
        "home" => "GoHome",
        "recents" => "AppSwitch",
        "volume-up" => "AudioVolumeUp",
        "volume-down" => "AudioVolumeDown",
        "power" => "Power",
        _ => return None,
    })
}

pub fn press(key: &str) -> pb::KeyboardEvent {
    pb::KeyboardEvent {
        code_type: pb::keyboard_event::KeyCodeType::Usb as i32,
        event_type: pb::keyboard_event::KeyEventType::Keypress as i32,
        key: key.into(),
        ..Default::default()
    }
}

/// Window scroll (logical px, macOS natural direction) → wheel clicks.
pub fn wheel(dx: f32, dy: f32) -> pb::WheelEvent {
    // About 40 px per notch; the emulator wants 120 units per notch.
    pb::WheelEvent {
        dx: (dx * 3.0).round() as i32,
        dy: (dy * 3.0).round() as i32,
        display: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Screen = Screen {
        width: 1080,
        height: 2400,
    };

    #[test]
    fn normalized_points_map_to_device_pixels() {
        assert_eq!(S.to_px(0.5, 0.5), (540, 1200));
        assert_eq!(S.to_px(-1.0, 2.0), (0, 2399));
        assert_eq!(S.to_px(1.0, 1.0), (1079, 2399));
    }

    #[test]
    fn gestures_add_a_second_finger() {
        assert_eq!(fingers(Gesture::Single, S, (100, 200)), vec![(1, 100, 200)]);
        assert_eq!(
            fingers(Gesture::Pinch, S, (100, 200)),
            vec![(1, 100, 200), (2, 980, 2200)]
        );
        let pan = fingers(Gesture::Pan, S, (100, 200));
        assert_eq!(pan[1], (2, 235, 200));
        assert_eq!(Gesture::from_mods(true, true), Gesture::Pan);
        assert_eq!(Gesture::from_mods(false, true), Gesture::Single);
    }

    #[test]
    fn touch_release_has_zero_pressure() {
        let ev = touch_event(&[(1, 10, 20)], false);
        let Some(pb::input_event::Type::TouchEvent(t)) = ev.r#type else {
            panic!()
        };
        assert_eq!(t.touches[0].pressure, 0);
        assert_eq!(
            t.touches[0].expiration,
            pb::touch::EventExpiration::NeverExpire as i32
        );
    }

    #[test]
    fn keys_map_to_w3c_names_and_text() {
        let k = key_event("\u{8}", true).unwrap();
        assert_eq!(k.key, "Backspace");
        assert_eq!(
            k.event_type,
            pb::keyboard_event::KeyEventType::Keydown as i32
        );
        assert_eq!(key_event("\u{f702}", false).unwrap().key, "ArrowLeft");
        let a = key_event("a", true).unwrap();
        assert_eq!(a.text, "a");
        assert!(key_event("a", false).is_none());
        assert!(key_event("\u{10}", true).is_none());
        assert!(key_event("\u{f704}", true).is_none());
        assert_eq!(tool_key("home"), Some("GoHome"));
        assert_eq!(tool_key("screenshot"), None);
    }
}
