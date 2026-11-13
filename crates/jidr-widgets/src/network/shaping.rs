//! Packet loss and bad DNS on the device's networks (#17), through the
//! rooted on-device helper (ADR-0003, `jidr-helper`).
//!
//! The helper uses Linux traffic control on every interface with a default
//! route (Wi-Fi and mobile data): a netem drops `loss_pct` % of what the
//! device sends, and DNS queries (ports 53 and 853) go through their own
//! netem that delays them (`slow`) or drops them all (`fail`). It needs
//! Shell root (`adb`) or Full root (`magisk`).
//!
//! The setting is kept on the device record ([`NetworkShaping`]) and
//! re-applied by [`on_booted`] after every boot, so it survives cold boots
//! (D44). A quick boot or snapshot that kept it in the guest kernel is left
//! as it is. A device whose root mode became `none` keeps it stored but
//! skips it at boot with a warning; clearing it works without root.

use std::time::{Duration, Instant};

use jidr_core::provider::CallCtx;
use jidr_core::{Device, DnsMode, JidrError, JidrResult, NetworkShaping, RootMode};
use jidr_helper::host::Helper;
use jidr_helper::netem::{applied, check_dns_delay, check_loss};
use jidr_helper::rpc::{self, Shaping, ShapingGet, ShapingSet};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::device::target;

/// The DNS delay when `slow` is asked for without one: Genymotion's "bad
/// DNS" profile.
pub const DEFAULT_DNS_DELAY_MS: u32 = 3000;

/// How long the boot hook waits for Wi-Fi or mobile data to have a route.
const NETWORK_WAIT: Duration = Duration::from_secs(60);
/// How often it looks.
const NETWORK_POLL: Duration = Duration::from_secs(2);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetParams {
    /// Device name or id (Shell root or Full root).
    pub device: String,
    /// Percent of outgoing packets to drop, 0–100 (decimals allowed); 0
    /// removes the loss. Omitted: unchanged.
    #[serde(default)]
    pub loss_pct: Option<f64>,
    /// DNS: `normal`, `slow` (each query delayed by `dns_delay_ms`) or `fail`
    /// (no answers, lookups time out). Omitted: unchanged.
    #[serde(default)]
    pub dns: Option<DnsMode>,
    /// The DNS delay for `slow`, 1–30000 ms (default 3000, or the device's
    /// current one). Given alone, it means `slow`.
    #[serde(default)]
    pub dns_delay_ms: Option<u32>,
}

fn usage(e: rpc::Error, field: &str) -> JidrError {
    JidrError::usage(e.message).with_data(json!({ "field": field }))
}

/// Checks the loss before anything touches the device.
pub fn check(loss_pct: f64) -> JidrResult<f64> {
    check_loss(loss_pct).map_err(|e| usage(e, "loss_pct"))
}

/// The device's new shaping: what `p` gives over what it has now.
pub fn merge(now: &NetworkShaping, p: &SetParams) -> JidrResult<NetworkShaping> {
    if p.loss_pct.is_none() && p.dns.is_none() && p.dns_delay_ms.is_none() {
        return Err(JidrError::usage("give loss_pct, dns or dns_delay_ms"));
    }
    let loss_pct = p.loss_pct.map(check).transpose()?.unwrap_or(now.loss_pct);
    let dns = p
        .dns
        .or(p.dns_delay_ms.map(|_| DnsMode::Slow))
        .unwrap_or(now.dns);
    if p.dns_delay_ms.is_some() && dns != DnsMode::Slow {
        return Err(JidrError::usage("dns_delay_ms only goes with dns: slow")
            .with_data(json!({ "field": "dns_delay_ms" })));
    }
    let dns_delay_ms = match dns {
        DnsMode::Slow => Some(
            check_dns_delay(
                p.dns_delay_ms
                    .or(now.dns_delay_ms)
                    .unwrap_or(DEFAULT_DNS_DELAY_MS),
            )
            .map_err(|e| usage(e, "dns_delay_ms"))?,
        ),
        _ => None,
    };
    Ok(NetworkShaping {
        loss_pct,
        dns,
        dns_delay_ms,
    })
}

