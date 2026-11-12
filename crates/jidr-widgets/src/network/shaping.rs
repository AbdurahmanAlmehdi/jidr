//! Packet loss on the device's networks (#17), the first widget built on the
//! rooted on-device helper (ADR-0003, `jidr-helper`).
//!
//! The helper runs `tc qdisc … netem loss` on every interface with a default
//! route (Wi-Fi and mobile data), so the loss applies to what the device
//! sends. It needs Shell root (`adb`) or Full root (`magisk`). The setting
//! lives in the guest kernel: a snapshot keeps it, a cold boot drops it.
//! Delay, bandwidth and bad-DNS profiles (#17) join `loss_pct` later as
//! more optional fields.

use jidr_core::provider::CallCtx;
use jidr_core::{JidrError, JidrResult};
use jidr_helper::host::Helper;
use jidr_helper::netem::check_loss;
use jidr_helper::rpc::{Shaping, ShapingGet, ShapingSet};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::device::target;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetParams {
    /// Device name or id (Shell root or Full root).
    pub device: String,
    /// Percent of outgoing packets to drop, 0–100 (decimals allowed); 0
    /// removes the loss.
    pub loss_pct: f64,
}

/// Checks the loss before anything touches the device.
pub fn check(loss_pct: f64) -> JidrResult<f64> {
    check_loss(loss_pct)
        .map_err(|e| JidrError::usage(e.message).with_data(json!({ "field": "loss_pct" })))
}

/// The result both commands return: one `loss_pct` when every interface
/// agrees (null otherwise), and each interface.
pub fn result(s: &Shaping) -> Value {
    let first = s.interfaces.first().map(|i| i.loss_pct);
    let same = first.filter(|l| s.interfaces.iter().all(|i| i.loss_pct == *l));
    json!({ "loss_pct": same, "interfaces": s.interfaces })
}

pub async fn get(ctx: &CallCtx, selector: &str) -> JidrResult<Value> {
    let t = target(ctx, selector).await?;
    let mut helper = Helper::for_device(ctx, &t.device).await?;
    Ok(result(&helper.call(&ShapingGet::default()).await?))
}

pub async fn set(ctx: &CallCtx, p: SetParams) -> JidrResult<Value> {
    let loss_pct = check(p.loss_pct)?;
    let t = target(ctx, &p.device).await?;
    let mut helper = Helper::for_device(ctx, &t.device).await?;
    let s = helper
        .call(&ShapingSet {
            interfaces: None,
            loss_pct,
        })
        .await?;
    Ok(result(&s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jidr_helper::rpc::InterfaceShaping;

    #[test]
    fn loss_is_checked_and_summarised() {
        assert_eq!(check(12.5).unwrap(), 12.5);
        let e = check(-1.0).unwrap_err();
        assert_eq!(e.code, jidr_core::ErrorCode::Usage);
        assert_eq!(e.data.unwrap()["field"], "loss_pct");
        let iface = |name: &str, loss_pct| InterfaceShaping {
            name: name.into(),
            qdisc: Some("netem".into()),
            loss_pct,
        };
        let same = Shaping {
            interfaces: vec![iface("wlan0", 10.0), iface("radio0", 10.0)],
        };
        assert_eq!(result(&same)["loss_pct"], 10.0);
        let mixed = Shaping {
            interfaces: vec![iface("wlan0", 10.0), iface("radio0", 0.0)],
        };
        assert_eq!(result(&mixed)["loss_pct"], Value::Null);
        assert_eq!(result(&mixed)["interfaces"][1]["name"], "radio0");
    }
}
