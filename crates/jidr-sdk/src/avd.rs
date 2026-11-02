//! AVD lifecycle in Jidr's own AVD home (never `~/.android/avd`): create from a
//! profile + image, edit `config.ini`, clone, delete, factory reset.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use jidr_core::{ErrorCode, HardwareProfile, ImageRef, JidrError, JidrResult, ResultExt};
use serde::{Deserialize, Serialize};

/// `config.ini` with order, comments and `key=value` / `key = value`
/// formatting preserved, so a read → write round trip is byte-identical.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigIni {
    lines: Vec<Line>,
    trailing_newline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    Entry {
        key: String,
        sep: String,
        value: String,
    },
    Other(String),
}

impl ConfigIni {
    pub fn parse(text: &str) -> Self {
        let lines = text
            .lines()
            .map(|l| match l.split_once('=') {
                Some((k, v)) if !l.trim_start().starts_with('#') && !k.trim().is_empty() => {
                    let key = k.trim_end();
                    let ws_before = &k[key.len()..];
                    let value = v.trim_start();
                    let ws_after = &v[..v.len() - value.len()];
                    Line::Entry {
                        key: key.to_string(),
                        sep: format!("{ws_before}={ws_after}"),
                        value: value.to_string(),
                    }
                }
                _ => Line::Other(l.to_string()),
            })
            .collect();
        Self {
            lines,
            trailing_newline: text.ends_with('\n') || text.is_empty(),
        }
    }

    pub fn read(path: &Path) -> JidrResult<Self> {
        let text = std::fs::read_to_string(path)
            .ctx(ErrorCode::Sdk, format!("reading {}", path.display()))?;
        Ok(Self::parse(&text))
    }

    pub fn write(&self, path: &Path) -> JidrResult<()> {
        let tmp = path.with_extension("ini.tmp");
        std::fs::write(&tmp, self.to_string())
            .ctx(ErrorCode::Sdk, format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path).ctx(ErrorCode::Sdk, format!("writing {}", path.display()))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.lines.iter().rev().find_map(|l| match l {
            Line::Entry { key: k, value, .. } if k == key => Some(value.as_str()),
            _ => None,
        })
    }

    /// Updates the last occurrence of `key`, or appends it.
    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        for l in self.lines.iter_mut().rev() {
            if let Line::Entry {
                key: k, value: v, ..
            } = l
                && k == key
            {
                *v = value;
                return;
            }
        }
        let sep = self
            .lines
            .iter()
            .find_map(|l| match l {
                Line::Entry { sep, .. } => Some(sep.clone()),
                Line::Other(_) => None,
            })
            .unwrap_or_else(|| "=".into());
        self.lines.push(Line::Entry {
            key: key.to_string(),
            sep,
            value,
        });
    }

    pub fn remove(&mut self, key: &str) {
        self.lines
            .retain(|l| !matches!(l, Line::Entry { key: k, .. } if k == key));
    }

    pub fn entries(&self) -> BTreeMap<String, String> {
        self.lines
            .iter()
            .filter_map(|l| match l {
                Line::Entry { key, value, .. } => Some((key.clone(), value.clone())),
                Line::Other(_) => None,
            })
            .collect()
    }
}

impl std::fmt::Display for ConfigIni {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, l) in self.lines.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            match l {
                Line::Entry { key, sep, value } => write!(f, "{key}{sep}{value}")?,
                Line::Other(s) => f.write_str(s)?,
            }
        }
        if self.trailing_newline && !self.lines.is_empty() {
            f.write_str("\n")?;
        }
        Ok(())
    }
}

/// What to create.
#[derive(Debug, Clone)]
pub struct AvdSpec {
    /// AVD id: `[A-Za-z0-9._-]`, ≤ 64 chars.
    pub name: String,
    pub display_name: String,
    pub profile: HardwareProfile,
    pub image: ImageRef,
    /// Extra or overriding `config.ini` keys.
    pub overrides: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AvdInfo {
    pub name: String,
    pub dir: PathBuf,
    pub display_name: Option<String>,
    pub image: Option<ImageRef>,
}

/// Files and directories that belong to a running or previous emulator
/// session and must not be cloned.
const RUNTIME_ENTRIES: &[&str] = &[
    "hardware-qemu.ini",
    "hardware-qemu.ini.lock",
    "multiinstance.lock",
    "bootcompleted.ini",
    "emu-launch-params.txt",
    "read-snapshot.txt",
    "snapshots",
    "tmpAdbCmds",
    "emulator-user.ini",
    "AVD.conf",
];

/// Files removed by a factory reset; the emulator recreates them from the
/// image on the next boot (same effect as `-wipe-data`).
const USER_DATA_ENTRIES: &[&str] = &[
    "userdata-qemu.img",
    "userdata-qemu.img.qcow2",
    "cache.img",
    "cache.img.qcow2",
    "encryptionkey.img",
    "encryptionkey.img.qcow2",
    "snapshots",
    "data",
    "modem_simulator",
    "bootcompleted.ini",
];

pub fn validate_name(name: &str) -> JidrResult<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(JidrError::usage(format!(
            "invalid AVD name {name:?}: use 1–64 of A-Z a-z 0-9 . _ -, not starting with a dot"
        )))
    }
}

