//! Rotation and motion/environment sensors (gRPC `setPhysicalModel`,
//! `setSensor`).

use jidr_core::provider::{CallCtx, CommandSpec, parse_params};
use jidr_core::{JidrError, JidrResult};
use jidr_proto::control as pb;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::device::{grpc_err, target};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RotateParams {
    pub device: String,
    /// Device rotation in degrees: 0, 90, 180 or 270 (counter-clockwise).
    pub angle: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Sensor {
    Acceleration,
    Gyroscope,
    MagneticField,
    Temperature,
    Proximity,
    Light,
    Pressure,
    Humidity,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SensorParams {
    pub device: String,
    pub sensor: Sensor,
    /// Values in the sensor's units (3 for vectors, 1 for scalars).
    pub values: Vec<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetSensorParams {
    pub device: String,
    pub sensor: Sensor,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new::<RotateParams>("sensor.rotate", "Rotate the device (0, 90, 180, 270°)"),
        CommandSpec::new::<SensorParams>("sensor.set", "Set a motion or environment sensor value"),
        CommandSpec::new::<GetSensorParams>("sensor.get", "Read a sensor value"),
    ]
}

/// Environment sensors go through the physical model (the emulator derives
/// the sensor from it); motion sensors are overridden directly.
enum Route {
    Physical(pb::physical_model_value::PhysicalType),
    Sensor(pb::sensor_value::SensorType),
}

fn route(s: Sensor) -> Route {
    use pb::physical_model_value::PhysicalType as P;
    use pb::sensor_value::SensorType as S;
    match s {
        Sensor::Acceleration => Route::Sensor(S::Acceleration),
        Sensor::Gyroscope => Route::Sensor(S::Gyroscope),
        Sensor::MagneticField => Route::Physical(P::MagneticField),
        Sensor::Temperature => Route::Physical(P::Temperature),
        Sensor::Proximity => Route::Physical(P::Proximity),
        Sensor::Light => Route::Physical(P::Light),
        Sensor::Pressure => Route::Physical(P::Pressure),
        Sensor::Humidity => Route::Physical(P::Humidity),
    }
}

fn expected_len(s: Sensor) -> usize {
    match s {
        Sensor::Acceleration | Sensor::Gyroscope | Sensor::MagneticField => 3,
        _ => 1,
    }
}

pub fn check_angle(a: u16) -> JidrResult<f32> {
    match a {
        0 | 90 | 180 | 270 => Ok(f32::from(a)),
        _ => Err(JidrError::usage("angle must be 0, 90, 180 or 270")),
    }
}

pub async fn call(ctx: &CallCtx, method: &str, params: Value) -> JidrResult<Value> {
    match method {
        "sensor.rotate" => {
            let p: RotateParams = parse_params(params)?;
            let z = check_angle(p.angle)?;
            let t = target(ctx, &p.device).await?;
            t.grpc()
                .await?
                .set_physical_model(pb::PhysicalModelValue {
                    target: pb::physical_model_value::PhysicalType::Rotation as i32,
                    value: Some(pb::ParameterValue {
                        data: vec![0.0, 0.0, z],
                    }),
                    ..Default::default()
                })
                .await
                .map_err(grpc_err("setPhysicalModel"))?;
            Ok(json!({ "angle": p.angle }))
        }
        "sensor.set" => {
            let p: SensorParams = parse_params(params)?;
            if p.values.len() != expected_len(p.sensor) {
                return Err(JidrError::usage(format!(
                    "{:?} takes {} value(s)",
                    p.sensor,
                    expected_len(p.sensor)
                )));
            }
            let t = target(ctx, &p.device).await?;
            let mut c = t.grpc().await?;
            let value = Some(pb::ParameterValue {
                data: p.values.clone(),
            });
            match route(p.sensor) {
                Route::Physical(target) => c
                    .set_physical_model(pb::PhysicalModelValue {
                        target: target as i32,
                        value,
                        ..Default::default()
                    })
                    .await
                    .map_err(grpc_err("setPhysicalModel"))?,
                Route::Sensor(target) => c
                    .set_sensor(pb::SensorValue {
                        target: target as i32,
                        value,
                        ..Default::default()
                    })
                    .await
                    .map_err(grpc_err("setSensor"))?,
            };
            Ok(json!({ "sensor": p.sensor, "values": p.values }))
        }
        "sensor.get" => {
            let p: GetSensorParams = parse_params(params)?;
            let t = target(ctx, &p.device).await?;
            let mut c = t.grpc().await?;
            let data = match route(p.sensor) {
                Route::Physical(target) => {
                    c.get_physical_model(pb::PhysicalModelValue {
                        target: target as i32,
                        ..Default::default()
                    })
                    .await
                    .map_err(grpc_err("getPhysicalModel"))?
                    .into_inner()
                    .value
                }
                Route::Sensor(target) => {
                    c.get_sensor(pb::SensorValue {
                        target: target as i32,
                        ..Default::default()
                    })
                    .await
                    .map_err(grpc_err("getSensor"))?
                    .into_inner()
                    .value
                }
            };
            Ok(json!({ "sensor": p.sensor, "values": data.map(|d| d.data).unwrap_or_default() }))
        }
        _ => Err(JidrError::usage(format!("unknown method {method}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angles_and_value_counts_are_checked() {
        assert_eq!(check_angle(90).unwrap(), 90.0);
        assert!(check_angle(45).is_err());
        assert_eq!(expected_len(Sensor::Acceleration), 3);
        assert_eq!(expected_len(Sensor::Light), 1);
    }
}
