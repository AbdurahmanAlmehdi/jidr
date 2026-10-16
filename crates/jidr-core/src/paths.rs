//! Where Jidr keeps its files. Everything Jidr writes lives under these
//! directories; the Android Studio SDK and `~/.android/avd` are never written (D3).

use std::path::{Path, PathBuf};

use crate::error::{JidrError, JidrResult};

/// Set to put every Jidr directory under one root (tests, portable installs).
pub const JIDR_HOME_ENV: &str = "JIDR_HOME";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JidrPaths {
    /// `~/Library/Application Support/Jidr`
    pub data: PathBuf,
    /// `~/Library/Caches/Jidr`
    pub cache: PathBuf,
    /// `~/Library/Logs/Jidr`
    pub logs: PathBuf,
}

impl JidrPaths {
    /// Platform defaults, or everything under `$JIDR_HOME` when set.
    pub fn discover() -> JidrResult<Self> {
        if let Some(home) = std::env::var_os(JIDR_HOME_ENV).filter(|h| !h.is_empty()) {
            return Ok(Self::under(PathBuf::from(home)));
        }
        let data = dirs::data_dir()
            .ok_or_else(|| JidrError::unexpected("no data directory for this user"))?
            .join("Jidr");
        let cache = dirs::cache_dir()
            .ok_or_else(|| JidrError::unexpected("no cache directory for this user"))?
            .join("Jidr");
        let logs = if cfg!(target_os = "macos") {
            dirs::home_dir()
                .ok_or_else(|| JidrError::unexpected("no home directory"))?
                .join("Library/Logs/Jidr")
        } else {
            data.join("logs")
        };
        Ok(Self { data, cache, logs })
    }

    /// All directories under one root.
    pub fn under(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            data: root.join("data"),
            cache: root.join("cache"),
            logs: root.join("logs"),
        }
    }

    /// Jidr's private Android SDK.
    pub fn sdk(&self) -> PathBuf {
        self.data.join("sdk")
    }

    /// Jidr's AVD home (`ANDROID_AVD_HOME` for every emulator we launch).
    pub fn avd_home(&self) -> PathBuf {
        self.data.join("avd")
    }

    /// The device registry.
    pub fn devices_file(&self) -> PathBuf {
        self.data.join("devices.json")
    }

    /// Daemon-wide settings (gap B4).
    pub fn settings_file(&self) -> PathBuf {
        self.data.join("settings.json")
    }

    /// License consent records (D8).
    pub fn consent_file(&self) -> PathBuf {
        self.data.join("consent.json")
    }

    /// The daemon's Unix socket.
    pub fn socket(&self) -> PathBuf {
        self.data.join("daemon.sock")
    }

    /// Download cache (SDK archives, Magisk APKs by SHA-256).
    pub fn downloads(&self) -> PathBuf {
        self.cache.join("downloads")
    }

    /// Per-device log directory.
    pub fn device_logs(&self, device: &str) -> PathBuf {
        self.logs.join("devices").join(device)
    }

    /// True if `path` is inside one of Jidr's own directories.
    pub fn owns(&self, path: &Path) -> bool {
        [&self.data, &self.cache, &self.logs]
            .iter()
            .any(|root| path.starts_with(root))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_root_layout() {
        let p = JidrPaths::under("/tmp/j");
        assert_eq!(p.sdk(), PathBuf::from("/tmp/j/data/sdk"));
        assert_eq!(p.avd_home(), PathBuf::from("/tmp/j/data/avd"));
        assert_eq!(p.socket(), PathBuf::from("/tmp/j/data/daemon.sock"));
        assert!(p.owns(Path::new("/tmp/j/cache/downloads/x")));
        assert!(!p.owns(Path::new("/Users/me/Library/Android/sdk")));
    }
}