/// Manages AVDs under one AVD home.
#[derive(Debug, Clone)]
pub struct AvdManager {
    home: PathBuf,
}

impl AvdManager {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn dir(&self, name: &str) -> PathBuf {
        self.home.join(format!("{name}.avd"))
    }

    fn ini(&self, name: &str) -> PathBuf {
        self.home.join(format!("{name}.ini"))
    }

    pub fn exists(&self, name: &str) -> bool {
        self.dir(name).join("config.ini").is_file()
    }

    fn require(&self, name: &str) -> JidrResult<PathBuf> {
        validate_name(name)?;
        if self.exists(name) {
            Ok(self.dir(name))
        } else {
            Err(JidrError::not_found(format!("AVD {name} does not exist")))
        }
    }

    /// The `config.ini` Jidr writes for a profile + image.
    pub fn config_for(spec: &AvdSpec) -> ConfigIni {
        let p = &spec.profile;
        let i = &spec.image;
        let cpu_arch = if i.abi.starts_with("arm64") {
            "arm64"
        } else {
            "x86_64"
        };
        let heap = if p.ram_mb >= 4096 { 512 } else { 256 };
        let mut c = ConfigIni::default();
        let entries: [(&str, String); 26] = [
            ("AvdId", spec.name.clone()),
            ("avd.ini.displayname", spec.display_name.clone()),
            ("avd.ini.encoding", "UTF-8".into()),
            ("abi.type", i.abi.clone()),
            ("hw.cpu.arch", cpu_arch.into()),
            ("hw.cpu.ncore", p.cpus.to_string()),
            ("hw.ramSize", p.ram_mb.to_string()),
            ("vm.heapSize", heap.to_string()),
            ("hw.lcd.width", p.width.to_string()),
            ("hw.lcd.height", p.height.to_string()),
            ("hw.lcd.density", p.density.to_string()),
            ("hw.gpu.enabled", "yes".into()),
            ("hw.gpu.mode", "auto".into()),
            ("hw.keyboard", "yes".into()),
            ("hw.mainKeys", "no".into()),
            ("hw.initialOrientation", "portrait".into()),
            ("hw.sdCard", "no".into()),
            ("image.sysdir.1", i.sysdir()),
            ("tag.id", i.tag.clone()),
            ("target", format!("android-{}", i.api)),
            ("PlayStore.enabled", i.tag.contains("playstore").to_string()),
            ("disk.dataPartition.size", format!("{}M", p.data_mb)),
            ("fastboot.forceColdBoot", "no".into()),
            ("showDeviceFrame", "no".into()),
            ("runtime.network.latency", "none".into()),
            ("runtime.network.speed", "full".into()),
        ];
        for (k, v) in entries {
            c.set(k, v);
        }
        for (k, v) in p.config.iter().chain(&spec.overrides) {
            c.set(k, v.clone());
        }
        c
    }

    pub fn create(&self, spec: &AvdSpec) -> JidrResult<AvdInfo> {
        validate_name(&spec.name)?;
        if self.exists(&spec.name) {
            return Err(JidrError::usage(format!(
                "AVD {} already exists",
                spec.name
            )));
        }
        let dir = self.dir(&spec.name);
        std::fs::create_dir_all(&dir).ctx(ErrorCode::Sdk, format!("creating {}", dir.display()))?;
        Self::config_for(spec).write(&dir.join("config.ini"))?;
        self.write_pointer(&spec.name, spec.image.api)?;
        self.info(&spec.name)
    }

    /// `<home>/<name>.ini`, which the emulator uses to find the AVD dir.
    fn write_pointer(&self, name: &str, api: u32) -> JidrResult<()> {
        let mut ini = ConfigIni::default();
        ini.set("avd.ini.encoding", "UTF-8");
        ini.set("path", self.dir(name).to_string_lossy());
        ini.set("path.rel", format!("avd/{name}.avd"));
        ini.set("target", format!("android-{api}"));
        ini.write(&self.ini(name))
    }

    pub fn config(&self, name: &str) -> JidrResult<ConfigIni> {
        ConfigIni::read(&self.require(name)?.join("config.ini"))
    }

    /// Applies `changes` (value `None` removes the key).
    pub fn edit(
        &self,
        name: &str,
        changes: &BTreeMap<String, Option<String>>,
    ) -> JidrResult<ConfigIni> {
        self.ensure_stopped(name)?;
        let path = self.require(name)?.join("config.ini");
        let mut c = ConfigIni::read(&path)?;
        for (k, v) in changes {
            match v {
                Some(v) => c.set(k, v.clone()),
                None => c.remove(k),
            }
        }
        c.write(&path)?;
        Ok(c)
    }

