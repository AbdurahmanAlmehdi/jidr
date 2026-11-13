//! Packet loss and bad DNS with Linux traffic control (`tc`), on the guest
//! (#17).
//!
//! The emulator kernels from API 31 on are built with `CONFIG_NET_SCH_NETEM`
//! (API 30's isn't). netem shapes what an interface sends, so the loss
//! applies to outgoing packets, and a DNS delay to the queries (the answer
//! comes back that much later).
//!
//! What the helper leaves in the kernel, per interface:
//!
//! - **loss only:** one root netem, `tc qdisc replace dev <if> root netem loss <n>%`;
//! - **bad DNS (with or without loss):** a two-band `prio` root (handle
//!   [`PRIO`]) whose `priomap` sends everything to band 2. `u32` filters send
//!   DNS (UDP and TCP port 53, and DNS over TLS on 853, over IPv4 and IPv6)
//!   to band 1, which has a netem that delays it (`slow`) or drops all of it
//!   (`fail`). Band 2 has a netem with the loss, when there is one;
//! - **nothing:** the kernel's default qdisc. The helper only ever removes a
//!   root netem or its own `prio`, never a qdisc someone else set.
//!
//! By default the helper shapes every interface that carries a default route
//! in any routing table: Android keeps one table per network, so that is
//! Wi-Fi (`wlan0`) and mobile data (`radio0`/`eth0`) on the emulator images,
//! whichever one apps use. `dummy0` is left out: Android gives it a default
//! route in its own table at every boot, but no app traffic uses it.

use crate::rpc::{DnsMode, Error, InterfaceShaping, Shaping, ShapingGet, ShapingSet};

/// Android's own `ip` and `tc` (toybox and iproute2 in `/system/bin`).
pub const IP: &str = "/system/bin/ip";
pub const TC: &str = "/system/bin/tc";

/// The handle of the helper's `prio` root (`1d` for "DNS"); its bands are
/// classes `1d:1` (DNS) and `1d:2` (everything else).
pub const PRIO: &str = "1d:";
const DNS_BAND: &str = "1d:1";
const OTHER_BAND: &str = "1d:2";

/// The DNS ports the filters match: DNS over UDP and TCP, and DNS over TLS
/// (Android's Private DNS).
pub const DNS_PORTS: [u16; 2] = [53, 853];

/// The longest DNS delay. Android's resolver gives up on a query after
/// 5 s and retries; longer delays are only slower failures.
pub const MAX_DNS_DELAY_MS: u32 = 30_000;

/// What a tool printed and how it exited.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Runs a program; real on the guest, fake in tests.
pub trait Run {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<Output>;
}

/// Runs programs with `std::process::Command`.
#[derive(Debug, Default, Clone, Copy)]
pub struct System;

impl Run for System {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<Output> {
        let out = std::process::Command::new(program).args(args).output()?;
        Ok(Output {
            status: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

fn run(r: &dyn Run, program: &str, args: &[&str]) -> Result<Output, Error> {
    r.run(program, args).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::unsupported(format!("{program} isn't on this image"))
        } else {
            Error::failed(format!("running {program}: {e}"))
        }
    })
}

/// An interface name as the kernel allows it (IFNAMSIZ), so it's safe as a
/// `tc` argument.
pub fn check_interface(name: &str) -> Result<&str, Error> {
    let ok = !name.is_empty()
        && name.len() < 16
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if ok {
        Ok(name)
    } else {
        Err(Error::usage(format!(
            "interfaces: {name:?} isn't an interface name"
        )))
    }
}

/// 0–100, as `tc` takes it.
pub fn check_loss(loss_pct: f64) -> Result<f64, Error> {
    if loss_pct.is_finite() && (0.0..=100.0).contains(&loss_pct) {
        Ok(loss_pct)
    } else {
        Err(Error::usage(format!(
            "loss_pct: {loss_pct} isn't between 0 and 100"
        )))
    }
}

/// 1–[`MAX_DNS_DELAY_MS`].
pub fn check_dns_delay(ms: u32) -> Result<u32, Error> {
    if (1..=MAX_DNS_DELAY_MS).contains(&ms) {
        Ok(ms)
    } else {
        Err(Error::usage(format!(
            "dns_delay_ms: {ms} isn't between 1 and {MAX_DNS_DELAY_MS}"
        )))
    }
}

/// The devices of every `default` route in `ip -4 route show table all`,
/// except Android's `dummy*` and loopback.
pub fn parse_default_routes(routes: &str) -> Vec<String> {
    let mut names: Vec<String> = routes
        .lines()
        .filter(|l| l.trim_start().starts_with("default "))
        .filter_map(|l| {
            let mut words = l.split_whitespace();
            words.find(|w| *w == "dev")?;
            words.next().map(str::to_owned)
        })
        .filter(|name| name != "lo" && !name.starts_with("dummy"))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// The word after `key` in a `tc` line.
fn after<'a>(words: &[&'a str], key: &str) -> Option<&'a str> {
    words
        .iter()
        .position(|w| *w == key)
        .and_then(|i| words.get(i + 1).copied())
}

