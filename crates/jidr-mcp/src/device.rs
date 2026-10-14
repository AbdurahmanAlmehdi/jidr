//! Typed device tools: screenshot, touch, keys, UI dump. They reach the
//! emulator directly (gRPC + adb) using `device.connect_info`, like the player.

use std::time::Duration;

use jidr_adb::Adb;
use jidr_core::{ConnectInfo, ErrorCode, JidrError, JidrResult};
use jidr_proto::Controller;
use jidr_proto::control::{
    ImageFormat, KeyboardEvent, image_format::ImgFormat, keyboard_event::KeyEventType,
};

/// Screenshots are fitted in a box of this size (plan: PNG ≤ 1024 px).
pub const SHOT_BOX: u32 = 1024;

pub struct DeviceIo {
    pub info: ConnectInfo,
    pub adb: Adb,
}

impl DeviceIo {
    pub fn new(info: ConnectInfo) -> Self {
        let adb = Adb::new(&info.adb_path)
            .device(&info.adb_serial)
            .with_timeout(Duration::from_secs(60));
        Self { info, adb }
    }

    async fn grpc(&self) -> JidrResult<Controller> {
        Ok(
            jidr_backend::grpc::connect_port(self.info.grpc_port, &self.info.grpc_token)
                .await?
                .0,
        )
    }

    /// The current screen size in device pixels, rotation applied.
    pub async fn screen_size(&self) -> JidrResult<(u32, u32)> {
        let wm = self.adb.shell("wm size").await?;
        let (w, h) = parse_wm_size(&wm).ok_or_else(|| {
            JidrError::unexpected(format!("can't read the screen size from {wm:?}"))
        })?;
        let input = self
            .adb
            .shell("dumpsys input | grep -m1 SurfaceOrientation")
            .await
            .unwrap_or_default();
        let rotated = input
            .split(':')
            .nth(1)
            .and_then(|o| o.trim().parse::<u32>().ok())
            .is_some_and(|o| o % 2 == 1);
        Ok(if rotated { (h, w) } else { (w, h) })
    }

    /// PNG fitted in `SHOT_BOX`; returns (png, width, height).
    pub async fn screenshot(&self) -> JidrResult<(Vec<u8>, u32, u32)> {
        let mut c = self.grpc().await?;
        // ADR-0001: ask for width *and* height, or the emulator ignores the size.
        let fmt = ImageFormat {
            format: ImgFormat::Png as i32,
            width: SHOT_BOX,
            height: SHOT_BOX,
            ..Default::default()
        };
        let img = c
            .get_screenshot(fmt)
            .await
            .map_err(|s| jidr_backend::grpc::status_error("screenshot", s))?
            .into_inner();
        if img.image.is_empty() {
            return Err(JidrError::unexpected(
                "the display is off (empty screenshot); press_key Power to wake it",
            ));
        }
        let (w, h) = png_size(&img.image).unwrap_or((0, 0));
        Ok((img.image, w, h))
    }

    pub async fn tap(&self, x: u32, y: u32) -> JidrResult<()> {
        self.adb
            .shell(&format!("input tap {x} {y}"))
            .await
            .map(|_| ())
    }

    pub async fn swipe(&self, from: (u32, u32), to: (u32, u32), ms: u32) -> JidrResult<()> {
        self.adb
            .shell(&format!(
                "input swipe {} {} {} {} {ms}",
                from.0, from.1, to.0, to.1
            ))
            .await
            .map(|_| ())
    }

    /// Types into the focused field with `input text`. (gRPC `sendKey` text
    /// was flaky here: in the agent tests it sometimes dismissed the Settings
    /// search screen it was typing into; `input text` never did.)
    pub async fn type_text(&self, text: &str) -> JidrResult<()> {
        if text.is_empty() {
            return Ok(());
        }
        if !text.chars().all(|c| c.is_ascii() && !c.is_ascii_control()) {
            return Err(JidrError::usage(
                "type_text supports printable ASCII; paste other text with command_run clipboard commands",
            ));
        }
        self.adb.shell(&input_text_command(text)).await.map(|_| ())
    }

    /// A key by its web name (`GoHome`, `GoBack`, `AppSwitch`, `Enter`, `Power`, …).
    pub async fn press_key(&self, key: &str) -> JidrResult<()> {
        let mut c = self.grpc().await?;
        let ev = KeyboardEvent {
            key: key.to_string(),
            event_type: KeyEventType::Keypress as i32,
            ..Default::default()
        };
        c.send_key(ev)
            .await
            .map_err(|s| jidr_backend::grpc::status_error("press_key", s))?;
        Ok(())
    }

