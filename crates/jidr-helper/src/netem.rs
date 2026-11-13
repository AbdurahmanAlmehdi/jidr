//! Packet loss with Linux traffic control (`tc … netem`), on the guest (#17).
//!
//! The emulator kernels are built with `CONFIG_NET_SCH_NETEM` (checked on
//! API 31, #17). netem shapes what an interface sends, so the loss applies
//! to outgoing packets; incoming shaping needs an `ifb` redirect (#17).
//!
//! By default the helper shapes every interface that carries a default route
//! in any routing table: Android keeps one table per network, so that is
//! Wi-Fi (`wlan0`) and mobile data (`radio0`/`eth0`) on the emulator images,
//! whichever one apps use. `dummy0` is left out: Android gives it a default
//! route in its own table at every boot, but no app traffic uses it.

use crate::rpc::{Error, InterfaceShaping, Shaping, ShapingGet, ShapingSet};

/// Android's own `ip` and `tc` (toybox and iproute2 in `/system/bin`).
pub const IP: &str = "/system/bin/ip";
pub const TC: &str = "/system/bin/tc";

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

/// The root qdisc of `tc qdisc show dev <iface>`: its kind and netem loss.
///
/// `qdisc netem 8001: root refcnt 2 limit 1000 loss 10%`
pub fn parse_root_qdisc(show: &str) -> (Option<String>, f64) {
    let Some(line) = show
        .lines()
        .find(|l| l.split_whitespace().any(|w| w == "root"))
    else {
        return (None, 0.0);
    };
    let mut words = line.split_whitespace();
    let kind = match (words.next(), words.next()) {
        (Some("qdisc"), Some(kind)) => kind.to_string(),
        _ => return (None, 0.0),
    };
    let loss = if kind == "netem" {
        let mut words = line.split_whitespace();
        words
            .find(|w| *w == "loss")
            .and_then(|_| words.next())
            .and_then(|w| w.trim_end_matches('%').parse().ok())
            .unwrap_or(0.0)
    } else {
        0.0
    };
    (Some(kind), loss)
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

fn read(r: &dyn Run, name: &str) -> Result<InterfaceShaping, Error> {
    let out = run(r, TC, &["qdisc", "show", "dev", name])?;
    if out.status != 0 {
        return Err(Error::failed(format!(
            "tc qdisc show dev {name}: {}",
            out.stderr.trim()
        )));
    }
    let (qdisc, loss_pct) = parse_root_qdisc(&out.stdout);
    Ok(InterfaceShaping {
        name: name.to_string(),
        qdisc,
        loss_pct,
    })
}

pub fn get(r: &dyn Run, p: &ShapingGet) -> Result<Shaping, Error> {
    let names = interfaces(r, p.interfaces.as_deref())?;
    Ok(Shaping {
        interfaces: names.iter().map(|n| read(r, n)).collect::<Result<_, _>>()?,
    })
}

/// A kernel without netem. Either `tc` itself can't find the qdisc kind, or
/// `tc` knows it but the kernel doesn't and answers ENOENT (API 30's x86_64
/// kernel: "RTNETLINK answers: No such file or directory").
fn no_netem(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    s.contains("unknown qdisc")
        || s.contains("specified qdisc kind is unknown")
        || s.contains("rtnetlink answers: no such file or directory")
}

pub fn set(r: &dyn Run, p: &ShapingSet) -> Result<Shaping, Error> {
    let loss = check_loss(p.loss_pct)?;
    let names = interfaces(r, p.interfaces.as_deref())?;
    for name in &names {
        if loss > 0.0 {
            let pct = format!("{loss}%");
            let out = run(
                r,
                TC,
                &[
                    "qdisc", "replace", "dev", name, "root", "netem", "loss", &pct,
                ],
            )?;
            if out.status != 0 {
                return Err(if no_netem(&out.stderr) {
                    Error::unsupported("this image's kernel has no netem (CONFIG_NET_SCH_NETEM)")
                } else {
                    Error::failed(format!(
                        "tc qdisc replace dev {name}: {}",
                        out.stderr.trim()
                    ))
                });
            }
        } else if read(r, name)?.qdisc.as_deref() == Some("netem") {
            // Only remove our own netem; the kernel puts its default back.
            let out = run(r, TC, &["qdisc", "del", "dev", name, "root"])?;
            if out.status != 0 {
                return Err(Error::failed(format!(
                    "tc qdisc del dev {name}: {}",
                    out.stderr.trim()
                )));
            }
        }
    }
    let now = Shaping {
        interfaces: names.iter().map(|n| read(r, n)).collect::<Result<_, _>>()?,
    };
    if let Some(i) = now
        .interfaces
        .iter()
        .find(|i| (i.loss_pct - loss).abs() > 0.001)
    {
        return Err(Error::failed(format!(
            "{} reports {}% loss after setting {loss}%",
            i.name, i.loss_pct
        )));
    }
    Ok(now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// A guest with `tc` and `ip`: root qdiscs per interface.
    struct Guest {
        routes: String,
        qdiscs: RefCell<HashMap<String, String>>,
        netem: bool,
        /// What `tc` prints when the kernel has no netem.
        no_netem_stderr: &'static str,
        calls: RefCell<Vec<String>>,
    }

    impl Guest {
        fn new(routes: &str) -> Self {
            Self {
                routes: routes.into(),
                qdiscs: RefCell::new(HashMap::from([
                    ("wlan0".into(), "qdisc mq 0: root".into()),
                    ("radio0".into(), "qdisc pfifo_fast 0: root refcnt 2".into()),
                ])),
                netem: true,
                no_netem_stderr: "Unknown qdisc \"netem\", hence option \"loss\" is unparsable",
                calls: RefCell::default(),
            }
        }
    }

    impl Run for Guest {
        fn run(&self, program: &str, args: &[&str]) -> std::io::Result<Output> {
            self.calls
                .borrow_mut()
                .push(format!("{program} {}", args.join(" ")));
            let ok = |stdout: &str| Output {
                stdout: stdout.into(),
                ..Output::default()
            };
            Ok(match (program, args) {
                (IP, _) => ok(&self.routes),
                (TC, ["qdisc", "show", "dev", dev]) => {
                    ok(self.qdiscs.borrow().get(*dev).map_or("", String::as_str))
                }
                (TC, ["qdisc", "replace", "dev", dev, "root", "netem", "loss", pct]) => {
                    if !self.netem {
                        return Ok(Output {
                            status: 2,
                            stderr: self.no_netem_stderr.into(),
                            ..Output::default()
                        });
                    }
                    self.qdiscs.borrow_mut().insert(
                        dev.to_string(),
                        format!("qdisc netem 8001: root refcnt 2 limit 1000 loss {pct}"),
                    );
                    ok("")
                }
                (TC, ["qdisc", "del", "dev", dev, "root"]) => {
                    self.qdiscs
                        .borrow_mut()
                        .insert(dev.to_string(), "qdisc mq 0: root".into());
                    ok("")
                }
                _ => Output {
                    status: 1,
                    stderr: "unexpected".into(),
                    ..Output::default()
                },
            })
        }
    }

    const ROUTES: &str = "\
default via 10.0.2.2 dev wlan0 table wlan0 proto static
10.0.2.0/24 dev wlan0 table wlan0 proto static scope link
default via 10.0.2.2 dev radio0 table 1022 proto static
local 127.0.0.1 dev lo table local proto kernel scope host src 127.0.0.1
";

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
    fn root_qdisc_and_loss_are_read_back() {
        assert_eq!(
            parse_root_qdisc("qdisc netem 8001: root refcnt 2 limit 1000 loss 12.5%"),
            (Some("netem".into()), 12.5)
        );
        assert_eq!(
            parse_root_qdisc("qdisc mq 0: root\nqdisc pfifo_fast 0: parent :1 bands 3"),
            (Some("mq".into()), 0.0)
        );
        assert_eq!(parse_root_qdisc(""), (None, 0.0));
    }

    #[test]
    fn set_shapes_every_default_route_and_clear_restores_it() {
        let g = Guest::new(ROUTES);
        let s = set(
            &g,
            &ShapingSet {
                interfaces: None,
                loss_pct: 10.0,
            },
        )
        .unwrap();
        assert_eq!(s.interfaces.len(), 2);
        assert!(s.interfaces.iter().all(|i| i.loss_pct == 10.0));
        assert!(
            g.calls.borrow().contains(
                &"/system/bin/tc qdisc replace dev wlan0 root netem loss 10%".to_string()
            )
        );
        let s = set(
            &g,
            &ShapingSet {
                interfaces: None,
                loss_pct: 0.0,
            },
        )
        .unwrap();
        assert!(s.interfaces.iter().all(|i| i.loss_pct == 0.0));
        assert_eq!(s.interfaces[1].qdisc.as_deref(), Some("mq"));
    }

    #[test]
    fn clearing_leaves_a_qdisc_that_isnt_ours() {
        let g = Guest::new(ROUTES);
        set(
            &g,
            &ShapingSet {
                interfaces: None,
                loss_pct: 0.0,
            },
        )
        .unwrap();
        assert!(!g.calls.borrow().iter().any(|c| c.contains(" del ")));
        let got = get(&g, &ShapingGet::default()).unwrap();
        assert_eq!(got.interfaces[0].qdisc.as_deref(), Some("pfifo_fast"));
    }

    #[test]
    fn bad_params_and_kernels_without_netem_are_typed_errors() {
        let g = Guest::new(ROUTES);
        let bad = |loss_pct, iface: &str| {
            set(
                &g,
                &ShapingSet {
                    interfaces: Some(vec![iface.into()]),
                    loss_pct,
                },
            )
            .unwrap_err()
            .code
        };
        use crate::rpc::ErrorCode;
        assert_eq!(bad(101.0, "wlan0"), ErrorCode::Usage);
        assert_eq!(bad(f64::NAN, "wlan0"), ErrorCode::Usage);
        assert_eq!(bad(5.0, "wlan0; reboot"), ErrorCode::Usage);
        // Both ways a kernel without netem answers, the second from API 30.
        for stderr in [
            "Unknown qdisc \"netem\", hence option \"loss\" is unparsable",
            "RTNETLINK answers: No such file or directory",
        ] {
            let mut g = Guest::new(ROUTES);
            g.netem = false;
            g.no_netem_stderr = stderr;
            let e = set(
                &g,
                &ShapingSet {
                    interfaces: None,
                    loss_pct: 5.0,
                },
            )
            .unwrap_err();
            assert_eq!(e.code, ErrorCode::Unsupported, "{stderr}");
        }
        let none = Guest::new("10.0.2.0/24 dev wlan0");
        assert_eq!(
            get(&none, &ShapingGet::default()).unwrap_err().code,
            ErrorCode::Failed
        );
    }
}