/// A netem's `loss 12.5%`.
fn netem_loss(words: &[&str]) -> f64 {
    after(words, "loss")
        .and_then(|w| w.trim_end_matches('%').parse().ok())
        .unwrap_or(0.0)
}

/// A `tc` time (`3s`, `3.0s`, `250ms`, `250.0ms`, `800us`) in milliseconds.
pub fn parse_time_ms(word: &str) -> Option<u32> {
    let (num, scale) = if let Some(n) = word.strip_suffix("ms") {
        (n, 1.0)
    } else if let Some(n) = word.strip_suffix("us") {
        (n, 0.001)
    } else {
        (word.strip_suffix('s')?, 1000.0)
    };
    let ms = num.parse::<f64>().ok()? * scale;
    (ms.is_finite() && ms >= 0.0).then(|| ms.round() as u32)
}

/// How the shaping of one interface reads in `tc qdisc show dev <iface>`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Parsed {
    /// The root qdisc's kind (`netem`, `prio`, `mq`, `pfifo_fast`…).
    pub qdisc: Option<String>,
    pub loss_pct: f64,
    pub dns: DnsMode,
    pub dns_delay_ms: u32,
    /// The root is the helper's own (a netem, or the `prio` with [`PRIO`]).
    pub ours: bool,
}

/// Reads the helper's shaping back from `tc qdisc show dev <iface>`:
///
/// ```text
/// qdisc prio 1d: root refcnt 2 bands 2 priomap 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1
/// qdisc netem 1d1: parent 1d:1 limit 1000 delay 3s
/// qdisc netem 1d2: parent 1d:2 limit 1000 loss 10%
/// ```
pub fn parse_qdiscs(show: &str) -> Parsed {
    let lines: Vec<Vec<&str>> = show
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .filter(|w| w.first() == Some(&"qdisc") && w.len() >= 3)
        .collect();
    let Some(root) = lines.iter().find(|w| w.contains(&"root")) else {
        return Parsed::default();
    };
    let (kind, handle) = (root[1], root[2]);
    let mut p = Parsed {
        qdisc: Some(kind.to_string()),
        ..Parsed::default()
    };
    match kind {
        "netem" => {
            p.ours = true;
            p.loss_pct = netem_loss(root);
        }
        "prio" if handle == PRIO => {
            p.ours = true;
            let child = |band: &str| {
                lines
                    .iter()
                    .find(|w| w[1] == "netem" && after(w, "parent") == Some(band))
            };
            if let Some(other) = child(OTHER_BAND) {
                p.loss_pct = netem_loss(other);
            }
            if let Some(dns) = child(DNS_BAND) {
                if netem_loss(dns) >= 100.0 {
                    p.dns = DnsMode::Fail;
                } else if let Some(ms) = after(dns, "delay").and_then(parse_time_ms) {
                    p.dns = DnsMode::Slow;
                    p.dns_delay_ms = ms;
                }
            }
        }
        _ => {}
    }
    p
}

/// Whether a delay read back from `tc` is the one that was set: `tc` prints
/// times with two or three significant digits (`1.23s`, older builds
/// `1.2s`).
pub fn delay_matches(read: u32, set: u32) -> bool {
    read.abs_diff(set) <= 50.max(set / 100)
}

/// Whether an interface already has the shaping `want` asks for.
pub fn applied(i: &InterfaceShaping, want: &ShapingSet) -> bool {
    (i.loss_pct - want.loss_pct).abs() <= 0.001
        && i.dns == want.dns
        && (want.dns != DnsMode::Slow || delay_matches(i.dns_delay_ms, want.dns_delay_ms))
}