    /// The UI hierarchy as uiautomator XML.
    pub async fn ui_dump(&self) -> JidrResult<String> {
        let out = self
            .adb
            .shell("uiautomator dump /sdcard/jidr_ui.xml >/dev/null && cat /sdcard/jidr_ui.xml")
            .await?;
        let start = out.find("<?xml").or_else(|| out.find("<hierarchy"));
        match start {
            Some(i) => Ok(out[i..].trim().to_string()),
            None => Err(JidrError::new(
                ErrorCode::Unexpected,
                format!("uiautomator gave no hierarchy: {}", out.trim()),
            )),
        }
    }
}

/// `input text '<text>'` for the device shell: spaces become `%s` (how
/// `input text` spells them) and the argument is single-quoted.
pub fn input_text_command(text: &str) -> String {
    let arg = text.replace(' ', "%s").replace('\'', "'\\''");
    format!("input text '{arg}'")
}

/// `Physical size: 1080x2400` / `Override size: 720x1600` (override wins).
pub fn parse_wm_size(text: &str) -> Option<(u32, u32)> {
    let pick = |label: &str| {
        text.lines().find_map(|l| {
            let rest = l.trim().strip_prefix(label)?.trim();
            let (w, h) = rest.split_once('x')?;
            Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
        })
    };
    pick("Override size:").or_else(|| pick("Physical size:"))
}

/// Width and height from a PNG's IHDR chunk.
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.get(..8)? != b"\x89PNG\r\n\x1a\n" || png.get(12..16)? != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(png.get(16..20)?.try_into().ok()?);
    let h = u32::from_be_bytes(png.get(20..24)?.try_into().ok()?);
    Some((w, h))
}

/// Scale from screenshot space to device pixels for a screen of `w`×`h`
/// fitted in `SHOT_BOX`.
pub fn shot_scale(w: u32, h: u32) -> f64 {
    let fit = (f64::from(SHOT_BOX) / f64::from(w.max(1)))
        .min(f64::from(SHOT_BOX) / f64::from(h.max(1)))
        .min(1.0);
    1.0 / fit
}

/// Maps a point given in screenshot space to device pixels, clamped.
pub fn to_device(p: (f64, f64), screen: (u32, u32)) -> (u32, u32) {
    let s = shot_scale(screen.0, screen.1);
    let clamp =
        |v: f64, max: u32| (v * s).round().clamp(0.0, f64::from(max.saturating_sub(1))) as u32;
    (clamp(p.0, screen.0), clamp(p.1, screen.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_text_is_shell_safe() {
        assert_eq!(input_text_command("battery"), "input text 'battery'");
        assert_eq!(input_text_command("hi there"), "input text 'hi%sthere'");
        assert_eq!(
            input_text_command("it's; rm -rf / & $(x)"),
            "input text 'it'\\''s;%srm%s-rf%s/%s&%s$(x)'"
        );
    }

    #[test]
    fn wm_size() {
        assert_eq!(
            parse_wm_size("Physical size: 1080x2400\n"),
            Some((1080, 2400))
        );
        assert_eq!(
            parse_wm_size("Physical size: 1080x2400\nOverride size: 720x1600"),
            Some((720, 1600))
        );
        assert_eq!(parse_wm_size("nope"), None);
    }

    #[test]
    fn png_header() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(461u32.to_be_bytes());
        png.extend(1024u32.to_be_bytes());
        assert_eq!(png_size(&png), Some((461, 1024)));
        assert_eq!(png_size(b"GIF89a"), None);
    }

    #[test]
    fn screenshot_space_maps_to_device_pixels() {
        // 1080×2400 fits 1024 tall: 460.8 wide, scale 2.34375.
        assert!((shot_scale(1080, 2400) - 2.34375).abs() < 1e-9);
        assert_eq!(to_device((230.0, 512.0), (1080, 2400)), (539, 1200));
        assert_eq!(
            to_device((5000.0, -3.0), (1080, 2400)),
            (1079, 0),
            "clamped"
        );
        // Small screens aren't upscaled.
        assert_eq!(shot_scale(720, 1000), 1.0);
    }
}