    pub fn info(&self, name: &str) -> JidrResult<AvdInfo> {
        let dir = self.require(name)?;
        let c = ConfigIni::read(&dir.join("config.ini"))?;
        let image = c.get("image.sysdir.1").and_then(|s| {
            let parts: Vec<&str> = s.trim_end_matches('/').split('/').collect();
            match parts.as_slice() {
                ["system-images", api, tag, abi] => Some(ImageRef {
                    api: api.strip_prefix("android-")?.parse().ok()?,
                    tag: tag.to_string(),
                    abi: abi.to_string(),
                }),
                _ => None,
            }
        });
        Ok(AvdInfo {
            name: name.to_string(),
            display_name: c.get("avd.ini.displayname").map(str::to_string),
            image,
            dir,
        })
    }

    pub fn list(&self) -> JidrResult<Vec<AvdInfo>> {
        let Ok(rd) = std::fs::read_dir(&self.home) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for e in rd.flatten() {
            let file = e.file_name().to_string_lossy().into_owned();
            if let Some(name) = file.strip_suffix(".avd")
                && validate_name(name).is_ok()
                && self.exists(name)
            {
                out.push(self.info(name)?);
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Turns the emulator's own quick-boot save on exit on or off in
    /// `<avd>/AVD.conf` (`[perAvd] set\saveSnapshotOnExit`, 0 = save). The
    /// headless emulator ignores the exit save without it ("Snapshots have
    /// been disabled by the user"), which left Jidr devices with an empty
    /// `default_boot` (Phase 13c). Other keys in the file are kept.
    pub fn set_save_on_exit(&self, name: &str, save: bool) -> JidrResult<()> {
        let path = self.require(name)?.join("AVD.conf");
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let next = with_save_on_exit(&text, save);
        if next != text {
            std::fs::write(&path, next)
                .ctx(ErrorCode::Sdk, format!("writing {}", path.display()))?;
        }
        Ok(())
    }

    /// True if an emulator process currently holds this AVD.
    pub fn is_running(&self, name: &str) -> bool {
        is_locked(&self.dir(name))
    }

    fn ensure_stopped(&self, name: &str) -> JidrResult<()> {
        if self.is_running(name) {
            return Err(JidrError::busy(format!(
                "AVD {name} is running; stop it first"
            )));
        }
        Ok(())
    }

    /// Copies an AVD, minus runtime state and snapshots (the clone cold-boots).
    /// The qcow2 overlays reference their backing files by relative name, so
    /// they stay valid in the new directory.
    pub fn clone_avd(&self, src: &str, dst: &str, display_name: &str) -> JidrResult<AvdInfo> {
        let from = self.require(src)?;
        validate_name(dst)?;
        self.ensure_stopped(src)?;
        if self.exists(dst) {
            return Err(JidrError::usage(format!("AVD {dst} already exists")));
        }
        let to = self.dir(dst);
        let result = copy_tree(&from, &to, RUNTIME_ENTRIES).and_then(|()| {
            let mut c = ConfigIni::read(&to.join("config.ini"))?;
            c.set("AvdId", dst);
            c.set("avd.ini.displayname", display_name);
            c.write(&to.join("config.ini"))?;
            let api = c
                .get("target")
                .and_then(|t| t.strip_prefix("android-"))
                .and_then(|a| a.parse().ok())
                .unwrap_or(0);
            self.write_pointer(dst, api)?;
            self.info(dst)
        });
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&to);
            let _ = std::fs::remove_file(self.ini(dst));
        }
        result
    }

    /// Copies an AVD with one of its snapshots (D42, Phase 13b): a
    /// copy-on-write clone ([`crate::cow::cow_copy`]) that keeps snapshot
    /// `snapshot`, drops the quick-boot one, the others and every lock, and
    /// rewrites the AVD's name and folder in `<dst>.ini`, `config.ini`,
    /// `hardware-qemu.ini` and the snapshot's `hardware.ini`. Launch it with
    /// `-snapshot <snapshot>` to resume where the source was saved.
    pub fn clone_with_snapshot(
        &self,
        src: &str,
        dst: &str,
        display_name: &str,
        snapshot: &str,
    ) -> JidrResult<crate::cow::CopyMethod> {
        let from = self.require(src)?;
        validate_name(dst)?;
        self.ensure_stopped(src)?;
        if self.exists(dst) {
            return Err(JidrError::usage(format!("AVD {dst} already exists")));
        }
        check_snapshot(&from, snapshot)?;
        let to = self.dir(dst);
        let result = crate::cow::cow_copy(&from, &to).and_then(|method| {
            prune_clone(&to, snapshot)?;
            let mut c = ConfigIni::read(&to.join("config.ini"))?;
            c.set("AvdId", dst);
            c.set("avd.ini.displayname", display_name);
            c.write(&to.join("config.ini"))?;
            for ini in [
                to.join("hardware-qemu.ini"),
                to.join("snapshots").join(snapshot).join("hardware.ini"),
            ] {
                if ini.is_file() {
                    let text = std::fs::read_to_string(&ini)
                        .ctx(ErrorCode::Sdk, format!("reading {}", ini.display()))?;
                    std::fs::write(&ini, rewrite_avd_refs(&text, src, dst))
                        .ctx(ErrorCode::Sdk, format!("writing {}", ini.display()))?;
                }
            }
            let api = c
                .get("target")
                .and_then(|t| t.strip_prefix("android-"))
                .and_then(|a| a.parse().ok())
                .unwrap_or(0);
            self.write_pointer(dst, api)?;
            Ok(method)
        });
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&to);
            let _ = std::fs::remove_file(self.ini(dst));
        }
        result
    }

    pub fn delete(&self, name: &str) -> JidrResult<()> {
        let dir = self.require(name)?;
        self.ensure_stopped(name)?;
        std::fs::remove_dir_all(&dir).ctx(ErrorCode::Sdk, format!("deleting {}", dir.display()))?;
        match std::fs::remove_file(self.ini(name)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(JidrError::sdk(format!("deleting {}.ini: {e}", name)))
            }
            _ => Ok(()),
        }
    }

    /// Wipes user data; the next boot is a fresh first boot.
    pub fn factory_reset(&self, name: &str) -> JidrResult<()> {
        let dir = self.require(name)?;
        self.ensure_stopped(name)?;
        for entry in USER_DATA_ENTRIES {
            let p = dir.join(entry);
            let res = if p.is_dir() {
                std::fs::remove_dir_all(&p)
            } else {
                std::fs::remove_file(&p)
            };
            match res {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(JidrError::sdk(format!("removing {}: {e}", p.display())));
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Bytes on disk used by the AVD (allocated blocks; images are sparse).
    pub fn disk_usage(&self, name: &str) -> JidrResult<u64> {
        Ok(disk_usage(&self.require(name)?))
    }
}

const SAVE_ON_EXIT_KEY: &str = "set\\saveSnapshotOnExit";

/// `AVD.conf` (a Qt settings file) with `saveSnapshotOnExit` set in its
/// `[perAvd]` section: `0` saves on exit, `1` doesn't.
pub fn with_save_on_exit(text: &str, save: bool) -> String {
    let line = format!("{SAVE_ON_EXIT_KEY}={}", if save { 0 } else { 1 });
    let mut out: Vec<String> = Vec::new();
    let (mut in_section, mut seen_section, mut done) = (false, false, false);
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            if in_section && !done {
                out.push(line.clone());
                done = true;
            }
            in_section = t == "[perAvd]";
            seen_section |= in_section;
        } else if in_section && t.split('=').next().map(str::trim) == Some(SAVE_ON_EXIT_KEY) {
            if !done {
                out.push(line.clone());
                done = true;
            }
            continue;
        }
        out.push(l.to_string());
    }
    if !done {
        if !seen_section {
            out.push("[perAvd]".into());
        }
        out.push(line);
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// A snapshot a clone can resume from: `snapshots/<name>/` with a
/// `snapshot.pb` and a non-empty `ram.bin` (an empty one can't be loaded).
pub fn check_snapshot(avd_dir: &Path, name: &str) -> JidrResult<()> {
    let plain = !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control);
    if !plain {
        return Err(JidrError::usage(format!("not a snapshot: {name:?}")));
    }
    let dir = avd_dir.join("snapshots").join(name);
    if !dir.join("snapshot.pb").is_file() {
        return Err(JidrError::not_found(format!("no snapshot {name:?}")));
    }
    let ram = std::fs::metadata(dir.join("ram.bin"))
        .map(|m| m.len())
        .unwrap_or(0);
    if ram == 0 {
        return Err(JidrError::sdk(format!(
            "snapshot {name:?} has no saved memory (empty ram.bin); save it again while the device runs"
        )));
    }
    Ok(())
}

/// In a fresh clone: every lock, the per-session files and every snapshot
/// but `keep` go; `jidr/snapshots.json` keeps only `keep`'s tag (D29: the
/// root-mode tag travels with the snapshot).
fn prune_clone(dir: &Path, keep: &str) -> JidrResult<()> {
    remove_locks(dir)?;
    for entry in RUNTIME_ENTRIES {
        if matches!(*entry, "hardware-qemu.ini" | "snapshots") {
            continue;
        }
        remove_any(&dir.join(entry))?;
    }
    let snaps = dir.join("snapshots");
    for e in
        std::fs::read_dir(&snaps).ctx(ErrorCode::Sdk, format!("reading {}", snaps.display()))?
    {
        let e = e.ctx(ErrorCode::Sdk, "reading directory entry")?;
        if e.file_name() != keep {
            remove_any(&e.path())?;
        }
    }
    let tags = dir.join("jidr").join("snapshots.json");
    if let Ok(text) = std::fs::read_to_string(&tags)
        && let Ok(serde_json::Value::Object(mut map)) = serde_json::from_str(&text)
    {
        map.retain(|id, _| id == keep);
        let text = serde_json::to_string_pretty(&map).unwrap_or_else(|_| "{}".into());
        std::fs::write(&tags, text).ctx(ErrorCode::Sdk, format!("writing {}", tags.display()))?;
    }
    Ok(())
}

fn remove_locks(dir: &Path) -> JidrResult<()> {
    for e in std::fs::read_dir(dir).ctx(ErrorCode::Sdk, format!("reading {}", dir.display()))? {
        let e = e.ctx(ErrorCode::Sdk, "reading directory entry")?;
        let path = e.path();
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            remove_locks(&path)?;
        } else if e.file_name().to_string_lossy().ends_with(".lock") {
            remove_any(&path)?;
        }
    }
    Ok(())
}