fn interfaces(r: &dyn Run, given: Option<&[String]>) -> Result<Vec<String>, Error> {
    if let Some(given) = given {
        return given
            .iter()
            .map(|n| check_interface(n).map(str::to_owned))
            .collect();
    }
    let out = run(r, IP, &["-4", "route", "show", "table", "all"])?;
    if out.status != 0 {
        return Err(Error::failed(format!("ip route: {}", out.stderr.trim())));
    }
    let names = parse_default_routes(&out.stdout);
    if names.is_empty() {
        return Err(Error::failed(
            "no interface has a default route (is Wi-Fi or mobile data on?)",
        ));
    }
    Ok(names)
}

fn show(r: &dyn Run, name: &str) -> Result<Parsed, Error> {
    let out = run(r, TC, &["qdisc", "show", "dev", name])?;
    if out.status != 0 {
        return Err(Error::failed(format!(
            "tc qdisc show dev {name}: {}",
            out.stderr.trim()
        )));
    }
    Ok(parse_qdiscs(&out.stdout))
}

fn read(r: &dyn Run, name: &str) -> Result<InterfaceShaping, Error> {
    let p = show(r, name)?;
    Ok(InterfaceShaping {
        name: name.to_string(),
        qdisc: p.qdisc,
        loss_pct: p.loss_pct,
        dns: p.dns,
        dns_delay_ms: p.dns_delay_ms,
    })
}

pub fn get(r: &dyn Run, p: &ShapingGet) -> Result<Shaping, Error> {
    let names = interfaces(r, p.interfaces.as_deref())?;
    Ok(Shaping {
        interfaces: names.iter().map(|n| read(r, n)).collect::<Result<_, _>>()?,
    })
}

/// A kernel without a qdisc or filter kind. Either `tc` itself can't find
/// it, or `tc` knows it but the kernel doesn't and answers ENOENT (API 30's
/// x86_64 kernel for netem: "RTNETLINK answers: No such file or directory").
fn kernel_lacks(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    s.contains("unknown qdisc")
        || s.contains("unknown filter")
        || s.contains("kind is unknown")
        || s.contains("rtnetlink answers: no such file or directory")
}

/// Runs one `tc` change; `feature` names what a kernel that can't do it
/// lacks.
fn tc(r: &dyn Run, args: &[&str], feature: &str) -> Result<(), Error> {
    let out = run(r, TC, args)?;
    if out.status == 0 {
        return Ok(());
    }
    Err(if kernel_lacks(&out.stderr) {
        Error::unsupported(format!("this image's kernel has no {feature}"))
    } else {
        Error::failed(format!("tc {}: {}", args.join(" "), out.stderr.trim()))
    })
}

const NETEM: &str = "netem (CONFIG_NET_SCH_NETEM)";
const PRIO_QDISC: &str = "prio qdisc (CONFIG_NET_SCH_PRIO)";
const U32: &str = "u32 classifier (CONFIG_NET_CLS_U32)";

/// The `prio` tree for bad DNS (see the module docs).
fn dns_tree(r: &dyn Run, name: &str, loss: f64, dns: DnsMode, delay_ms: u32) -> Result<(), Error> {
    // Band 2 (class 1d:2) for everything the filters don't send to band 1.
    let mut prio = vec!["qdisc", "replace", "dev", name, "root", "handle", PRIO];
    prio.extend(["prio", "bands", "2", "priomap"]);
    prio.extend(["1"; 16]);
    tc(r, &prio, PRIO_QDISC)?;

    let loss_arg = format!("{loss}%");
    let delay_arg = format!("{delay_ms}ms");
    let mut band1 = vec!["qdisc", "add", "dev", name, "parent", DNS_BAND];
    band1.extend(["handle", "1d1:", "netem"]);
    match dns {
        DnsMode::Fail => band1.extend(["loss", "100%"]),
        _ => {
            band1.extend(["delay", &delay_arg]);
            if loss > 0.0 {
                band1.extend(["loss", &loss_arg]);
            }
        }
    }
    tc(r, &band1, NETEM)?;
    if loss > 0.0 {
        let band2 = [
            "qdisc", "add", "dev", name, "parent", OTHER_BAND, "handle", "1d2:", "netem", "loss",
            &loss_arg,
        ];
        tc(r, &band2, NETEM)?;
    }

    let mut pref = 0;
    for (protocol, field) in [("ip", "ip"), ("ipv6", "ip6")] {
        for port in DNS_PORTS {
            pref += 1;
            let (pref, port) = (pref.to_string(), port.to_string());
            let filter = [
                "filter", "add", "dev", name, "parent", PRIO, "protocol", protocol, "prio", &pref,
                "u32", "match", field, "dport", &port, "0xffff", "flowid", DNS_BAND,
            ];
            tc(r, &filter, U32)?;
        }
    }
    Ok(())
}

