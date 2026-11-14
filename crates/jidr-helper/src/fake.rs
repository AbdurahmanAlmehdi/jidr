//! A fake guest kernel for offline tests (not used by the product): `ip`
//! prints its routes, and `tc` keeps each interface's qdiscs as the lines
//! `tc qdisc show` prints, for the commands [`netem`](crate::netem) runs.

use std::collections::HashMap;

use crate::netem::{IP, Output, TC};

/// Routes, qdiscs and kernel features of a fake guest.
#[derive(Debug, Clone)]
pub struct Kernel {
    /// `ip -4 route show table all`.
    pub routes: String,
    /// What `tc qdisc show dev <iface>` prints, per interface.
    pub qdiscs: HashMap<String, String>,
    /// `tc filter` rules added, per interface.
    pub filters: HashMap<String, Vec<String>>,
    /// Whether the kernel has netem.
    pub netem: bool,
    /// What `tc` prints when the kernel has no netem.
    pub no_netem_stderr: String,
    /// Every command run, as one line.
    pub calls: Vec<String>,
}

impl Default for Kernel {
    fn default() -> Self {
        Self {
            routes: "default via 10.0.2.2 dev wlan0 table wlan0 proto static\n\
                     default via 10.0.2.2 dev radio0 table 1022 proto static\n"
                .into(),
            qdiscs: HashMap::from([
                ("wlan0".into(), default_qdisc("wlan0")),
                ("radio0".into(), default_qdisc("radio0")),
            ]),
            filters: HashMap::new(),
            netem: true,
            no_netem_stderr: "Unknown qdisc \"netem\", hence option \"loss\" is unparsable".into(),
            calls: Vec::new(),
        }
    }
}

/// The kernel's own root qdisc: `mq` on Wi-Fi, `pfifo_fast` elsewhere.
pub fn default_qdisc(dev: &str) -> String {
    if dev.starts_with("wlan") {
        "qdisc mq 0: root".into()
    } else {
        "qdisc pfifo_fast 0: root refcnt 2 bands 3".into()
    }
}

/// A `tc` time as iproute2 prints it (`%.3g` and a unit).
fn print_ms(ms: f64) -> String {
    if ms >= 1000.0 {
        format!("{}s", three_digits(ms / 1000.0))
    } else {
        format!("{}ms", three_digits(ms))
    }
}

fn three_digits(v: f64) -> String {
    let digits = if v >= 100.0 {
        0
    } else if v >= 10.0 {
        1
    } else {
        2
    };
    let s = format!("{v:.digits$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// The netem parameters as `tc qdisc show` prints them back.
fn print_netem(args: &[&str]) -> String {
    let mut out = vec!["limit 1000".to_string()];
    let mut it = args.iter();
    while let Some(w) = it.next() {
        match (*w, it.next()) {
            ("delay", Some(v)) => {
                let ms: f64 = v.trim_end_matches("ms").parse().unwrap_or(0.0);
                out.push(format!("delay {}", print_ms(ms)));
            }
            ("loss", Some(v)) => out.push(format!("loss {v}")),
            _ => {}
        }
    }
    out.join(" ")
}

fn ok(stdout: impl Into<String>) -> Output {
    Output {
        stdout: stdout.into(),
        ..Output::default()
    }
}

fn err(status: i32, stderr: impl Into<String>) -> Output {
    Output {
        status,
        stderr: stderr.into(),
        ..Output::default()
    }
}

impl Kernel {
    fn root_is_prio(&self, dev: &str, handle: &str) -> bool {
        self.qdiscs
            .get(dev)
            .and_then(|q| q.lines().next())
            .is_some_and(|l| l.starts_with(&format!("qdisc prio {handle} root")))
    }

    fn no_netem(&self) -> Output {
        err(2, self.no_netem_stderr.clone())
    }

    /// Runs `ip` or `tc` against the fake kernel.
    pub fn run(&mut self, program: &str, args: &[&str]) -> Output {
        self.calls.push(format!("{program} {}", args.join(" ")));
        match (program, args) {
            (IP, _) => ok(self.routes.clone()),
            (TC, ["qdisc", "show", "dev", dev]) => {
                ok(self.qdiscs.get(*dev).cloned().unwrap_or_default())
            }
            (TC, ["qdisc", "replace", "dev", dev, "root", "netem", rest @ ..]) => {
                if !self.netem {
                    return self.no_netem();
                }
                let line = format!("qdisc netem 8001: root refcnt 2 {}", print_netem(rest));
                self.qdiscs.insert(dev.to_string(), line);
                self.filters.remove(*dev);
                ok("")
            }
            (
                TC,
                [
                    "qdisc",
                    "replace",
                    "dev",
                    dev,
                    "root",
                    "handle",
                    h,
                    "prio",
                    rest @ ..,
                ],
            ) => {
                let line = format!("qdisc prio {h} root refcnt 2 {}", rest.join(" "));
                self.qdiscs.insert(dev.to_string(), line);
                self.filters.remove(*dev);
                ok("")
            }
            (
                TC,
                [
                    "qdisc",
                    "add",
                    "dev",
                    dev,
                    "parent",
                    parent,
                    "handle",
                    h,
                    "netem",
                    rest @ ..,
                ],
            ) => {
                if !self.netem {
                    return self.no_netem();
                }
                let major = parent.split(':').next().unwrap_or_default();
                if !self.root_is_prio(dev, &format!("{major}:")) {
                    return err(2, "RTNETLINK answers: Invalid argument");
                }
                let line = format!("qdisc netem {h} parent {parent} {}", print_netem(rest));
                if let Some(q) = self.qdiscs.get_mut(*dev) {
                    q.push('\n');
                    q.push_str(&line);
                }
                ok("")
            }
            (TC, ["qdisc", "del", "dev", dev, "root"]) => {
                self.qdiscs.insert(dev.to_string(), default_qdisc(dev));
                self.filters.remove(*dev);
                ok("")
            }
            (TC, ["filter", "add", "dev", dev, "parent", parent, rest @ ..]) => {
                if !self.root_is_prio(dev, parent) {
                    return err(2, "RTNETLINK answers: Invalid argument");
                }
                self.filters
                    .entry(dev.to_string())
                    .or_default()
                    .push(rest.join(" "));
                ok("")
            }
            _ => err(1, format!("fake guest: no {program} {}", args.join(" "))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_print_like_iproute2() {
        assert_eq!(print_ms(3000.0), "3s");
        assert_eq!(print_ms(1234.0), "1.23s");
        assert_eq!(print_ms(12345.0), "12.3s");
        assert_eq!(print_ms(250.0), "250ms");
        assert_eq!(print_ms(7.5), "7.5ms");
    }
}