fn to_rpc(s: &NetworkShaping) -> ShapingSet {
    ShapingSet {
        interfaces: None,
        loss_pct: s.loss_pct,
        dns: match s.dns {
            DnsMode::Normal => rpc::DnsMode::Normal,
            DnsMode::Slow => rpc::DnsMode::Slow,
            DnsMode::Fail => rpc::DnsMode::Fail,
        },
        dns_delay_ms: s.dns_delay_ms.unwrap_or(0),
    }
}

/// The value every interface agrees on, or null.
fn same<T: PartialEq + serde::Serialize>(
    s: &Shaping,
    f: impl Fn(&rpc::InterfaceShaping) -> T,
) -> Value {
    let mut all = s.interfaces.iter().map(f);
    match all.next() {
        Some(first) if all.all(|v| v == first) => json!(first),
        _ => Value::Null,
    }
}

/// The result both commands return: `loss_pct`, `dns` and `dns_delay_ms`
/// when every interface agrees (null otherwise), and each interface.
pub fn result(s: &Shaping) -> Value {
    let delay = same(s, |i| {
        (i.dns == rpc::DnsMode::Slow).then_some(i.dns_delay_ms)
    });
    json!({
        "loss_pct": same(s, |i| i.loss_pct),
        "dns": same(s, |i| i.dns),
        "dns_delay_ms": delay,
        "interfaces": s.interfaces,
    })
}

pub async fn get(ctx: &CallCtx, selector: &str) -> JidrResult<Value> {
    let t = target(ctx, selector).await?;
    let mut helper = Helper::for_device(ctx, &t.device).await?;
    Ok(result(&helper.call(&ShapingGet::default()).await?))
}

async fn store(ctx: &CallCtx, device: &Device, s: &NetworkShaping) -> JidrResult<()> {
    if device.network_shaping != *s {
        let s = s.clone();
        ctx.daemon
            .update_device(device.id, Box::new(move |d| d.network_shaping = s))
            .await?;
    }
    Ok(())
}

pub async fn set(ctx: &CallCtx, p: SetParams) -> JidrResult<Value> {
    let device = ctx.daemon.device(&p.device).await?;
    let want = merge(&device.network_shaping, &p)?;
    // Without root nothing of Jidr's can be in the guest's kernel (a root
    // change forces a cold boot), so clearing only forgets the stored value.
    if device.root.mode == RootMode::None && want.is_off() {
        store(ctx, &device, &want).await?;
        return Ok(json!({
            "loss_pct": 0.0, "dns": DnsMode::Normal, "dns_delay_ms": null, "interfaces": [],
        }));
    }
    let t = target(ctx, &p.device).await?;
    let mut helper = Helper::for_device(ctx, &t.device).await?;
    let s = helper.call(&to_rpc(&want)).await?;
    store(ctx, &t.device, &want).await?;
    Ok(result(&s))
}