fn shape(r: &dyn Run, name: &str, want: &ShapingSet, loss: f64) -> Result<(), Error> {
    let now = show(r, name)?;
    // Start from the kernel's default when the helper's own tree is there:
    // a `prio` can't turn into a single netem in place, and its children
    // and filters would be added twice.
    if now.ours && (want.dns != DnsMode::Normal || now.qdisc.as_deref() == Some("prio")) {
        tc(r, &["qdisc", "del", "dev", name, "root"], NETEM)?;
    }
    if want.dns != DnsMode::Normal {
        let built = dns_tree(r, name, loss, want.dns, want.dns_delay_ms);
        if built.is_err() {
            // Don't leave half a tree behind; the kernel's default comes back.
            let _ = run(r, TC, &["qdisc", "del", "dev", name, "root"]);
        }
        return built;
    }
    if loss > 0.0 {
        let pct = format!("{loss}%");
        tc(
            r,
            &[
                "qdisc", "replace", "dev", name, "root", "netem", "loss", &pct,
            ],
            NETEM,
        )
    } else if now.ours && now.qdisc.as_deref() == Some("netem") {
        // Only remove our own netem; the kernel puts its default back.
        tc(r, &["qdisc", "del", "dev", name, "root"], NETEM)
    } else {
        Ok(())
    }
}

