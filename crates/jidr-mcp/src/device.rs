//! Typed device tools: screenshot, touch, keys, UI dump. They reach the
//! emulator directly (gRPC + adb) using `device.connect_info`, like the player.

use std::time::Duration;

use jidr_adb::Adb;
use jidr_core::{ConnectInfo, ErrorCode, JidrError, JidrResult, PhoneConnectInfo};
use jidr_proto::Controller;
use jidr_proto::control::{
    ImageFormat, KeyboardEvent, image_format::ImgFormat, keyboard_event::KeyEventType,
};

/// Screenshots are fitted in a box of this size (plan: PNG ≤ 1024 px).
pub const SHOT_BOX: u32 = 1024;

pub struct DeviceIo {
    /// The emulator's gRPC endpoint and token; `None` for a USB phone,
    /// which only adb reaches (#92).
    grpc: Option<(u16, String)>,
    pub adb: Adb,
}

impl DeviceIo {
    pub fn new(info: ConnectInfo) -> Self {
        let adb = Adb::new(&info.adb_path)
            .device(&info.adb_serial)
            .with_timeout(Duration::from_secs(60));
        Self {
            grpc: Some((info.grpc_port, info.grpc_token)),
            adb,
        }
    }

    /// A ready USB phone: everything goes through adb.
    pub fn phone(info: PhoneConnectInfo) -> Self {
        let adb = Adb::new(&info.adb_path)
            .device(&info.serial)
            .with_timeout(Duration::from_secs(60));
        Self { grpc: None, adb }
    }

    async fn grpc(&self) -> JidrResult<Option<Controller>> {
        match &self.grpc {
            Some((port, token)) => Ok(Some(
                jidr_backend::grpc::connect_port(*port, token).await?.0,
            )),
            None => Ok(None),
        }
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
        let Some(mut c) = self.grpc().await? else {
            let png = self.adb.screencap_png().await?;
            return fit_png(&png, SHOT_BOX);
        };
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
        let Some(mut c) = self.grpc().await? else {
            let code = jidr_adb::physical::android_keycode(key)
                .ok_or_else(|| JidrError::usage(format!("unknown key {key:?}")))?;
            return self
                .adb
                .shell(&format!("input keyevent {code}"))
                .await
                .map(|_| ());
        };
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
        // On a phone, keep the dump out of the user's shared storage.
        let file = if self.grpc.is_some() {
            "/sdcard/jidr_ui.xml"
        } else {
            "/data/local/tmp/jidr_ui.xml"
        };
        let out = self
            .adb
            .shell(&format!("uiautomator dump {file} >/dev/null && cat {file}"))
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

pub use jidr_adb::input_text_command;

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

/// Scales a PNG down to fit a `max`×`max` box (box filter), as the
/// emulator does for its gRPC screenshots; returns (png, width, height).
pub fn fit_png(png: &[u8], max: u32) -> JidrResult<(Vec<u8>, u32, u32)> {
    let bad = |e: &dyn std::fmt::Display| JidrError::unexpected(format!("screenshot PNG: {e}"));
    let mut dec = png::Decoder::new(std::io::Cursor::new(png));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().map_err(|e| bad(&e))?;
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| bad(&"too large"))?
    ];
    let info = reader.next_frame(&mut buf).map_err(|e| bad(&e))?;
    let (w, h) = (info.width, info.height);
    let channels = info.color_type.samples();
    let k = (f64::from(max) / f64::from(w.max(h).max(1))).min(1.0);
    if k >= 1.0 {
        return Ok((png.to_vec(), w, h));
    }
    let (nw, nh) = (
        ((f64::from(w) * k).round() as u32).max(1),
        ((f64::from(h) * k).round() as u32).max(1),
    );
    let mut out = Vec::with_capacity((nw * nh * 3) as usize);
    for y in 0..nh {
        let (y0, y1) = (y * h / nh, ((y + 1) * h / nh).max(y * h / nh + 1));
        for x in 0..nw {
            let (x0, x1) = (x * w / nw, ((x + 1) * w / nw).max(x * w / nw + 1));
            let mut sum = [0u32; 3];
            let mut n = 0;
            for sy in y0..y1.min(h) {
                for sx in x0..x1.min(w) {
                    let i = (sy * w + sx) as usize * channels;
                    for (c, s) in sum.iter_mut().enumerate() {
                        // Grey images repeat their one channel.
                        *s += u32::from(buf.get(i + c.min(channels - 1)).copied().unwrap_or(0));
                    }
                    n += 1;
                }
            }
            out.extend(sum.iter().map(|s| (s / n.max(1)) as u8));
        }
    }
    let mut png_out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png_out, nw, nh);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().map_err(|e| bad(&e))?;
        wr.write_image_data(&out).map_err(|e| bad(&e))?;
    }
    Ok((png_out, nw, nh))
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
    fn phone_screenshots_are_fitted_in_the_box() {
        let mut png = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png, 40, 100);
            enc.set_color(png::ColorType::Rgba);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&vec![200u8; 40 * 100 * 4]).unwrap();
        }
        let (out, w, h) = fit_png(&png, 50).unwrap();
        assert_eq!((w, h), (20, 50));
        assert_eq!(png_size(&out), Some((20, 50)));
        let (same, w, h) = fit_png(&png, 1024).unwrap();
        assert_eq!(
            (w, h, same == png),
            (40, 100, true),
            "small screens stay as they are"
        );
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
