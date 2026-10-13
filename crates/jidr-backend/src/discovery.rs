//! The emulator's discovery files (`pid_<pid>.ini`): one per running emulator,
//! holding its ports, AVD and gRPC token.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A parsed discovery file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    pub path: PathBuf,
    pub pid: u32,
    pub values: HashMap<String, String>,
}

impl Discovery {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn grpc_token(&self) -> Option<&str> {
        self.get("grpc.token")
    }

    pub fn grpc_port(&self) -> Option<u16> {
        self.get("grpc.port")?.parse().ok()
    }

    pub fn console_port(&self) -> Option<u16> {
        self.get("port.serial")?.parse().ok()
    }

    pub fn adb_port(&self) -> Option<u16> {
        self.get("port.adb")?.parse().ok()
    }

    pub fn avd_id(&self) -> Option<&str> {
        self.get("avd.id")
    }

    pub fn avd_dir(&self) -> Option<PathBuf> {
        self.get("avd.dir").map(PathBuf::from)
    }

    /// The ports the emulator actually bound.
    pub fn ports(&self) -> Option<crate::Ports> {
        let console = self.console_port()?;
        Some(crate::Ports {
            console,
            adb: self.adb_port().unwrap_or(console + 1),
            grpc: self.grpc_port()?,
        })
    }
}

pub fn parse(path: &Path, text: &str) -> Option<Discovery> {
    let pid = path
        .file_name()?
        .to_str()?
        .strip_prefix("pid_")?
        .strip_suffix(".ini")?
        .parse()
        .ok()?;
    let values = text
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    Some(Discovery {
        path: path.to_path_buf(),
        pid,
        values,
    })
}

/// Directories the emulator may write discovery files to, most likely first.
pub fn discovery_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(d) = std::env::var_os("JIDR_DISCOVERY_DIR") {
        dirs.push(PathBuf::from(d));
    }
    if cfg!(target_os = "macos") {
        if let Some(h) = dirs::home_dir() {
            dirs.push(h.join("Library/Caches/TemporaryItems/avd/running"));
        }
    } else {
        if let Some(r) = std::env::var_os("XDG_RUNTIME_DIR") {
            dirs.push(PathBuf::from(r).join("avd/running"));
        }
        if let Ok(user) = std::env::var("USER") {
            dirs.push(PathBuf::from(format!("/tmp/android-{user}/avd/running")));
        }
        dirs.push(std::env::temp_dir().join("avd/running"));
    }
    dirs
}

/// All readable discovery files.
pub fn scan() -> Vec<Discovery> {
    discovery_dirs()
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flat_map(|rd| rd.flatten())
        .filter_map(|e| {
            let p = e.path();
            let text = std::fs::read_to_string(&p).ok()?;
            parse(&p, &text)
        })
        .collect()
}

/// The discovery file of the emulator we launched: same pid, or (if the
/// launcher forked) the gRPC port we reserved for it. The console port is not
/// enough: stale files from crashed or foreign emulators reuse it, and the
/// emulator may not even get the console port it asked for.
pub fn find(pid: u32, grpc: u16) -> Option<Discovery> {
    select(scan(), pid, grpc)
}

fn select(found: Vec<Discovery>, pid: u32, grpc: u16) -> Option<Discovery> {
    found
        .into_iter()
        .find(|d| d.pid == pid || d.grpc_port() == Some(grpc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_discovery_file() {
        let text = "emulator.version=35.6.11.0\nport.serial=5554\nport.adb=5555\navd.id=Pixel\ngrpc.port=8554\ngrpc.token=abc==\navd.dir=/x/Pixel.avd\n";
        let d = parse(Path::new("/r/pid_61264.ini"), text).unwrap();
        assert_eq!(d.pid, 61264);
        assert_eq!(d.console_port(), Some(5554));
        assert_eq!(d.adb_port(), Some(5555));
        assert_eq!(d.grpc_port(), Some(8554));
        assert_eq!(
            d.grpc_token(),
            Some("abc=="),
            "base64 padding survives split_once"
        );
        assert_eq!(d.avd_id(), Some("Pixel"));
        assert!(parse(Path::new("/r/other.ini"), text).is_none());
    }

    /// Regression: a stale discovery file with the same console port (left by
    /// Android Studio or a previous boot) was picked, so we used its gRPC token
    /// and the boot never completed.
    #[test]
    fn stale_file_with_same_console_is_ignored() {
        let stale = parse(
            Path::new("/r/pid_100.ini"),
            "port.serial=5554\ngrpc.port=8554\ngrpc.token=old\n",
        )
        .unwrap();
        let ours = parse(
            Path::new("/r/pid_200.ini"),
            "port.serial=5554\ngrpc.port=9100\ngrpc.token=new\n",
        )
        .unwrap();
        let both = || vec![stale.clone(), ours.clone()];
        assert_eq!(select(both(), 200, 9100).unwrap().grpc_token(), Some("new"));
        // Launcher forked (pid differs): our gRPC port still identifies it.
        assert_eq!(select(both(), 999, 9100).unwrap().grpc_token(), Some("new"));
        assert!(select(vec![stale.clone()], 999, 9100).is_none());
    }

    /// Regression: two emulators launched concurrently by different processes
    /// both asked for console 5554; the loser silently bound 5556 and we
    /// talked to the wrong emulator's console. The discovery file's ports win.
    #[test]
    fn actual_ports_come_from_the_discovery_file() {
        let d = parse(
            Path::new("/r/pid_5.ini"),
            "port.serial=5556\nport.adb=5557\ngrpc.port=9100\n",
        )
        .unwrap();
        assert_eq!(
            d.ports(),
            Some(crate::Ports {
                console: 5556,
                adb: 5557,
                grpc: 9100
            })
        );
        let partial = parse(Path::new("/r/pid_6.ini"), "port.serial=5558\n").unwrap();
        assert_eq!(partial.ports(), None, "no gRPC port, no instance");
    }
}
