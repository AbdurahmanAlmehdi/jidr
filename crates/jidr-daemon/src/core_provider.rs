//! The daemon's own commands: daemon, sdk, image, profile, device, file, logcat.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use jidr_adb::{Adb, InstallOptions};
use jidr_core::provider::{
    CallCtx, CommandProvider, CommandSpec, DaemonContext, DeviceParams, NoParams, parse_params,
};
use jidr_core::{Device, ImageRef, JidrError, JidrResult, RootMode, RootState};
use jidr_sdk::{AvdSpec, CatalogFilter, ConsentStore, Fetcher, Repository, profiles, repo};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::state::State;

pub struct CoreProvider {
    state: Arc<State>,
}

impl CoreProvider {
    pub fn new(state: Arc<State>) -> Self {
        Self { state }
    }
}

// ---- params -------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateParams {
    /// Unique device name.
    pub name: String,
    /// System image: `35`, `35:google_apis`, `35:google_apis:arm64-v8a` or
    /// `system-images;android-35;google_apis;arm64-v8a`.
    pub image: String,
    /// Hardware profile id (`profile.list`); default `phone-6.1-fhd`.
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub ram_mb: Option<u32>,
    #[serde(default)]
    pub cpus: Option<u32>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub density: Option<u32>,
    #[serde(default)]
    pub data_mb: Option<u32>,
    /// Default: `adb` where the image allows it, else `none` (D7).
    #[serde(default)]
    pub root_mode: Option<RootMode>,
    /// Agents may run destructive commands on it (D5).
    #[serde(default)]
    pub agent_owned: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditParams {
    /// Device name or id.
    pub device: String,
    /// New name.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub ram_mb: Option<u32>,
    #[serde(default)]
    pub cpus: Option<u32>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub density: Option<u32>,
    #[serde(default)]
    pub agent_owned: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloneParams {
    /// Device to copy (must be stopped).
    pub device: String,
    /// Name of the copy.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StartParams {
    /// Device name or id.
    pub device: String,
    /// Ignore the quick-boot snapshot.
    #[serde(default)]
    pub cold_boot: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallParams {
    pub device: String,
    /// Path of the APK on this machine.
    pub apk: PathBuf,
    /// Replace an installed app (`-r`).
    #[serde(default = "yes")]
    pub replace: bool,
    /// Grant runtime permissions (`-g`).
    #[serde(default)]
    pub grant_permissions: bool,
    /// Allow a version downgrade (`-d`).
    #[serde(default)]
    pub downgrade: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PushParams {
    pub device: String,
    /// Local file.
    pub local: PathBuf,
    /// Device path; default `/sdcard/Download/`.
    #[serde(default)]
    pub remote: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PullParams {
    pub device: String,
    pub remote: String,
    pub local: PathBuf,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogcatDumpParams {
    pub device: String,
    /// Only the last N lines.
    #[serde(default)]
    pub lines: Option<u32>,
    /// logcat filter specs, e.g. `["ActivityManager:I", "*:S"]`.
    #[serde(default)]
    pub filter: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackagesParams {
    /// SDK package paths, e.g. `emulator`, `system-images;android-35;google_apis;arm64-v8a`.
    pub packages: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AcceptLicenseParams {
    /// License id, e.g. `android-sdk-license`.
    pub id: String,
    /// SHA-256 of the license text you were shown (from `sdk.licenses`).
    pub sha256: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImageListParams {
    /// Include images that aren't installed (needs network; falls back to
    /// installed images when offline).
    #[serde(default = "yes")]
    pub available: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImageParams {
    /// `35`, `35:google_apis` or a full `system-images;…` package.
    pub image: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeviceDetails {
    pub device: Device,
    pub avd_dir: PathBuf,
    pub config: BTreeMap<String, String>,
    /// Bytes on disk used by the device's AVD (D30).
    pub disk_usage: u64,
}

// ---- helpers ------------------------------------------------------------

/// Parses an image selector; the ABI defaults to the host's.
pub fn parse_image(sel: &str) -> JidrResult<ImageRef> {
    if sel.starts_with("system-images;") {
        return ImageRef::parse_package(sel)
            .ok_or_else(|| JidrError::usage(format!("invalid image package {sel:?}")));
    }
    let mut parts = sel.split(':');
    let api = parts
        .next()
        .and_then(|a| a.trim().trim_start_matches("android-").parse().ok())
        .ok_or_else(|| {
            JidrError::usage(format!(
                "invalid image {sel:?}: expected e.g. 35:google_apis"
            ))
        })?;
    let tag = parts
        .next()
        .filter(|t| !t.is_empty())
        .unwrap_or("google_apis")
        .to_string();
    let abi = parts
        .next()
        .filter(|a| !a.is_empty())
        .unwrap_or(jidr_sdk::host::host_abi())
        .to_string();
    if parts.next().is_some() {
        return Err(JidrError::usage(format!("invalid image {sel:?}")));
    }
    Ok(ImageRef { api, tag, abi })
}

fn validate_device_name(name: &str) -> JidrResult<()> {
    let name = name.trim();
    if name.is_empty() || name.len() > 64 || name.contains(['/', '\n', '\r']) {
        return Err(JidrError::usage(
            "device names are 1–64 characters, without / or newlines",
        ));
    }
    if Uuid::parse_str(name).is_ok() {
        return Err(JidrError::usage(
            "a device name can't look like a device id",
        ));
    }
    Ok(())
}

fn json<T: Serialize>(v: T) -> JidrResult<Value> {
    serde_json::to_value(v).map_err(|e| JidrError::unexpected(format!("encoding result: {e}")))
}

fn hw_changes(
    ram_mb: Option<u32>,
    cpus: Option<u32>,
    width: Option<u32>,
    height: Option<u32>,
    density: Option<u32>,
) -> BTreeMap<String, Option<String>> {
    [
        ("hw.ramSize", ram_mb),
        ("hw.cpu.ncore", cpus),
        ("hw.lcd.width", width),
        ("hw.lcd.height", height),
        ("hw.lcd.density", density),
    ]
    .into_iter()
    .filter_map(|(k, v)| v.map(|v| (k.to_string(), Some(v.to_string()))))
    .collect()
}

impl CoreProvider {
    async fn adb(&self, device: &Device) -> JidrResult<Adb> {
        let info = self.state.connect_info(device.id).await?;
        Ok(Adb::new(info.adb_path).device(info.adb_serial))
    }

    async fn repository(&self) -> JidrResult<Repository> {
        Fetcher::new()?.repository(&repo::repo_base()).await
    }

    fn consent(&self) -> JidrResult<ConsentStore> {
        ConsentStore::load(self.state.paths.consent_file())
    }

    /// Installs packages into the private SDK after checking licenses.
    async fn install_packages(
        &self,
        ctx: &CallCtx,
        repo: &Repository,
        packages: &[String],
    ) -> JidrResult<Value> {
        let pkgs = packages
            .iter()
            .map(|p| {
                repo.latest(p, 0)
                    .ok_or_else(|| JidrError::not_found(format!("no SDK package {p}")))
            })
            .collect::<JidrResult<Vec<_>>>()?;
        let consent = self.consent()?;
        let pending = consent.pending(repo, &pkgs)?;
        if !pending.is_empty() {
            return Err(JidrError::sdk(format!(
                "accept these licenses first: {}",
                pending
                    .iter()
                    .map(|p| p.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
            .with_data(json!({ "pending_licenses": pending })));
        }
        let fetcher = Fetcher::new()?;
        let sdk = self.state.sdks().private;
        let n = pkgs.len() as f32;
        let mut installed = Vec::new();
        for (i, pkg) in pkgs.iter().enumerate() {
            let report = |p: jidr_sdk::Progress| {
                let frac = if p.total > 0 {
                    p.done as f32 / p.total as f32
                } else {
                    0.0
                };
                ctx.report(
                    Some((i as f32 + frac) / n),
                    &format!("{:?} {}", p.phase, p.package),
                );
            };
            let dir = jidr_sdk::install(&fetcher, &sdk, repo, &consent, pkg, &report, &ctx.cancel)
                .await?;
            installed.push(
                json!({ "package": pkg.path, "revision": pkg.revision.to_string(), "path": dir }),
            );
        }
        Ok(json!({ "installed": installed }))
    }

    async fn create(&self, p: CreateParams) -> JidrResult<Value> {
        validate_device_name(&p.name)?;
        let name = p.name.trim().to_string();
        if self.state.with_store(|s| s.name_taken(&name, None)) {
            return Err(JidrError::usage(format!(
                "a device named {name:?} already exists"
            )));
        }
        let image = parse_image(&p.image)?;
        let sdks = self.state.sdks();
        let sdk = sdks.find_image(&image).cloned().ok_or_else(|| {
            JidrError::sdk(format!(
                "{} is not installed; run `jidr image install {}`",
                image.package(),
                p.image
            ))
        })?;
        let mut profile = match &p.profile {
            Some(id) => profiles::find(id)?,
            None => profiles::default_profile()?,
        };
        if let Some(v) = p.ram_mb {
            profile.ram_mb = v;
        }
        if let Some(v) = p.cpus {
            profile.cpus = v;
        }
        if let Some(v) = p.width {
            profile.width = v;
        }
        if let Some(v) = p.height {
            profile.height = v;
        }
        if let Some(v) = p.density {
            profile.density = v;
        }
        if let Some(v) = p.data_mb {
            profile.data_mb = v;
        }
        let mode = p.root_mode.unwrap_or(if image.allows_adb_root() {
            RootMode::Adb
        } else {
            RootMode::None
        });
        if mode == RootMode::Adb && !image.allows_adb_root() {
            return Err(JidrError::usage(format!(
                "{} images don't allow adb root; use root_mode magisk or none",
                image.tag
            )));
        }
        let avds = self.state.avds();
        let avd = self
            .state
            .with_store(|s| s.avd_id_for(&name, |id| avds.exists(id)));
        avds.create(&AvdSpec {
            name: avd.clone(),
            display_name: name.clone(),
            profile: profile.clone(),
            image: image.clone(),
            overrides: BTreeMap::new(),
        })?;
        let device = Device {
            id: Uuid::new_v4(),
            name,
            avd: avd.clone(),
            profile: profile.id,
            image,
            sdk_root: sdk.root,
            root: RootState {
                mode,
                ..Default::default()
            },
            agent_owned: p.agent_owned.unwrap_or(false),
            created_at: chrono::Utc::now(),
            launch: Default::default(),
            status: Default::default(),
            runtime: None,
        };
        if let Err(e) = self.state.with_store(|s| s.insert(device.clone())) {
            let _ = avds.delete(&avd);
            return Err(e);
        }
        self.state.emit(
            "device.created",
            json!({ "device": device.id, "name": device.name }),
        );
        json(device)
    }

    async fn edit(&self, p: EditParams) -> JidrResult<Value> {
        let d = self.state.device(&p.device).await?;
        let changes = hw_changes(p.ram_mb, p.cpus, p.width, p.height, p.density);
        if !changes.is_empty() {
            if d.status.is_active() {
                return Err(JidrError::busy(format!(
                    "{} is running; stop it to change its hardware",
                    d.name
                )));
            }
            self.state.avds().edit(&d.avd, &changes)?;
        }
        if let Some(name) = &p.name {
            validate_device_name(name)?;
            if self.state.with_store(|s| s.name_taken(name, Some(d.id))) {
                return Err(JidrError::usage(format!(
                    "a device named {name:?} already exists"
                )));
            }
            self.state.avds().edit(
                &d.avd,
                &BTreeMap::from([("avd.ini.displayname".into(), Some(name.clone()))]),
            )?;
        }
        let (name, agent_owned) = (p.name, p.agent_owned);
        let d = self
            .state
            .update_device(
                d.id,
                Box::new(move |d| {
                    if let Some(n) = name {
                        d.name = n.trim().to_string();
                    }
                    if let Some(a) = agent_owned {
                        d.agent_owned = a;
                    }
                }),
            )
            .await?;
        json(d)
    }

    async fn clone_device(&self, p: CloneParams) -> JidrResult<Value> {
        let src = self.state.device(&p.device).await?;
        if src.status.is_active() {
            return Err(JidrError::busy(format!(
                "{} is running; stop it before cloning",
                src.name
            )));
        }
        validate_device_name(&p.name)?;
        let name = p.name.trim().to_string();
        if self.state.with_store(|s| s.name_taken(&name, None)) {
            return Err(JidrError::usage(format!(
                "a device named {name:?} already exists"
            )));
        }
        let avds = self.state.avds();
        let avd = self
            .state
            .with_store(|s| s.avd_id_for(&name, |id| avds.exists(id)));
        avds.clone_avd(&src.avd, &avd, &name)?;
        let (src_dir, dst_dir) = (avds.dir(&src.avd), avds.dir(&avd));
        let mut launch = src.launch.clone();
        // Per-device files (e.g. a patched ramdisk) move with the copy.
        if let Some(r) = &launch.ramdisk
            && let Ok(rel) = r.strip_prefix(&src_dir)
        {
            launch.ramdisk = Some(dst_dir.join(rel));
        }
        let mut root = src.root.clone();
        if let Some(r) = &root.patched_ramdisk_path
            && let Ok(rel) = r.strip_prefix(&src_dir)
        {
            root.patched_ramdisk_path = Some(dst_dir.join(rel));
        }
        launch.force_cold_boot = true;
        let device = Device {
            id: Uuid::new_v4(),
            name,
            avd: avd.clone(),
            created_at: chrono::Utc::now(),
            launch,
            root,
            status: Default::default(),
            runtime: None,
            ..src
        };
        if let Err(e) = self.state.with_store(|s| s.insert(device.clone())) {
            let _ = avds.delete(&avd);
            return Err(e);
        }
        self.state.emit(
            "device.created",
            json!({ "device": device.id, "name": device.name }),
        );
        json(device)
    }

    async fn reset(&self, p: DeviceParams) -> JidrResult<Value> {
        let d = self.state.device(&p.device).await?;
        if d.status.is_active() {
            self.state.stop_device(d.id).await?;
        }
        self.state.avds().factory_reset(&d.avd)?;
        let d = self
            .state
            .update_device(d.id, Box::new(|d| d.launch.force_cold_boot = true))
            .await?;
        json(d)
    }

    async fn delete(&self, p: DeviceParams) -> JidrResult<Value> {
        let d = self.state.device(&p.device).await?;
        if d.status.is_active() {
            self.state.stop_device(d.id).await?;
        }
        let avds = self.state.avds();
        if avds.exists(&d.avd) {
            avds.delete(&d.avd)?;
        }
        self.state.with_store(|s| s.remove(d.id))?;
        self.state
            .emit("device.deleted", json!({ "device": d.id, "name": d.name }));
        Ok(json!({ "deleted": d.id, "name": d.name }))
    }

    async fn details(&self, p: DeviceParams) -> JidrResult<Value> {
        let device = self.state.device(&p.device).await?;
        let avds = self.state.avds();
        let config = avds
            .config(&device.avd)
            .map(|c| c.entries())
            .unwrap_or_default();
        let disk_usage = avds.disk_usage(&device.avd).unwrap_or(0);
        json(DeviceDetails {
            avd_dir: avds.dir(&device.avd),
            device,
            config,
            disk_usage,
        })
    }

    async fn image_list(&self, p: ImageListParams) -> JidrResult<Value> {
        let repo = if p.available {
            match self.repository().await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("offline image list: {e}");
                    Repository::default()
                }
            }
        } else {
            Repository::default()
        };
        json(jidr_sdk::image_catalog(
            &repo,
            &self.state.sdks(),
            &CatalogFilter::default(),
        ))
    }

    async fn image_install(&self, ctx: &CallCtx, p: ImageParams) -> JidrResult<Value> {
        let image = parse_image(&p.image)?;
        let repo = self.repository().await?;
        let mut packages = Vec::new();
        if self.state.sdks().tools().is_err() {
            packages.extend(["emulator".to_string(), "platform-tools".to_string()]);
        }
        if self.state.sdks().find_image(&image).is_none() {
            packages.push(image.package());
        }
        if packages.is_empty() {
            return Ok(
                json!({ "installed": [], "message": format!("{} is already installed", image.package()) }),
            );
        }
        self.install_packages(ctx, &repo, &packages).await
    }

    async fn sdk_status(&self) -> JidrResult<Value> {
        let sdks = self.state.sdks();
        let tools = sdks.tools().ok().cloned();
        Ok(json!({
            "private_sdk": sdks.private.root,
            "studio_sdk": sdks.studio.as_ref().map(|s| &s.root),
            "tools_sdk": tools.as_ref().map(|s| &s.root),
            "emulator_version": tools.as_ref().and_then(|s| s.emulator_version()),
            "hypervisor": jidr_sdk::host::hypervisor_available(),
            "host_abi": jidr_sdk::host::host_abi(),
            "free_bytes": jidr_sdk::host::free_space(&self.state.paths.data).unwrap_or(0),
            "installed_images": sdks.private.installed_images().into_iter()
                .chain(sdks.studio.iter().flat_map(|s| s.installed_images()))
                .map(|i| i.package()).collect::<Vec<_>>(),
        }))
    }
}

#[async_trait]
impl CommandProvider for CoreProvider {
    fn name(&self) -> &str {
        "core"
    }

    fn commands(&self) -> Vec<CommandSpec> {
        vec![
            CommandSpec::new::<NoParams>(
                "daemon.status",
                "Daemon version, uptime and device counts",
            ),
            CommandSpec::new::<NoParams>(
                "sdk.status",
                "Where the SDKs are, emulator version, hypervisor, free disk",
            ),
            CommandSpec::new::<PackagesParams>(
                "sdk.licenses",
                "Licenses still to accept before installing packages, with their text",
            ),
            CommandSpec::new::<AcceptLicenseParams>(
                "sdk.accept_license",
                "Record consent to a license text by its SHA-256 (D8)",
            ),
            CommandSpec::new::<PackagesParams>(
                "sdk.install",
                "Install SDK packages into Jidr's private SDK",
            )
            .long_running(),
            CommandSpec::new::<ImageListParams>(
                "image.list",
                "System images for this host, installed or available",
            ),
            CommandSpec::new::<ImageParams>(
                "image.install",
                "Install a system image (and the emulator if missing)",
            )
            .long_running(),
            CommandSpec::new::<NoParams>("profile.list", "Built-in hardware profiles"),
            CommandSpec::new::<CreateParams>(
                "device.create",
                "Create a device from a profile and a system image",
            ),
            CommandSpec::new::<EditParams>(
                "device.edit",
                "Rename a device or change its hardware (stopped)",
            )
            .locks_device(),
            CommandSpec::new::<CloneParams>(
                "device.clone",
                "Copy a stopped device under a new name",
            )
            .locks_device(),
            CommandSpec::new::<StartParams>(
                "device.start",
                "Start a device and wait until it has booted",
            )
            .long_running()
            .locks_device(),
            CommandSpec::new::<DeviceParams>(
                "device.stop",
                "Stop a device, saving its quick-boot state",
            )
            .locks_device(),
            CommandSpec::new::<DeviceParams>("device.reset", "Factory reset: wipe all user data")
                .destructive()
                .locks_device(),
            CommandSpec::new::<DeviceParams>("device.delete", "Delete a device and its data")
                .destructive()
                .locks_device(),
            CommandSpec::new::<NoParams>("device.list", "All devices with their status"),
            CommandSpec::new::<DeviceParams>(
                "device.details",
                "A device's settings, config.ini, status and disk usage",
            ),
            CommandSpec::new::<DeviceParams>(
                "device.connect_info",
                "gRPC endpoint, token and adb serial of a running device",
            ),
            CommandSpec::new::<InstallParams>("file.install", "Install an APK on a running device")
                .locks_device(),
            CommandSpec::new::<PushParams>(
                "file.push",
                "Copy a file to a running device (default /sdcard/Download/)",
            ),
            CommandSpec::new::<PullParams>("file.pull", "Copy a file from a running device"),
            CommandSpec::new::<LogcatDumpParams>("logcat.dump", "Dump the device log"),
            CommandSpec::new::<DeviceParams>("logcat.clear", "Clear the device log"),
        ]
    }

    async fn call(&self, ctx: &CallCtx, method: &str, params: Value) -> JidrResult<Value> {
        match method {
            "daemon.status" => {
                parse_params::<NoParams>(params)?;
                let devices = self.state.devices().await;
                Ok(json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "protocol": crate::rpc::PROTOCOL_VERSION,
                    "pid": std::process::id(),
                    "uptime_s": self.state.started_at.elapsed().as_secs(),
                    "devices": devices.len(),
                    "running": self.state.running_count(),
                    "socket": self.state.paths.socket(),
                }))
            }
            "sdk.status" => {
                parse_params::<NoParams>(params)?;
                self.sdk_status().await
            }
            "sdk.licenses" => {
                let p: PackagesParams = parse_params(params)?;
                let repo = self.repository().await?;
                let pkgs = p
                    .packages
                    .iter()
                    .map(|n| {
                        repo.latest(n, 0)
                            .ok_or_else(|| JidrError::not_found(format!("no SDK package {n}")))
                    })
                    .collect::<JidrResult<Vec<_>>>()?;
                json(self.consent()?.pending(&repo, &pkgs)?)
            }
            "sdk.accept_license" => {
                let p: AcceptLicenseParams = parse_params(params)?;
                let repo = self.repository().await?;
                json(self.consent()?.accept_hash(&repo, &p.id, &p.sha256)?)
            }
            "sdk.install" => {
                let p: PackagesParams = parse_params(params)?;
                let repo = self.repository().await?;
                self.install_packages(ctx, &repo, &p.packages).await
            }
            "image.list" => self.image_list(parse_params(params)?).await,
            "image.install" => self.image_install(ctx, parse_params(params)?).await,
            "profile.list" => {
                parse_params::<NoParams>(params)?;
                json(profiles::builtin())
            }
            "device.create" => self.create(parse_params(params)?).await,
            "device.edit" => self.edit(parse_params(params)?).await,
            "device.clone" => self.clone_device(parse_params(params)?).await,
            "device.start" => {
                let p: StartParams = parse_params(params)?;
                let d = self.state.device(&p.device).await?;
                json(
                    self.state
                        .start_device(d.id, p.cold_boot, &ctx.cancel)
                        .await?,
                )
            }
            "device.stop" => {
                let p: DeviceParams = parse_params(params)?;
                let d = self.state.device(&p.device).await?;
                json(self.state.stop_device(d.id).await?)
            }
            "device.reset" => self.reset(parse_params(params)?).await,
            "device.delete" => self.delete(parse_params(params)?).await,
            "device.list" => {
                parse_params::<NoParams>(params)?;
                json(self.state.devices().await)
            }
            "device.details" => self.details(parse_params(params)?).await,
            "device.connect_info" => {
                let p: DeviceParams = parse_params(params)?;
                let d = self.state.device(&p.device).await?;
                json(self.state.connect_info(d.id).await?)
            }
            "file.install" => {
                let p: InstallParams = parse_params(params)?;
                let d = self.state.device(&p.device).await?;
                if !p.apk.is_file() {
                    return Err(JidrError::usage(format!(
                        "no such APK: {}",
                        p.apk.display()
                    )));
                }
                let opts = InstallOptions {
                    replace: p.replace,
                    grant_permissions: p.grant_permissions,
                    downgrade: p.downgrade,
                };
                self.adb(&d)
                    .await?
                    .with_timeout(Duration::from_secs(600))
                    .install(&p.apk, opts)
                    .await?;
                Ok(json!({ "installed": p.apk }))
            }
            "file.push" => {
                let p: PushParams = parse_params(params)?;
                let d = self.state.device(&p.device).await?;
                let remote = p.remote.unwrap_or_else(|| "/sdcard/Download/".into());
                self.adb(&d)
                    .await?
                    .with_timeout(Duration::from_secs(600))
                    .push(&p.local, &remote)
                    .await?;
                Ok(json!({ "pushed": p.local, "remote": remote }))
            }
            "file.pull" => {
                let p: PullParams = parse_params(params)?;
                let d = self.state.device(&p.device).await?;
                self.adb(&d)
                    .await?
                    .with_timeout(Duration::from_secs(600))
                    .pull(&p.remote, &p.local)
                    .await?;
                Ok(json!({ "pulled": p.remote, "local": p.local }))
            }
            "logcat.dump" => {
                let p: LogcatDumpParams = parse_params(params)?;
                let d = self.state.device(&p.device).await?;
                let lines = p.lines.map(|n| n.to_string());
                let mut args: Vec<&str> = Vec::new();
                if let Some(n) = &lines {
                    args.extend(["-t", n]);
                }
                args.extend(p.filter.iter().map(String::as_str));
                let text = self.adb(&d).await?.logcat_dump(&args).await?;
                Ok(json!({ "text": text }))
            }
            "logcat.clear" => {
                let p: DeviceParams = parse_params(params)?;
                let d = self.state.device(&p.device).await?;
                self.adb(&d).await?.logcat_clear().await?;
                Ok(json!({ "cleared": true }))
            }
            other => Err(JidrError::usage(format!(
                "core provider has no command {other}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_selectors() {
        let host = jidr_sdk::host::host_abi();
        assert_eq!(
            parse_image("35").unwrap(),
            ImageRef {
                api: 35,
                tag: "google_apis".into(),
                abi: host.into()
            }
        );
        assert_eq!(parse_image("33:default").unwrap().tag, "default");
        assert_eq!(parse_image("34:google_apis:x86_64").unwrap().abi, "x86_64");
        assert_eq!(
            parse_image("system-images;android-31;google_apis_playstore;arm64-v8a")
                .unwrap()
                .tag,
            "google_apis_playstore"
        );
        for bad in ["", "x", "35:a:b:c", "system-images;nope"] {
            assert!(parse_image(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn device_names() {
        validate_device_name("My phone 6.1″").unwrap();
        for bad in [
            "",
            "   ",
            "a/b",
            &"x".repeat(65),
            "6f9619ff-8b86-d011-b42d-00cf4fc964ff",
        ] {
            assert!(validate_device_name(bad).is_err(), "{bad}");
        }
    }
}