fn remove_any(path: &Path) -> JidrResult<()> {
    let res = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match res {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(JidrError::sdk(format!("removing {}: {e}", path.display())))
        }
        _ => Ok(()),
    }
}

/// Rewrites an emulator ini (`hardware-qemu.ini`, a snapshot's
/// `hardware.ini`) for AVD `dst`: `avd.name` / `avd.id`, and every path
/// segment `<src>.avd` (the emulator writes paths like
/// `<home>/../avd/<src>.avd/userdata-qemu.img`).
pub fn rewrite_avd_refs(text: &str, src: &str, dst: &str) -> String {
    let (from, to) = (format!("/{src}.avd"), format!("/{dst}.avd"));
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (body, end) = match line.strip_suffix('\n') {
            Some(b) => (b, "\n"),
            None => (line, ""),
        };
        let rewritten = match body.split_once('=') {
            Some((k, v)) if matches!(k.trim(), "avd.name" | "avd.id") && v.trim() == src => {
                format!("{}= {dst}", k)
            }
            Some((k, v)) => {
                let mut v = v.replace(&format!("{from}/"), &format!("{to}/"));
                if v.trim_end().ends_with(&from) {
                    let cut = v.trim_end().len() - from.len();
                    v = format!("{}{to}", &v[..cut]);
                }
                format!("{k}={v}")
            }
            None => body.to_string(),
        };
        out.push_str(&rewritten);
        out.push_str(end);
    }
    out
}

