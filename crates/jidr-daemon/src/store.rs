//! The device registry (`devices.json`): Jidr's metadata for every AVD it owns.

use std::path::{Path, PathBuf};

use jidr_core::{Device, ErrorCode, JidrError, JidrResult, ResultExt};
use uuid::Uuid;

#[derive(Debug)]
pub struct DeviceStore {
    path: PathBuf,
    devices: Vec<Device>,
}

impl DeviceStore {
    pub fn load(path: impl Into<PathBuf>) -> JidrResult<Self> {
        let path = path.into();
        let devices = match std::fs::read(&path) {
            Ok(b) => serde_json::from_slice(&b)
                .ctx(ErrorCode::Unexpected, format!("reading {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                return Err(JidrError::unexpected(format!(
                    "reading {}: {e}",
                    path.display()
                )));
            }
        };
        Ok(Self { path, devices })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn save(&self) -> JidrResult<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)
                .ctx(ErrorCode::Unexpected, format!("creating {}", dir.display()))?;
        }
        // Runtime fields are not persisted: a restart re-adopts running devices.
        let persisted: Vec<Device> = self
            .devices
            .iter()
            .cloned()
            .map(|mut d| {
                d.runtime = None;
                if d.status.is_active() {
                    d.status = jidr_core::DeviceStatus::Stopped;
                }
                d
            })
            .collect();
        let json =
            serde_json::to_vec_pretty(&persisted).ctx(ErrorCode::Unexpected, "encoding devices")?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json)
            .ctx(ErrorCode::Unexpected, format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path).ctx(
            ErrorCode::Unexpected,
            format!("writing {}", self.path.display()),
        )
    }

    pub fn all(&self) -> &[Device] {
        &self.devices
    }

    /// By id, or by name (case-insensitive).
    pub fn find(&self, selector: &str) -> JidrResult<&Device> {
        let by_id = Uuid::parse_str(selector).ok();
        self.devices
            .iter()
            .find(|d| Some(d.id) == by_id || d.name.eq_ignore_ascii_case(selector))
            .ok_or_else(|| JidrError::not_found(format!("no device named {selector:?}")))
    }

    pub fn get(&self, id: Uuid) -> JidrResult<&Device> {
        self.devices
            .iter()
            .find(|d| d.id == id)
            .ok_or_else(|| JidrError::not_found(format!("no device {id}")))
    }

    pub fn name_taken(&self, name: &str, except: Option<Uuid>) -> bool {
        self.devices
            .iter()
            .any(|d| d.name.eq_ignore_ascii_case(name) && Some(d.id) != except)
    }

    /// A free AVD id derived from a display name: `My Phone` → `My_Phone`.
    pub fn avd_id_for(&self, name: &str, exists: impl Fn(&str) -> bool) -> String {
        let base: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .trim_start_matches('.')
            .chars()
            .take(48)
            .collect();
        let base = if base.is_empty() {
            "device".to_string()
        } else {
            base
        };
        let taken = |id: &str| exists(id) || self.devices.iter().any(|d| d.avd == id);
        if !taken(&base) {
            return base;
        }
        (2..)
            .map(|n| format!("{base}_{n}"))
            .find(|c| !taken(c))
            .unwrap_or(base)
    }

    pub fn insert(&mut self, device: Device) -> JidrResult<()> {
        if self.name_taken(&device.name, None) {
            return Err(JidrError::usage(format!(
                "a device named {:?} already exists",
                device.name
            )));
        }
        self.devices.push(device);
        self.save()
    }

    /// Updates a device in memory and on disk.
    pub fn update(&mut self, id: Uuid, f: impl FnOnce(&mut Device)) -> JidrResult<Device> {
        let d = self
            .devices
            .iter_mut()
            .find(|d| d.id == id)
            .ok_or_else(|| JidrError::not_found(format!("no device {id}")))?;
        f(d);
        let out = d.clone();
        self.save()?;
        Ok(out)
    }

    /// Runtime-only change (status, runtime): not written to disk.
    pub fn set_runtime(&mut self, id: Uuid, f: impl FnOnce(&mut Device)) -> Option<Device> {
        let d = self.devices.iter_mut().find(|d| d.id == id)?;
        f(d);
        Some(d.clone())
    }

    pub fn remove(&mut self, id: Uuid) -> JidrResult<Device> {
        let i = self
            .devices
            .iter()
            .position(|d| d.id == id)
            .ok_or_else(|| JidrError::not_found(format!("no device {id}")))?;
        let d = self.devices.remove(i);
        self.save()?;
        Ok(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jidr_core::{DeviceRuntime, DeviceStatus, ImageRef};

    fn device(name: &str) -> Device {
        Device {
            id: Uuid::new_v4(),
            name: name.into(),
            avd: name.replace(' ', "_"),
            profile: "phone-6.1-fhd".into(),
            image: ImageRef {
                api: 35,
                tag: "google_apis".into(),
                abi: "arm64-v8a".into(),
            },
            sdk_root: "/sdk".into(),
            root: Default::default(),
            agent_owned: false,
            created_at: chrono::Utc::now(),
            launch: Default::default(),
            status: Default::default(),
            runtime: None,
        }
    }

    #[test]
    fn insert_find_update_remove_persist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.json");
        let mut s = DeviceStore::load(&path).unwrap();
        let a = device("My Phone");
        s.insert(a.clone()).unwrap();
        assert_eq!(
            s.insert(device("my phone")).unwrap_err().code,
            ErrorCode::Usage,
            "names are unique, case-insensitive"
        );
        assert_eq!(s.find("MY PHONE").unwrap().id, a.id);
        assert_eq!(s.find(&a.id.to_string()).unwrap().name, "My Phone");
        assert_eq!(s.find("nope").unwrap_err().code, ErrorCode::NotFound);

        s.update(a.id, |d| d.agent_owned = true).unwrap();
        s.set_runtime(a.id, |d| {
            d.status = DeviceStatus::Running;
            d.runtime = Some(DeviceRuntime {
                pid: 1,
                console_port: 5554,
                adb_serial: "emulator-5554".into(),
                grpc_port: 1,
                started_at: chrono::Utc::now(),
            });
        });
        s.update(a.id, |_| {}).unwrap();
        let reloaded = DeviceStore::load(&path).unwrap();
        let d = reloaded.find("My Phone").unwrap();
        assert!(d.agent_owned);
        assert_eq!(
            d.status,
            DeviceStatus::Stopped,
            "runtime state is not persisted"
        );
        assert!(d.runtime.is_none());

        s.remove(a.id).unwrap();
        assert!(DeviceStore::load(&path).unwrap().all().is_empty());
    }

    #[test]
    fn avd_ids_are_sanitized_and_unique() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = DeviceStore::load(dir.path().join("d.json")).unwrap();
        assert_eq!(s.avd_id_for("My Phone 6.1″", |_| false), "My_Phone_6.1_");
        assert_eq!(s.avd_id_for(".hidden", |_| false), "hidden");
        assert_eq!(s.avd_id_for("ü", |_| false), "_");
        s.insert(device("Pixel")).unwrap();
        assert_eq!(s.avd_id_for("Pixel", |_| false), "Pixel_2");
        assert_eq!(s.avd_id_for("Other", |id| id == "Other"), "Other_2");
    }
}
