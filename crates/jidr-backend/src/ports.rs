//! Port allocation (D9: dynamic, no cap). Console ports are even and the adb
//! port is console + 1. adb auto-discovers consoles 5554–5584; beyond that the
//! device is reached with `adb connect 127.0.0.1:<adb>`.

use std::collections::HashSet;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use jidr_core::{JidrError, JidrResult};

pub const FIRST_CONSOLE: u16 = 5554;
/// Last console port adb scans by itself.
pub const LAST_AUTO_CONSOLE: u16 = 5584;
/// Upper bound of our search (not a device cap: ~220 pairs).
pub const LAST_CONSOLE: u16 = 5998;

/// Ports for one emulator instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ports {
    pub console: u16,
    pub adb: u16,
    pub grpc: u16,
}

impl Ports {
    /// True if adb finds this instance without `adb connect`.
    pub fn auto_discovered(&self) -> bool {
        self.console <= LAST_AUTO_CONSOLE
    }

    /// The adb serial for this instance.
    pub fn adb_serial(&self) -> String {
        if self.auto_discovered() {
            format!("emulator-{}", self.console)
        } else {
            format!("127.0.0.1:{}", self.adb)
        }
    }
}

/// Hands out free ports and remembers them until released, so two
/// concurrent starts in the same daemon never race for a pair.
#[derive(Debug, Clone, Default)]
pub struct PortAllocator {
    reserved: Arc<Mutex<HashSet<u16>>>,
}

fn free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

impl PortAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allocate(&self) -> JidrResult<Ports> {
        self.allocate_avoiding(&HashSet::new())
    }

    /// Like `allocate`, also skipping `in_use` (ports live emulators hold
    /// according to their discovery files: a bind test on 127.0.0.1 can
    /// miss a port bound on another address).
    pub fn allocate_avoiding(&self, in_use: &HashSet<u16>) -> JidrResult<Ports> {
        let mut reserved = self.reserved.lock().unwrap_or_else(|e| e.into_inner());
        let console = (FIRST_CONSOLE..=LAST_CONSOLE)
            .step_by(2)
            .find(|p| {
                ![*p, p + 1]
                    .iter()
                    .any(|q| reserved.contains(q) || in_use.contains(q))
                    && free(*p)
                    && free(p + 1)
            })
            .ok_or_else(|| JidrError::busy("no free emulator console port pair"))?;
        let grpc = (0..20)
            .find_map(|_| {
                let p = TcpListener::bind(("127.0.0.1", 0))
                    .ok()?
                    .local_addr()
                    .ok()?
                    .port();
                (!reserved.contains(&p) && p != console && p != console + 1).then_some(p)
            })
            .ok_or_else(|| JidrError::busy("no free gRPC port"))?;
        reserved.extend([console, console + 1, grpc]);
        Ok(Ports {
            console,
            adb: console + 1,
            grpc,
        })
    }

    /// Reserves ports already used by an adopted instance.
    pub fn claim(&self, ports: Ports) {
        let mut reserved = self.reserved.lock().unwrap_or_else(|e| e.into_inner());
        reserved.extend([ports.console, ports.adb, ports.grpc]);
    }

    pub fn release(&self, ports: Ports) {
        let mut reserved = self.reserved.lock().unwrap_or_else(|e| e.into_inner());
        for p in [ports.console, ports.adb, ports.grpc] {
            reserved.remove(&p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocations_are_distinct_until_released() {
        let a = PortAllocator::new();
        let p1 = a.allocate().unwrap();
        let p2 = a.allocate().unwrap();
        assert_eq!(p1.console % 2, 0);
        assert_eq!(p1.adb, p1.console + 1);
        assert_ne!(p1.console, p2.console);
        assert_ne!(p1.grpc, p2.grpc);
        a.release(p1);
        let p3 = a.allocate().unwrap();
        assert_ne!(
            p3.console, p2.console,
            "still-reserved pair is never handed out"
        );
    }

    #[test]
    fn skips_busy_ports() {
        let a = PortAllocator::new();
        let first = a.allocate().unwrap();
        // Hold the port before releasing the reservation, so other test
        // processes can't race us for it.
        let Ok(_hold) = TcpListener::bind(("127.0.0.1", first.console)) else {
            return;
        };
        a.release(first);
        assert_ne!(a.allocate().unwrap().console, first.console);
    }

    /// Regression: two leaked emulators both asked for console 5554; ports
    /// a live emulator reports in its discovery file are skipped.
    #[test]
    fn skips_ports_live_emulators_hold() {
        let a = PortAllocator::new();
        let first = a.allocate().unwrap();
        a.release(first);
        let held: HashSet<u16> = [first.console].into();
        assert_ne!(a.allocate_avoiding(&held).unwrap().console, first.console);
    }

    #[test]
    fn serials_past_adb_scan_range_use_tcp() {
        let low = Ports {
            console: 5554,
            adb: 5555,
            grpc: 1,
        };
        let high = Ports {
            console: 5586,
            adb: 5587,
            grpc: 1,
        };
        assert_eq!(low.adb_serial(), "emulator-5554");
        assert!(!high.auto_discovered());
        assert_eq!(high.adb_serial(), "127.0.0.1:5587");
    }
}