/// The boot hook: re-applies the device's stored shaping (D44). It changes
/// nothing when the kernel already has it (a quick boot, or a snapshot taken
/// with the same shaping).
pub async fn on_booted(ctx: &CallCtx, device: &Device) -> JidrResult<()> {
    if device.network_shaping.is_off() {
        return Ok(());
    }
    if device.root.mode == RootMode::None {
        return Err(JidrError::usage(format!(
            "{0}'s packet loss and bad DNS weren't re-applied: its root mode is none. \
             Root it again (adb or magisk) to bring them back, or forget them with \
             `jidr network loss {0} 0` and `jidr network dns {0} normal`",
            device.name
        )));
    }
    let want = to_rpc(&device.network_shaping);
    ctx.report(None, "Re-applying packet loss and bad DNS");
    let mut helper = Helper::for_device(ctx, device).await?;
    // Wi-Fi and mobile data come up a few seconds after boot, not always
    // together. The helper shapes the interfaces with a default route, so
    // wait until there is one and the list stays the same for one poll.
    let deadline = Instant::now() + NETWORK_WAIT;
    let names = |s: &Shaping| {
        s.interfaces
            .iter()
            .map(|i| i.name.clone())
            .collect::<Vec<_>>()
    };
    let mut seen: Option<Shaping> = None;
    let now = loop {
        match helper.call(&ShapingGet::default()).await {
            Ok(s) if seen.as_ref().is_some_and(|p| names(p) == names(&s)) => break s,
            Ok(s) if Instant::now() >= deadline => break s,
            Ok(s) => seen = Some(s),
            Err(e) if Instant::now() < deadline => {
                tracing::debug!(device = %device.name, "waiting for a network to shape: {e}");
            }
            Err(e) => return Err(e),
        }
        tokio::time::sleep(NETWORK_POLL).await;
    };
    if now.interfaces.iter().all(|i| applied(i, &want)) {
        return Ok(());
    }
    helper.call(&want).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jidr_helper::rpc::InterfaceShaping;

    fn params(loss_pct: Option<f64>, dns: Option<DnsMode>, dns_delay_ms: Option<u32>) -> SetParams {
        SetParams {
            device: "P".into(),
            loss_pct,
            dns,
            dns_delay_ms,
        }
    }

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
            dns: rpc::DnsMode::Normal,
            dns_delay_ms: 0,
        };
        let same = Shaping {
            interfaces: vec![iface("wlan0", 10.0), iface("radio0", 10.0)],
        };
        assert_eq!(result(&same)["loss_pct"], 10.0);
        assert_eq!(result(&same)["dns"], "normal");
        assert_eq!(result(&same)["dns_delay_ms"], Value::Null);
        let mut mixed = Shaping {
            interfaces: vec![iface("wlan0", 10.0), iface("radio0", 0.0)],
        };
        mixed.interfaces[0].dns = rpc::DnsMode::Slow;
        mixed.interfaces[0].dns_delay_ms = 3000;
        assert_eq!(result(&mixed)["loss_pct"], Value::Null);
        assert_eq!(result(&mixed)["dns"], Value::Null);
        assert_eq!(result(&mixed)["interfaces"][1]["name"], "radio0");
    }

    #[test]
    fn a_set_changes_only_what_it_gives() {
        let none = NetworkShaping::default();
        let e = merge(&none, &params(None, None, None)).unwrap_err();
        assert_eq!(e.code, jidr_core::ErrorCode::Usage);
        let loss = merge(&none, &params(Some(10.0), None, None)).unwrap();
        assert_eq!((loss.loss_pct, loss.dns), (10.0, DnsMode::Normal));
        // DNS keeps the loss; slow defaults to 3000 ms.
        let slow = merge(&loss, &params(None, Some(DnsMode::Slow), None)).unwrap();
        assert_eq!(
            (slow.loss_pct, slow.dns, slow.dns_delay_ms),
            (10.0, DnsMode::Slow, Some(3000))
        );
        // A delay alone means slow, and is kept by a later loss change.
        let d = merge(&none, &params(None, None, Some(800))).unwrap();
        assert_eq!((d.dns, d.dns_delay_ms), (DnsMode::Slow, Some(800)));
        let d = merge(&d, &params(Some(5.0), None, None)).unwrap();
        assert_eq!((d.loss_pct, d.dns_delay_ms), (5.0, Some(800)));
        // Fail and normal carry no delay.
        let f = merge(&d, &params(None, Some(DnsMode::Fail), None)).unwrap();
        assert_eq!((f.dns, f.dns_delay_ms), (DnsMode::Fail, None));
        let off = merge(&f, &params(Some(0.0), Some(DnsMode::Normal), None)).unwrap();
        assert!(off.is_off());
        for (p, field) in [
            (params(None, Some(DnsMode::Fail), Some(100)), "dns_delay_ms"),
            (params(None, None, Some(0)), "dns_delay_ms"),
            (params(None, None, Some(30_001)), "dns_delay_ms"),
            (params(Some(100.5), None, None), "loss_pct"),
        ] {
            let e = merge(&none, &p).unwrap_err();
            assert_eq!(e.data.unwrap()["field"], field, "{p:?}");
        }
    }
}