pub fn set(r: &dyn Run, p: &ShapingSet) -> Result<Shaping, Error> {
    let loss = check_loss(p.loss_pct)?;
    let mut want = p.clone();
    want.dns_delay_ms = match p.dns {
        DnsMode::Slow => check_dns_delay(p.dns_delay_ms)?,
        _ => 0,
    };
    let names = interfaces(r, p.interfaces.as_deref())?;
    for name in &names {
        shape(r, name, &want, loss)?;
    }
    let now = Shaping {
        interfaces: names.iter().map(|n| read(r, n)).collect::<Result<_, _>>()?,
    };
    if let Some(i) = now.interfaces.iter().find(|i| !applied(i, &want)) {
        return Err(Error::failed(format!(
            "{} reports {}% loss and {:?} DNS ({} ms) after setting {loss}% and {:?} ({} ms)",
            i.name, i.loss_pct, i.dns, i.dns_delay_ms, want.dns, want.dns_delay_ms
        )));
    }
    Ok(now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::Kernel;
    use crate::rpc::ErrorCode;
    use std::cell::RefCell;

    /// The fake kernel as a guest the helper runs `ip` and `tc` on.
    struct Guest(RefCell<Kernel>);

    impl Guest {
        fn new(routes: &str) -> Self {
            Self(RefCell::new(Kernel {
                routes: routes.into(),
                ..Kernel::default()
            }))
        }

        fn calls(&self) -> Vec<String> {
            self.0.borrow().calls.clone()
        }
    }

    impl Run for Guest {
        fn run(&self, program: &str, args: &[&str]) -> std::io::Result<Output> {
            Ok(self.0.borrow_mut().run(program, args))
        }
    }

    const ROUTES: &str = "\
default via 10.0.2.2 dev wlan0 table wlan0 proto static
10.0.2.0/24 dev wlan0 table wlan0 proto static scope link
default via 10.0.2.2 dev radio0 table 1022 proto static
local 127.0.0.1 dev lo table local proto kernel scope host src 127.0.0.1
";

    fn loss(loss_pct: f64) -> ShapingSet {
        ShapingSet {
            loss_pct,
            ..ShapingSet::default()
        }
    }

    fn dns(dns: DnsMode, dns_delay_ms: u32, loss_pct: f64) -> ShapingSet {
        ShapingSet {
            interfaces: None,
            loss_pct,
            dns,
            dns_delay_ms,
        }
    }

    #[test]
    fn default_routes_name_the_shaped_interfaces() {
        assert_eq!(parse_default_routes(ROUTES), ["radio0", "wlan0"]);
        assert_eq!(
            parse_default_routes("default dev tun0 scope link"),
            ["tun0"]
        );
        assert!(parse_default_routes("10.0.2.0/24 dev wlan0").is_empty());
        // Regression (API 35 arm64, 2026-11-12): Android gives dummy0 a
        // default route in its own table at every boot; before Wi-Fi is up
        // it was the only interface picked.
        let early = "default dev dummy0 table dummy0 proto static scope link\n";
        assert!(parse_default_routes(early).is_empty());
        assert_eq!(
            parse_default_routes(&format!("{early}{ROUTES}")),
            ["radio0", "wlan0"]
        );
    }

    #[test]
    fn qdiscs_are_read_back() {
        let p = parse_qdiscs("qdisc netem 8001: root refcnt 2 limit 1000 loss 12.5%");
        assert_eq!(
            (p.qdisc.as_deref(), p.loss_pct, p.ours),
            (Some("netem"), 12.5, true)
        );
        let p = parse_qdiscs("qdisc mq 0: root\nqdisc pfifo_fast 0: parent :1 bands 3");
        assert_eq!(
            (p.qdisc.as_deref(), p.loss_pct, p.ours),
            (Some("mq"), 0.0, false)
        );
        assert_eq!(parse_qdiscs(""), Parsed::default());
        let tree = "\
qdisc prio 1d: root refcnt 2 bands 2 priomap 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1
qdisc netem 1d1: parent 1d:1 limit 1000 delay 3.0s loss 10%
qdisc netem 1d2: parent 1d:2 limit 1000 loss 10%
";
        let p = parse_qdiscs(tree);
        assert_eq!(
            (p.loss_pct, p.dns, p.dns_delay_ms, p.ours),
            (10.0, DnsMode::Slow, 3000, true)
        );
        let fail = "qdisc prio 1d: root refcnt 2 bands 2\n\
                    qdisc netem 1d1: parent 1d:1 limit 1000 loss 100%";
        let p = parse_qdiscs(fail);
        assert_eq!((p.loss_pct, p.dns), (0.0, DnsMode::Fail));
        // Someone else's prio isn't ours, and isn't read as bad DNS.
        let p = parse_qdiscs(
            "qdisc prio 1: root refcnt 2 bands 3\n\
                              qdisc netem 10: parent 1:1 limit 1000 loss 100%",
        );
        assert_eq!((p.dns, p.ours), (DnsMode::Normal, false));
    }

    #[test]
    fn times_parse_in_every_tc_spelling() {
        assert_eq!(parse_time_ms("3s"), Some(3000));
        assert_eq!(parse_time_ms("3.0s"), Some(3000));
        assert_eq!(parse_time_ms("1.23s"), Some(1230));
        assert_eq!(parse_time_ms("250ms"), Some(250));
        assert_eq!(parse_time_ms("250.0ms"), Some(250));
        assert_eq!(parse_time_ms("800us"), Some(1));
        assert_eq!(parse_time_ms("3"), None);
        assert!(delay_matches(1230, 1234));
        assert!(delay_matches(12_300, 12_345));
        assert!(!delay_matches(2000, 3000));
    }

    #[test]
    fn set_shapes_every_default_route_and_clear_restores_it() {
        let g = Guest::new(ROUTES);
        let s = set(&g, &loss(10.0)).unwrap();
        assert_eq!(s.interfaces.len(), 2);
        assert!(s.interfaces.iter().all(|i| i.loss_pct == 10.0));
        assert!(
            g.calls().contains(
                &"/system/bin/tc qdisc replace dev wlan0 root netem loss 10%".to_string()
            )
        );
        let s = set(&g, &loss(0.0)).unwrap();
        assert!(s.interfaces.iter().all(|i| i.loss_pct == 0.0));
        assert_eq!(s.interfaces[1].qdisc.as_deref(), Some("mq"));
    }

    #[test]
    fn clearing_leaves_a_qdisc_that_isnt_ours() {
        let g = Guest::new(ROUTES);
        set(&g, &loss(0.0)).unwrap();
        assert!(!g.calls().iter().any(|c| c.contains(" del ")));
        let got = get(&g, &ShapingGet::default()).unwrap();
        assert_eq!(got.interfaces[0].qdisc.as_deref(), Some("pfifo_fast"));
    }

    #[test]
    fn slow_dns_delays_ports_53_and_853_only() {
        let g = Guest::new(ROUTES);
        let s = set(&g, &dns(DnsMode::Slow, 3000, 0.0)).unwrap();
        for i in &s.interfaces {
            assert_eq!(
                (i.dns, i.dns_delay_ms, i.loss_pct),
                (DnsMode::Slow, 3000, 0.0)
            );
            assert_eq!(i.qdisc.as_deref(), Some("prio"));
        }
        let k = g.0.borrow();
        assert_eq!(
            k.qdiscs["wlan0"],
            "qdisc prio 1d: root refcnt 2 bands 2 priomap 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1\n\
             qdisc netem 1d1: parent 1d:1 limit 1000 delay 3s"
        );
        assert_eq!(
            k.filters["wlan0"],
            [
                "protocol ip prio 1 u32 match ip dport 53 0xffff flowid 1d:1",
                "protocol ip prio 2 u32 match ip dport 853 0xffff flowid 1d:1",
                "protocol ipv6 prio 3 u32 match ip6 dport 53 0xffff flowid 1d:1",
                "protocol ipv6 prio 4 u32 match ip6 dport 853 0xffff flowid 1d:1",
            ]
        );
    }

    #[test]
    fn dns_and_loss_combine_and_each_clears_on_its_own() {
        let g = Guest::new(ROUTES);
        // Fail with loss: DNS dropped, the rest loses 10%.
        let s = set(&g, &dns(DnsMode::Fail, 0, 10.0)).unwrap();
        assert!(
            s.interfaces
                .iter()
                .all(|i| i.dns == DnsMode::Fail && i.loss_pct == 10.0)
        );
        assert!(g.0.borrow().qdiscs["radio0"].contains("parent 1d:1 limit 1000 loss 100%"));
        // Changing it again rebuilds the tree: no filter is added twice.
        set(&g, &dns(DnsMode::Slow, 1500, 10.0)).unwrap();
        assert_eq!(g.0.borrow().filters["wlan0"].len(), 4);
        assert!(g.0.borrow().qdiscs["wlan0"].contains("delay 1.5s loss 10%"));
        // DNS back to normal keeps the loss as a plain root netem.
        let s = set(&g, &loss(10.0)).unwrap();
        assert!(
            s.interfaces
                .iter()
                .all(|i| i.dns == DnsMode::Normal && i.loss_pct == 10.0)
        );
        assert_eq!(s.interfaces[0].qdisc.as_deref(), Some("netem"));
        assert!(!g.0.borrow().filters.contains_key("wlan0"));
        // Loss to 0 with slow DNS keeps the DNS band only.
        set(&g, &dns(DnsMode::Slow, 200, 0.0)).unwrap();
        assert!(!g.0.borrow().qdiscs["wlan0"].contains("1d2:"));
        let s = set(&g, &loss(0.0)).unwrap();
        assert!(
            s.interfaces
                .iter()
                .all(|i| i.dns == DnsMode::Normal && i.loss_pct == 0.0)
        );
        assert_eq!(g.0.borrow().qdiscs["wlan0"], "qdisc mq 0: root");
    }

    #[test]
    fn bad_params_and_kernels_without_netem_are_typed_errors() {
        let g = Guest::new(ROUTES);
        let bad = |p: ShapingSet| set(&g, &p).unwrap_err().code;
        let on = |iface: &str, loss_pct| ShapingSet {
            interfaces: Some(vec![iface.into()]),
            loss_pct,
            ..ShapingSet::default()
        };
        assert_eq!(bad(on("wlan0", 101.0)), ErrorCode::Usage);
        assert_eq!(bad(on("wlan0", f64::NAN)), ErrorCode::Usage);
        assert_eq!(bad(on("wlan0; reboot", 5.0)), ErrorCode::Usage);
        assert_eq!(bad(dns(DnsMode::Slow, 0, 0.0)), ErrorCode::Usage);
        assert_eq!(bad(dns(DnsMode::Slow, 30_001, 0.0)), ErrorCode::Usage);
        // Both ways a kernel without netem answers, the second from API 30.
        for stderr in [
            "Unknown qdisc \"netem\", hence option \"loss\" is unparsable",
            "RTNETLINK answers: No such file or directory",
        ] {
            for p in [loss(5.0), dns(DnsMode::Fail, 0, 0.0)] {
                let g = Guest::new(ROUTES);
                g.0.borrow_mut().netem = false;
                g.0.borrow_mut().no_netem_stderr = stderr.into();
                let e = set(&g, &p).unwrap_err();
                assert_eq!(e.code, ErrorCode::Unsupported, "{stderr}");
                assert!(e.message.contains("netem"), "{e}");
                // Half a DNS tree is taken down again.
                assert_eq!(g.0.borrow().qdiscs["wlan0"], "qdisc mq 0: root");
            }
        }
        let none = Guest::new("10.0.2.0/24 dev wlan0");
        assert_eq!(
            get(&none, &ShapingGet::default()).unwrap_err().code,
            ErrorCode::Failed
        );
    }
}
