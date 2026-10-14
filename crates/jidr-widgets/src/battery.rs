//! Battery widget: level, charger, health (gRPC `setBattery`).

use jidr_core::provider::{CallCtx, CommandSpec, DeviceParams, parse_params};
use jidr_core::{JidrError, JidrResult};
use jidr_proto::control as pb;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::device::{grpc_err, target};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Charger {
    None,
    Ac,
    Usb,
    Wireless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    Good,
    Failed,
    Dead,
    Overvoltage,
    Overheated,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetParams {
    pub device: String,
    /// Charge level, 0–100 %.
    #[serde(default)]
    pub level: Option<u8>,
    /// Shorthand: `true` plugs in AC, `false` unplugs.
    #[serde(default)]
    pub charging: Option<bool>,
    #[serde(default)]
    pub charger: Option<Charger>,
    #[serde(default)]
    pub health: Option<Health>,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new::<DeviceParams>("battery.get", "Battery level, charger and health"),
        CommandSpec::new::<SetParams>("battery.set", "Set battery level, charging and health"),
    ]
}

fn charger_of(c: i32) -> Charger {
    match pb::battery_state::BatteryCharger::try_from(c) {
        Ok(pb::battery_state::BatteryCharger::Ac) => Charger::Ac,
        Ok(pb::battery_state::BatteryCharger::Usb) => Charger::Usb,
        Ok(pb::battery_state::BatteryCharger::Wireless) => Charger::Wireless,
        _ => Charger::None,
    }
}

fn health_of(h: i32) -> Health {
    match pb::battery_state::BatteryHealth::try_from(h) {
        Ok(pb::battery_state::BatteryHealth::Failed) => Health::Failed,
        Ok(pb::battery_state::BatteryHealth::Dead) => Health::Dead,
        Ok(pb::battery_state::BatteryHealth::Overvoltage) => Health::Overvoltage,
        Ok(pb::battery_state::BatteryHealth::Overheated) => Health::Overheated,
        _ => Health::Good,
    }
}

pub fn to_json(b: &pb::BatteryState) -> Value {
    let charger = charger_of(b.charger);
    json!({
        "level": b.charge_level,
        "charging": charger != Charger::None,
        "charger": charger,
        "health": health_of(b.health),
        "present": b.is_present,
    })
}

/// Applies the requested changes to the current state.
pub fn apply(mut b: pb::BatteryState, p: &SetParams) -> JidrResult<pb::BatteryState> {
    use pb::battery_state::{BatteryCharger as C, BatteryHealth as H, BatteryStatus as S};
    if let Some(l) = p.level {
        if l > 100 {
            return Err(JidrError::usage("level must be 0–100"));
        }
        b.charge_level = i32::from(l);
    }
    let charger = match (p.charger, p.charging) {
        (Some(c), _) => Some(c),
        (None, Some(true)) => Some(Charger::Ac),
        (None, Some(false)) => Some(Charger::None),
        (None, None) => None,
    };
    if let Some(c) = charger {
        b.charger = match c {
            Charger::None => C::None,
            Charger::Ac => C::Ac,
            Charger::Usb => C::Usb,
            Charger::Wireless => C::Wireless,
        } as i32;
    }
    if let Some(h) = p.health {
        b.health = match h {
            Health::Good => H::Good,
            Health::Failed => H::Failed,
            Health::Dead => H::Dead,
            Health::Overvoltage => H::Overvoltage,
            Health::Overheated => H::Overheated,
        } as i32;
    }
    b.has_battery = true;
    b.is_present = true;
    b.status = if b.charger == C::None as i32 {
        S::Discharging
    } else if b.charge_level >= 100 {
        S::Full
    } else {
        S::Charging
    } as i32;
    Ok(b)
}

pub async fn call(ctx: &CallCtx, method: &str, params: Value) -> JidrResult<Value> {
    match method {
        "battery.get" => {
            let p: DeviceParams = parse_params(params)?;
            let t = target(ctx, &p.device).await?;
            let b = t
                .grpc()
                .await?
                .get_battery(())
                .await
                .map_err(grpc_err("getBattery"))?;
            Ok(to_json(b.get_ref()))
        }
        "battery.set" => {
            let p: SetParams = parse_params(params)?;
            let t = target(ctx, &p.device).await?;
            let mut c = t.grpc().await?;
            let cur = c
                .get_battery(())
                .await
                .map_err(grpc_err("getBattery"))?
                .into_inner();
            let next = apply(cur, &p)?;
            c.set_battery(next).await.map_err(grpc_err("setBattery"))?;
            Ok(to_json(&next))
        }
        _ => Err(JidrError::usage(format!("unknown method {method}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(level: Option<u8>, charging: Option<bool>) -> SetParams {
        SetParams {
            device: "d".into(),
            level,
            charging,
            charger: None,
            health: None,
        }
    }

    #[test]
    fn level_and_charging_apply() {
        let b = apply(pb::BatteryState::default(), &params(Some(64), Some(false))).unwrap();
        assert_eq!(b.charge_level, 64);
        assert_eq!(
            b.status,
            pb::battery_state::BatteryStatus::Discharging as i32
        );
        let b = apply(b, &params(Some(100), Some(true))).unwrap();
        assert_eq!(b.status, pb::battery_state::BatteryStatus::Full as i32);
        assert_eq!(to_json(&b)["charger"], "ac");
        assert!(apply(b, &params(Some(101), None)).is_err());
    }
}
