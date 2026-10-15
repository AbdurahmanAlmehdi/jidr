//! GPS widget: set a fix (gRPC `setGps`) and replay GPX/KML routes as a
//! job that walks the track at a chosen speed (D14).

pub mod track;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use jidr_core::provider::{CallCtx, CommandSpec, DeviceParams, parse_params};
use jidr_core::{Cancel, JidrError, JidrResult};
use jidr_proto::control as pb;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::device::{grpc_err, target};
use track::{Point, Track};

/// Metres per second at 1× (a city drive).
pub const BASE_MPS: f64 = 10.0;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetParams {
    pub device: String,
    /// Decimal degrees, −90…90.
    pub latitude: f64,
    /// Decimal degrees, −180…180.
    pub longitude: f64,
    /// Metres above sea level.
    #[serde(default)]
    pub altitude: Option<f64>,
    /// Degrees clockwise from north.
    #[serde(default)]
    pub bearing: Option<f64>,
    /// Metres per second.
    #[serde(default)]
    pub speed: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouteParams {
    pub device: String,
    /// A `.gpx` or `.kml` file on this machine.
    pub path: PathBuf,
    /// Playback speed multiplier (1, 2, 4…).
    #[serde(default = "one")]
    pub speed: f32,
    /// Start again from the first point at the end.
    #[serde(default, rename = "loop")]
    pub repeat: bool,
    /// Metres per second at 1× (default 10, a city drive).
    #[serde(default)]
    pub mps: Option<f64>,
}

fn one() -> f32 {
    1.0
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ControlParams {
    pub device: String,
    #[serde(default)]
    pub speed: Option<f32>,
    #[serde(default)]
    pub paused: Option<bool>,
    #[serde(default, rename = "loop")]
    pub repeat: Option<bool>,
}

/// Live controls of a running route replay.
#[derive(Debug)]
pub struct RouteCtl {
    speed: AtomicU32,
    paused: AtomicBool,
    repeat: AtomicBool,
    cancel: Cancel,
    stop: AtomicBool,
}

impl RouteCtl {
    fn speed(&self) -> f32 {
        f32::from_bits(self.speed.load(Ordering::Relaxed))
    }
}

#[derive(Debug, Default)]
pub struct Routes(Mutex<HashMap<Uuid, Arc<RouteCtl>>>);

impl Routes {
    fn get(&self, id: Uuid) -> Option<Arc<RouteCtl>> {
        self.0.lock().ok()?.get(&id).cloned()
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new::<DeviceParams>("gps.get", "The current GPS fix"),
        CommandSpec::new::<SetParams>("gps.set", "Set the GPS location"),
        CommandSpec::new::<RouteParams>("gps.route", "Replay a GPX or KML route at a chosen speed")
            .long_running(),
        CommandSpec::new::<ControlParams>(
            "gps.route_control",
            "Change the running route's speed, pause or loop",
        ),
        CommandSpec::new::<DeviceParams>("gps.route_stop", "Stop the running route replay"),
    ]
}

pub fn check(lat: f64, lon: f64) -> JidrResult<()> {
    if !(-90.0..=90.0).contains(&lat)
        || !(-180.0..=180.0).contains(&lon)
        || lat.is_nan()
        || lon.is_nan()
    {
        return Err(JidrError::usage(
            "latitude must be −90…90 and longitude −180…180",
        ));
    }
    Ok(())
}

fn fix(p: Point, bearing: f64, speed: f64) -> pb::GpsState {
    pb::GpsState {
        passive_update: false,
        latitude: p.lat,
        longitude: p.lon,
        altitude: p.alt,
        bearing,
        speed,
        satellites: 12,
    }
}

/// "4.1 / 10.8 km · 6 min left at 2× · bearing 168°"
pub fn progress_text(
    done_m: f64,
    total_m: f64,
    mps: f64,
    speed: f32,
    bearing: f64,
    paused: bool,
) -> String {
    let left_s = ((total_m - done_m).max(0.0) / (mps * f64::from(speed)).max(0.001)).round() as i64;
    let left = if left_s >= 60 {
        format!("{} min left", (left_s + 59) / 60)
    } else {
        format!("{left_s} s left")
    };
    let sp = if speed.fract() == 0.0 {
        format!("{speed:.0}")
    } else {
        format!("{speed}")
    };
    format!(
        "{:.1} / {:.1} km · {}{left} at {sp}× · bearing {:.0}°",
        done_m / 1000.0,
        total_m / 1000.0,
        if paused { "paused · " } else { "" },
        bearing
    )
}

pub async fn call(
    ctx: &CallCtx,
    routes: &Routes,
    method: &str,
    params: Value,
) -> JidrResult<Value> {
    match method {
        "gps.get" => {
            let p: DeviceParams = parse_params(params)?;
            let t = target(ctx, &p.device).await?;
            let g = t
                .grpc()
                .await?
                .get_gps(())
                .await
                .map_err(grpc_err("getGps"))?
                .into_inner();
            Ok(json!({
                "latitude": g.latitude, "longitude": g.longitude, "altitude": g.altitude,
                "bearing": g.bearing, "speed": g.speed,
                "route": routes.get(t.device.id).is_some(),
            }))
        }
        "gps.set" => {
            let p: SetParams = parse_params(params)?;
            check(p.latitude, p.longitude)?;
            let t = target(ctx, &p.device).await?;
            let point = Point {
                lat: p.latitude,
                lon: p.longitude,
                alt: p.altitude.unwrap_or(0.0),
            };
            t.grpc()
                .await?
                .set_gps(fix(point, p.bearing.unwrap_or(0.0), p.speed.unwrap_or(0.0)))
                .await
                .map_err(grpc_err("setGps"))?;
            Ok(json!({ "latitude": p.latitude, "longitude": p.longitude }))
        }
        "gps.route" => {
            let p: RouteParams = parse_params(params)?;
            if !(p.speed > 0.0 && p.speed <= 64.0) {
                return Err(JidrError::usage("speed must be between 0 and 64×"));
            }
            let text = std::fs::read_to_string(&p.path)
                .map_err(|e| JidrError::usage(format!("cannot read {}: {e}", p.path.display())))?;
            let track = Track::new(track::parse(&text)?);
            let t = target(ctx, &p.device).await?;
            let ctl = Arc::new(RouteCtl {
                speed: AtomicU32::new(p.speed.to_bits()),
                paused: AtomicBool::new(false),
                repeat: AtomicBool::new(p.repeat),
                cancel: ctx.cancel.clone(),
                stop: AtomicBool::new(false),
            });
            if let Ok(mut m) = routes.0.lock()
                && let Some(old) = m.insert(t.device.id, ctl.clone())
            {
                old.stop.store(true, Ordering::Relaxed);
            }
            let r = replay(ctx, &t, &track, &ctl, p.mps.unwrap_or(BASE_MPS)).await;
            if let Ok(mut m) = routes.0.lock()
                && m.get(&t.device.id).is_some_and(|c| Arc::ptr_eq(c, &ctl))
            {
                m.remove(&t.device.id);
            }
            r
        }
        "gps.route_control" => {
            let p: ControlParams = parse_params(params)?;
            let d = ctx.daemon.device(&p.device).await?;
            let ctl = routes.get(d.id).ok_or_else(|| {
                JidrError::not_found(format!("no route is playing on {}", d.name))
            })?;
            if let Some(s) = p.speed {
                if !(s > 0.0 && s <= 64.0) {
                    return Err(JidrError::usage("speed must be between 0 and 64×"));
                }
                ctl.speed.store(s.to_bits(), Ordering::Relaxed);
            }
            if let Some(v) = p.paused {
                ctl.paused.store(v, Ordering::Relaxed);
            }
            if let Some(v) = p.repeat {
                ctl.repeat.store(v, Ordering::Relaxed);
            }
            Ok(json!({ "speed": ctl.speed(), "paused": ctl.paused.load(Ordering::Relaxed) }))
        }
        "gps.route_stop" => {
            let p: DeviceParams = parse_params(params)?;
            let d = ctx.daemon.device(&p.device).await?;
            match routes.get(d.id) {
                Some(c) => {
                    c.stop.store(true, Ordering::Relaxed);
                    Ok(json!({ "stopped": true }))
                }
                None => Ok(json!({ "stopped": false })),
            }
        }
        _ => Err(JidrError::usage(format!("unknown method {method}"))),
    }
}

/// Walks the track: one fix a second (more often at high speeds) until the
/// end, a stop, or cancel.
async fn replay(
    ctx: &CallCtx,
    t: &crate::device::Target,
    track: &Track,
    ctl: &RouteCtl,
    mps: f64,
) -> JidrResult<Value> {
    let mut grpc = t.grpc().await?;
    let total = track.length();
    let mut done = 0.0;
    let mut last = Instant::now();
    let mut laps = 0u32;
    loop {
        ctl.cancel.check()?;
        let paused = ctl.paused.load(Ordering::Relaxed);
        let speed = ctl.speed();
        let now = Instant::now();
        if !paused {
            done += mps * f64::from(speed) * now.duration_since(last).as_secs_f64();
        }
        last = now;
        let at_end = done >= total;
        let (p, b) = track.at(done.min(total));
        let v = if paused { 0.0 } else { mps * f64::from(speed) };
        grpc.set_gps(fix(p, b, v))
            .await
            .map_err(grpc_err("setGps"))?;
        ctx.report(
            Some(if total > 0.0 {
                (done / total).min(1.0) as f32
            } else {
                1.0
            }),
            &progress_text(done.min(total), total, mps, speed, b, paused),
        );
        if ctl.stop.load(Ordering::Relaxed) {
            break;
        }
        if at_end {
            if ctl.repeat.load(Ordering::Relaxed) {
                done = 0.0;
                laps += 1;
            } else {
                break;
            }
        }
        // Step so each fix advances at most ~50 m (smooth at 4× and up).
        let step = (50.0 / (mps * f64::from(speed)).max(1.0)).clamp(0.2, 1.0);
        tokio::time::sleep(Duration::from_secs_f64(step)).await;
    }
    Ok(json!({ "distance_m": done.min(total), "total_m": total, "laps": laps }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_reads_like_the_design() {
        assert_eq!(
            progress_text(4100.0, 10800.0, 9.3, 2.0, 168.0, false),
            "4.1 / 10.8 km · 6 min left at 2× · bearing 168°"
        );
        assert!(progress_text(0.0, 100.0, 10.0, 1.0, 0.0, true).contains("paused"));
    }

    #[test]
    fn coordinates_are_checked() {
        assert!(check(32.8954, 13.1802).is_ok());
        assert!(check(91.0, 0.0).is_err());
        assert!(check(0.0, f64::NAN).is_err());
    }
}
