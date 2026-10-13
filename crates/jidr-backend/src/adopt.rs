//! Re-adopting emulators that kept running across a daemon restart.

use std::path::Path;

use chrono::Utc;

use crate::discovery::{self, Discovery};
use crate::ports::Ports;
use crate::{InstanceInfo, aosp};

/// Turns a discovery file into an `InstanceInfo` if it belongs to an AVD
/// under `avd_home` (Jidr's own; emulators started by Android Studio are left
/// alone).
pub fn instance_from(d: &Discovery, avd_home: &Path) -> Option<InstanceInfo> {
    let avd_dir = d.avd_dir()?;
    let home = std::fs::canonicalize(avd_home).unwrap_or_else(|_| avd_home.to_path_buf());
    let dir = std::fs::canonicalize(&avd_dir).unwrap_or(avd_dir);
    if !dir.starts_with(&home) {
        return None;
    }
    let console = d.console_port()?;
    let ports = Ports {
        console,
        adb: d.adb_port().unwrap_or(console + 1),
        grpc: d.grpc_port()?,
    };
    Some(InstanceInfo {
        avd: d.avd_id()?.to_string(),
        pid: d.pid,
        console_port: ports.console,
        adb_port: ports.adb,
        grpc_port: ports.grpc,
        adb_serial: ports.adb_serial(),
        grpc_token: d.grpc_token()?.to_string(),
        // The discovery file doesn't record the start time.
        started_at: Utc::now(),
        log_path: None,
    })
}

impl aosp::AospBackend {
    /// Finds live emulators running Jidr AVDs and claims their ports.
    pub async fn adopt(&self, avd_home: &Path) -> Vec<InstanceInfo> {
        let mut out = Vec::new();
        for d in discovery::scan() {
            let Some(info) = instance_from(&d, avd_home) else {
                continue;
            };
            if !aosp::process_alive(info.pid).await {
                continue;
            }
            self.ports().claim(info.ports());
            tracing::info!(avd = %info.avd, pid = info.pid, "adopted running emulator");
            out.push(info);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_jidr_avds_are_adopted() {
        let home = tempfile::tempdir().unwrap();
        let ours = home.path().join("phone.avd");
        std::fs::create_dir_all(&ours).unwrap();
        let text = |dir: &Path| {
            format!(
                "port.serial=5590\nport.adb=5591\ngrpc.port=9001\ngrpc.token=tok\navd.id=phone\navd.dir={}\n",
                dir.display()
            )
        };
        let d = discovery::parse(Path::new("/r/pid_77.ini"), &text(&ours)).unwrap();
        let info = instance_from(&d, home.path()).unwrap();
        assert_eq!(info.pid, 77);
        assert_eq!(info.adb_serial, "127.0.0.1:5591");
        assert_eq!(info.grpc_token, "tok");

        let studio = discovery::parse(
            Path::new("/r/pid_78.ini"),
            &text(Path::new("/Users/x/.android/avd/Pixel.avd")),
        )
        .unwrap();
        assert!(instance_from(&studio, home.path()).is_none());
    }
}
