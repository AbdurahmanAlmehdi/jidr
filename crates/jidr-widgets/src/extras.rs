//! The D36 single-command extras: fingerprint touch/remove (R8) and the
//! host webcam as the camera (R7). Mobile signal bars (R3) are `phone.gsm`.

use jidr_core::provider::{CallCtx, CommandSpec, DeviceParams, parse_params};
use jidr_core::{Device, JidrError, JidrResult};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::device::target;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FingerParams {
    pub device: String,
    /// Enrolled finger id (1–10). `touch` only.
    #[serde(default = "one")]
    pub finger: u8,
}

fn one() -> u8 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CameraSource {
    /// The emulator's animated test pattern.
    Emulated,
    /// The 3D virtual scene (back camera only).
    Virtualscene,
    /// The Mac's first webcam.
    Webcam0,
    /// The Mac's second webcam.
    Webcam1,
    None,
}

impl CameraSource {
    fn arg(self) -> &'static str {
        match self {
            CameraSource::Emulated => "emulated",
            CameraSource::Virtualscene => "virtualscene",
            CameraSource::Webcam0 => "webcam0",
            CameraSource::Webcam1 => "webcam1",
            CameraSource::None => "none",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CameraParams {
    pub device: String,
    #[serde(default)]
    pub back: Option<CameraSource>,
    #[serde(default)]
    pub front: Option<CameraSource>,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new::<FingerParams>(
            "fingerprint.touch",
            "Touch the sensor with an enrolled finger (recognised) or another (unrecognised)",
        ),
        CommandSpec::new::<DeviceParams>("fingerprint.remove", "Lift the finger off the sensor"),
        CommandSpec::new::<DeviceParams>("camera.get", "Camera sources used at the next start"),
        CommandSpec::new::<CameraParams>(
            "camera.set",
            "Use the host webcam (or the emulated camera) at the next start",
        )
        .locks_device(),
    ]
}

/// Replaces `-camera-back` / `-camera-front` in the launch args.
pub fn set_camera_args(args: &mut Vec<String>, flag: &str, source: CameraSource) {
    let mut out = Vec::with_capacity(args.len() + 2);
    let mut it = args.drain(..);
    while let Some(a) = it.next() {
        if a == flag {
            it.next();
            continue;
        }
        out.push(a);
    }
    drop(it);
    out.push(flag.to_string());
    out.push(source.arg().to_string());
    *args = out;
}

pub fn camera_of(d: &Device, flag: &str) -> Option<String> {
    d.launch
        .extra_args
        .iter()
        .position(|a| a == flag)
        .and_then(|i| d.launch.extra_args.get(i + 1))
        .cloned()
}

pub async fn call(ctx: &CallCtx, method: &str, params: Value) -> JidrResult<Value> {
    match method {
        "fingerprint.touch" => {
            let p: FingerParams = parse_params(params)?;
            if !(1..=10).contains(&p.finger) {
                return Err(JidrError::usage("finger is 1–10"));
            }
            let t = target(ctx, &p.device).await?;
            t.console_cmd(&format!("finger touch {}", p.finger)).await?;
            Ok(json!({ "finger": p.finger }))
        }
        "fingerprint.remove" => {
            let p: DeviceParams = parse_params(params)?;
            let t = target(ctx, &p.device).await?;
            t.console_cmd("finger remove").await?;
            Ok(json!({ "ok": true }))
        }
        "camera.get" => {
            let p: DeviceParams = parse_params(params)?;
            let d = ctx.daemon.device(&p.device).await?;
            Ok(json!({
                "back": camera_of(&d, "-camera-back").unwrap_or_else(|| "default".into()),
                "front": camera_of(&d, "-camera-front").unwrap_or_else(|| "default".into()),
            }))
        }
        "camera.set" => {
            let p: CameraParams = parse_params(params)?;
            if p.front == Some(CameraSource::Virtualscene) {
                return Err(JidrError::usage("the virtual scene is a back camera only"));
            }
            let d = ctx.daemon.device(&p.device).await?;
            let (back, front) = (p.back, p.front);
            let d = ctx
                .daemon
                .update_device(
                    d.id,
                    Box::new(move |d: &mut Device| {
                        if let Some(b) = back {
                            set_camera_args(&mut d.launch.extra_args, "-camera-back", b);
                        }
                        if let Some(f) = front {
                            set_camera_args(&mut d.launch.extra_args, "-camera-front", f);
                        }
                    }),
                )
                .await?;
            Ok(json!({
                "back": camera_of(&d, "-camera-back"),
                "front": camera_of(&d, "-camera-front"),
                "applies": "next start",
            }))
        }
        _ => Err(JidrError::usage(format!("unknown method {method}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_args_replace_the_old_value() {
        let mut a = vec![
            "-no-audio".to_string(),
            "-camera-back".into(),
            "emulated".into(),
        ];
        set_camera_args(&mut a, "-camera-back", CameraSource::Webcam0);
        assert_eq!(a, ["-no-audio", "-camera-back", "webcam0"]);
        set_camera_args(&mut a, "-camera-front", CameraSource::None);
        assert_eq!(
            a,
            [
                "-no-audio",
                "-camera-back",
                "webcam0",
                "-camera-front",
                "none"
            ]
        );
    }
}