/// The emulator holds `hardware-qemu.ini.lock` (containing its pid) while
/// running. The file stays behind after exit, so the pid must be alive.
fn is_locked(dir: &Path) -> bool {
    let Ok(bytes) = std::fs::read(dir.join("hardware-qemu.ini.lock")) else {
        return false;
    };
    match lock_pid(&bytes) {
        Some(pid) => process_alive(pid),
        // An empty or garbled lock can't name a live process.
        None => false,
    }
}

/// The pid in a lock file: leading ASCII digits (the emulator pads the file
/// with NUL bytes).
fn lock_pid(bytes: &[u8]) -> Option<u32> {
    let digits: String = bytes
        .iter()
        .skip_while(|b| b.is_ascii_whitespace())
        .take_while(|b| b.is_ascii_digit())
        .map(|b| *b as char)
        .collect();
    digits.parse().ok()
}

pub fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn copy_tree(from: &Path, to: &Path, skip: &[&str]) -> JidrResult<()> {
    std::fs::create_dir_all(to).ctx(ErrorCode::Sdk, format!("creating {}", to.display()))?;
    for e in std::fs::read_dir(from).ctx(ErrorCode::Sdk, format!("reading {}", from.display()))? {
        let e = e.ctx(ErrorCode::Sdk, "reading directory entry")?;
        let name = e.file_name();
        if skip.iter().any(|s| name == *s) || name.to_string_lossy().ends_with(".lock") {
            continue;
        }
        let (src, dst) = (e.path(), to.join(&name));
        let ty = e.file_type().ctx(ErrorCode::Sdk, "reading file type")?;
        if ty.is_dir() {
            copy_tree(&src, &dst, &[])?;
        } else if ty.is_file() {
            // On APFS, std::fs::copy uses clonefile: instant and space-free.
            std::fs::copy(&src, &dst).ctx(ErrorCode::Sdk, format!("copying {}", src.display()))?;
        }
    }
    Ok(())
}

pub fn disk_usage(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    let own = allocated(&meta);
    if !meta.is_dir() {
        return own;
    }
    std::fs::read_dir(path)
        .map(|rd| rd.flatten().map(|e| disk_usage(&e.path())).sum::<u64>())
        .unwrap_or(0)
        + own
}

#[cfg(unix)]
fn allocated(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.blocks() * 512
}

#[cfg(not(unix))]
fn allocated(meta: &std::fs::Metadata) -> u64 {
    meta.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    const STUDIO_STYLE: &str = "\
AvdId = Pixel_5_API_31
PlayStore.enabled = false
# a comment
abi.type = arm64-v8a
hw.lcd.width = 1080
image.sysdir.1 = system-images/android-31/google_apis/arm64-v8a/
odd=no-spaces
";

    #[test]
    fn config_ini_round_trips_byte_identical() {
        for text in [STUDIO_STYLE, "a=1\nb=2", "", "x = y\n\n\nz=w\n"] {
            assert_eq!(ConfigIni::parse(text).to_string(), text, "{text:?}");
        }
    }

    #[test]
    fn config_ini_edits_preserve_style() {
        let mut c = ConfigIni::parse(STUDIO_STYLE);
        assert_eq!(c.get("hw.lcd.width"), Some("1080"));
        c.set("hw.lcd.width", "1440");
        c.set("hw.ramSize", "4096");
        c.remove("odd");
        let out = c.to_string();
        assert!(out.contains("hw.lcd.width = 1440\n"));
        assert!(out.ends_with("hw.ramSize = 4096\n"), "{out}");
        assert!(out.contains("# a comment\n"));
        assert!(!out.contains("odd"));
        assert_eq!(ConfigIni::parse(&out).get("hw.ramSize"), Some("4096"));
    }

    fn profile() -> HardwareProfile {
        crate::profiles::builtin().into_iter().next().unwrap()
    }

    fn spec(name: &str) -> AvdSpec {
        AvdSpec {
            name: name.into(),
            display_name: format!("{name} display"),
            profile: profile(),
            image: ImageRef {
                api: 31,
                tag: "google_apis".into(),
                abi: "arm64-v8a".into(),
            },
            overrides: BTreeMap::from([("hw.keyboard".to_string(), "no".to_string())]),
        }
    }

    #[test]
    fn create_edit_clone_reset_delete() {
        let dir = tempfile::tempdir().unwrap();
        let m = AvdManager::new(dir.path());
        let info = m.create(&spec("a1")).unwrap();
        assert_eq!(info.image.as_ref().unwrap().api, 31);
        assert_eq!(m.create(&spec("a1")).unwrap_err().code, ErrorCode::Usage);
        let c = m.config("a1").unwrap();
        assert_eq!(c.get("hw.keyboard"), Some("no"), "overrides win");
        assert_eq!(c.get("hw.cpu.arch"), Some("arm64"));
        assert!(
            std::fs::read_to_string(dir.path().join("a1.ini"))
                .unwrap()
                .contains("a1.avd")
        );

        m.edit(
            "a1",
            &BTreeMap::from([
                ("hw.ramSize".into(), Some("3072".into())),
                ("hw.sdCard".into(), None),
            ]),
        )
        .unwrap();
        assert_eq!(m.config("a1").unwrap().get("hw.ramSize"), Some("3072"));
        assert_eq!(m.config("a1").unwrap().get("hw.sdCard"), None);

        // Simulate a used AVD: user data, a stale lock from a dead process, snapshots.
        let d = m.dir("a1");
        std::fs::write(d.join("userdata-qemu.img"), b"data").unwrap();
        std::fs::write(d.join("hardware-qemu.ini.lock"), b"999999").unwrap();
        std::fs::create_dir_all(d.join("snapshots/default_boot")).unwrap();
        assert!(!m.is_running("a1"), "stale lock from a dead pid");

        let cl = m.clone_avd("a1", "a2", "Copy").unwrap();
        assert_eq!(cl.display_name.as_deref(), Some("Copy"));
        assert_eq!(m.config("a2").unwrap().get("AvdId"), Some("a2"));
        assert!(m.dir("a2").join("userdata-qemu.img").is_file());
        assert!(!m.dir("a2").join("snapshots").exists());
        assert!(!m.dir("a2").join("hardware-qemu.ini.lock").exists());
        assert_eq!(
            m.list()
                .unwrap()
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            ["a1", "a2"]
        );

        m.factory_reset("a1").unwrap();
        assert!(!d.join("userdata-qemu.img").exists());
        assert!(d.join("config.ini").exists());

        m.delete("a1").unwrap();
        m.delete("a2").unwrap();
        assert!(m.list().unwrap().is_empty());
        assert!(!dir.path().join("a1.ini").exists());
        assert_eq!(m.delete("a1").unwrap_err().code, ErrorCode::NotFound);
    }

    const HW_QEMU: &str = "\
disk.dataPartition.path = /h/avd/../avd/src.avd/userdata-qemu.img
disk.ramdisk.path = /h/avd/src.avd/jidr/ramdisk.magisk-30.img
kernel.path = /sdk/system-images/android-35/google_apis/arm64-v8a//kernel-ranchu
avd.name = src
avd.id = src
android.avd.home = /h/avd
other.src.avd.like = /h/avd/src.avd.bak/x
";

    #[test]
    fn rewrite_moves_names_and_paths() {
        let out = rewrite_avd_refs(HW_QEMU, "src", "dst");
        assert!(
            out.contains("disk.dataPartition.path = /h/avd/../avd/dst.avd/userdata-qemu.img\n")
        );
        assert!(out.contains("disk.ramdisk.path = /h/avd/dst.avd/jidr/ramdisk.magisk-30.img\n"));
        assert!(out.contains("avd.name = dst\n") && out.contains("avd.id = dst\n"));
        assert!(
            out.contains("kernel.path = /sdk/system-images/"),
            "image paths stay"
        );
        assert!(out.contains("android.avd.home = /h/avd\n"));
        assert!(
            out.contains("/h/avd/src.avd.bak/x"),
            "only whole `<src>.avd` segments"
        );
        assert_eq!(out.lines().count(), HW_QEMU.lines().count());
        // A folder at the end of a value moves too; no-op for other AVDs.
        assert_eq!(
            rewrite_avd_refs("p = /a/src.avd\n", "src", "d"),
            "p = /a/d.avd\n"
        );
        assert_eq!(rewrite_avd_refs(HW_QEMU, "zzz", "dst"), HW_QEMU);
    }

    /// A used AVD with a saved snapshot, the quick-boot one, locks and tags.
    fn snapshotted(m: &AvdManager, name: &str) {
        m.create(&spec(name)).unwrap();
        let d = m.dir(name);
        std::fs::write(d.join("userdata-qemu.img.qcow2"), b"overlay").unwrap();
        std::fs::write(
            d.join("hardware-qemu.ini"),
            format!(
                "avd.name = {name}\ndisk.dataPartition.path = {}/userdata-qemu.img\n",
                d.display()
            ),
        )
        .unwrap();
        std::fs::write(d.join("hardware-qemu.ini.lock"), b"999999").unwrap();
        std::fs::write(d.join("multiinstance.lock"), b"").unwrap();
        for s in ["base", "other", "default_boot"] {
            let sd = d.join("snapshots").join(s);
            std::fs::create_dir_all(&sd).unwrap();
            std::fs::write(sd.join("snapshot.pb"), b"pb").unwrap();
            std::fs::write(sd.join("ram.bin"), vec![1u8; 64]).unwrap();
            std::fs::write(sd.join("hardware.ini"), format!("avd.name = {name}\n")).unwrap();
            std::fs::write(sd.join("snapshot.lock"), b"").unwrap();
        }
        std::fs::create_dir_all(d.join("jidr")).unwrap();
        std::fs::write(
            d.join("jidr/snapshots.json"),
            r#"{"base": {"root_mode": "adb", "created_at": "2026-11-02T00:00:00Z"}, "other": {"root_mode": "none", "created_at": "2026-11-02T00:00:00Z"}}"#,
        )
        .unwrap();
    }

    #[test]
    fn clone_with_snapshot_keeps_only_that_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let m = AvdManager::new(dir.path());
        snapshotted(&m, "src");
        m.clone_with_snapshot("src", "dst", "Copy", "base").unwrap();
        let d = m.dir("dst");
        assert_eq!(m.config("dst").unwrap().get("AvdId"), Some("dst"));
        assert_eq!(m.info("dst").unwrap().display_name.as_deref(), Some("Copy"));
        assert!(
            std::fs::read_to_string(dir.path().join("dst.ini"))
                .unwrap()
                .contains("dst.avd")
        );
        assert_eq!(
            std::fs::read(d.join("userdata-qemu.img.qcow2")).unwrap(),
            b"overlay"
        );
        assert_eq!(
            std::fs::read(d.join("snapshots/base/ram.bin"))
                .unwrap()
                .len(),
            64
        );
        for gone in [
            "snapshots/default_boot",
            "snapshots/other",
            "hardware-qemu.ini.lock",
            "multiinstance.lock",
            "snapshots/base/snapshot.lock",
        ] {
            assert!(!d.join(gone).exists(), "{gone} should be gone");
        }
        let hw = std::fs::read_to_string(d.join("hardware-qemu.ini")).unwrap();
        assert!(
            hw.contains("avd.name = dst") && hw.contains("/dst.avd/userdata-qemu.img"),
            "{hw}"
        );
        let snap_hw = std::fs::read_to_string(d.join("snapshots/base/hardware.ini")).unwrap();
        assert_eq!(snap_hw, "avd.name = dst\n");
        let tags: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(d.join("jidr/snapshots.json")).unwrap())
                .unwrap();
        assert_eq!(
            tags["base"]["root_mode"], "adb",
            "the root tag travels (D29)"
        );
        assert!(tags.get("other").is_none());
        // The source is untouched.
        let s = m.dir("src");
        assert!(s.join("snapshots/default_boot").is_dir() && s.join("snapshots/other").is_dir());
        assert!(
            std::fs::read_to_string(s.join("hardware-qemu.ini"))
                .unwrap()
                .contains("avd.name = src")
        );
    }

    #[test]
    fn clone_with_snapshot_refuses_bad_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let m = AvdManager::new(dir.path());
        snapshotted(&m, "src");
        assert_eq!(
            m.clone_with_snapshot("src", "x", "X", "missing")
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        std::fs::write(m.dir("src").join("snapshots/other/ram.bin"), b"").unwrap();
        let e = m.clone_with_snapshot("src", "x", "X", "other").unwrap_err();
        assert_eq!(e.code, ErrorCode::Sdk, "an empty ram.bin can't be resumed");
        for bad in ["..", "a/b", ".x", ""] {
            assert_eq!(
                m.clone_with_snapshot("src", "x", "X", bad)
                    .unwrap_err()
                    .code,
                ErrorCode::Usage,
                "{bad:?}"
            );
        }
        assert!(!m.exists("x"), "nothing left behind");
        std::fs::write(
            m.dir("src").join("hardware-qemu.ini.lock"),
            std::process::id().to_string(),
        )
        .unwrap();
        assert_eq!(
            m.clone_with_snapshot("src", "x", "X", "base")
                .unwrap_err()
                .code,
            ErrorCode::Busy
        );
    }

    /// Regression (Phase 13c): Jidr AVDs had no `AVD.conf`, so the headless
    /// emulator never saved quick-boot state on exit.
    #[test]
    fn save_on_exit_setting() {
        assert_eq!(
            with_save_on_exit("", true),
            "[perAvd]\nset\\saveSnapshotOnExit=0\n"
        );
        let studio = "[perAvd]\nbattery\\charge_level=100\nset\\saveSnapshotOnExit=1\nset\\pauseAvdWhenMinimized=false\n";
        assert_eq!(
            with_save_on_exit(studio, true),
            "[perAvd]\nbattery\\charge_level=100\nset\\saveSnapshotOnExit=0\nset\\pauseAvdWhenMinimized=false\n"
        );
        let other = "[General]\nx=1\n[perAvd]\nloc\\altitude=5\n";
        assert_eq!(
            with_save_on_exit(other, false),
            "[General]\nx=1\n[perAvd]\nloc\\altitude=5\nset\\saveSnapshotOnExit=1\n"
        );
        assert_eq!(
            with_save_on_exit("[General]\nx=1\n", true),
            "[General]\nx=1\n[perAvd]\nset\\saveSnapshotOnExit=0\n"
        );
        let dir = tempfile::tempdir().unwrap();
        let m = AvdManager::new(dir.path());
        m.create(&spec("q")).unwrap();
        m.set_save_on_exit("q", true).unwrap();
        let conf = std::fs::read_to_string(m.dir("q").join("AVD.conf")).unwrap();
        assert!(conf.contains("set\\saveSnapshotOnExit=0"), "{conf}");
    }

    #[test]
    fn running_avd_is_protected() {
        let dir = tempfile::tempdir().unwrap();
        let m = AvdManager::new(dir.path());
        m.create(&spec("busy")).unwrap();
        // Our own pid is alive.
        std::fs::write(
            m.dir("busy").join("hardware-qemu.ini.lock"),
            std::process::id().to_string(),
        )
        .unwrap();
        assert!(m.is_running("busy"));
        assert_eq!(m.delete("busy").unwrap_err().code, ErrorCode::Busy);
        assert_eq!(m.factory_reset("busy").unwrap_err().code, ErrorCode::Busy);
        assert_eq!(
            m.clone_avd("busy", "b2", "x").unwrap_err().code,
            ErrorCode::Busy
        );
    }

    /// Regression: real emulator lock files are NUL-padded after the pid;
    /// parsing failed and every stopped AVD looked busy.
    #[test]
    fn nul_padded_lock_files() {
        assert_eq!(lock_pid(b"12345\0\0\0\0"), Some(12345));
        assert_eq!(lock_pid(b" 42\n"), Some(42));
        assert_eq!(lock_pid(b"\0\0"), None);
        let dir = tempfile::tempdir().unwrap();
        let m = AvdManager::new(dir.path());
        m.create(&spec("pad")).unwrap();
        let lock = m.dir("pad").join("hardware-qemu.ini.lock");
        std::fs::write(&lock, b"999999\0\0\0").unwrap();
        assert!(!m.is_running("pad"), "dead pid");
        let mut own = std::process::id().to_string().into_bytes();
        own.extend([0, 0, 0]);
        std::fs::write(&lock, own).unwrap();
        assert!(m.is_running("pad"), "live pid");
    }

    #[test]
    fn names_are_validated() {
        for bad in ["", ".hidden", "a b", "a/b", "x".repeat(65).as_str()] {
            assert_eq!(
                validate_name(bad).unwrap_err().code,
                ErrorCode::Usage,
                "{bad:?}"
            );
        }
        validate_name("Pixel_6.1-fhd").unwrap();
    }
}
