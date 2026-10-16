//! Magisk mode: the per-device patched ramdisk and the `root.set magisk`
//! state machine.
//!
//! Flow (plan, "Root engine"): fetch the pinned APK → hash the pristine
//! `ramdisk.img` → patch a per-device copy with `jidr-cpio` → cold boot with
//! `-ramdisk` → install the app → set up `/data/adb/magisk` → one reboot →
//! grant the shell su → verify. Revert clears the ramdisk, deletes the patched
//! file and cold-boots. Re-patching happens whenever the pristine hash or the
//! pinned version differs from what was recorded; patches never stack.

pub mod auto_grant;
pub mod env;
pub mod grant;
pub mod patch;
pub mod pins;

use std::path::{Path, PathBuf};

use jidr_adb::InstallOptions;
use jidr_core::provider::CallCtx;
use jidr_core::{Device, ErrorCode, JidrError, JidrResult, ResultExt, RootHealth, RootMode};
use jidr_sdk::DownloadCache;
use serde_json::{Value, json};

use crate::verify;
use pins::MagiskPin;

/// Where the pristine `ramdisk.img` of a device's image lives.
pub fn pristine_ramdisk_path(device: &Device) -> PathBuf {
    device
        .sdk_root
        .join(device.image.sysdir())
        .join("ramdisk.img")
}

/// The per-device patched ramdisk, under `<avd>/jidr/` (never the SDK image).
pub fn patched_ramdisk_path(ctx: &CallCtx, device: &Device, version: &str) -> PathBuf {
    ctx.daemon
        .paths()
        .avd_home()
        .join(format!("{}.avd", device.avd))
        .join("jidr")
        .join(format!("ramdisk.magisk-{version}.img"))
}

/// Resolves the Magisk APK: either the pinned release (downloaded through the
/// 5b cache, verified by SHA-256) or a local `--magisk <path>` imported into
/// the cache. Returns the APK path, its SHA-256 and the Magisk version it
/// carries.
async fn resolve_apk(
    ctx: &CallCtx,
    pin: &MagiskPin,
    magisk_sha256: Option<&str>,
) -> JidrResult<(PathBuf, String, String)> {
    let cache = DownloadCache::new(ctx.daemon.paths().downloads());
    match magisk_sha256 {
        Some(sha) => {
            let apk = cache.get_verified(sha)?.ok_or_else(|| {
                JidrError::usage(format!(
                    "no Magisk APK with sha256 {sha} in the download cache; \
                     import one with `jidr root set --magisk <apk>` first"
                ))
            })?;
            let version = patch::version_from_apk(&apk)?;
            Ok((apk, sha.to_string(), version))
        }
        None => {
            // A verified cached copy means no download, so the GPL licence and
            // source link are shown only before the first fetch.
            if cache.get_verified(&pin.sha256)?.is_none() {
                ctx.report(None, &pins::notice(pin));
            }
            let fetcher = jidr_sdk::Fetcher::new()?;
            let report = ctx.clone();
            let version = pin.version.clone();
            let apk = cache
                .fetch(
                    &fetcher,
                    &pin.url,
                    &pin.sha256,
                    &move |done, total| {
                        let frac = if total > 0 {
                            done as f32 / total as f32
                        } else {
                            0.0
                        };
                        report.report(Some(frac), &format!("Downloading Magisk {version}"));
                    },
                    &ctx.cancel,
                )
                .await?;
            Ok((apk, pin.sha256.clone(), pin.version.clone()))
        }
    }
}

/// Hashes the pristine ramdisk and rebuilds the patched copy when the pristine
/// hash, the Magisk version or the device's build props (Phase 9f) differ
/// from what is recorded (patches never stack). Returns the patched file path.
async fn ensure_patched(
    ctx: &CallCtx,
    device: &Device,
    version: &str,
    apk: &Path,
    apk_sha256: &str,
) -> JidrResult<PathBuf> {
    let pristine_path = pristine_ramdisk_path(device);
    let pristine_sha = jidr_sdk::cache::sha256_file(&pristine_path)?;
    let patched = patched_ramdisk_path(ctx, device, version);
    let props = crate::props::load(ctx, device);

    if patched.is_file()
        && patch::read_provenance(&patched).is_some_and(|p| {
            p.pristine_sha256 == pristine_sha && p.magisk_version == version && p.props == props
        })
    {
        return Ok(patched);
    }

    ctx.report(None, &format!("Patching the ramdisk for Magisk {version}"));
    let pristine = std::fs::read(&pristine_path).ctx(
        ErrorCode::Sdk,
        format!("reading {}", pristine_path.display()),
    )?;
    let payload = patch::load_payload(apk, &device.image.abi)?;
    let bytes = patch::patch_ramdisk(&pristine, &payload, &props)?;
    patch::write_atomic(&patched, &bytes)?;
    patch::write_provenance(
        &patched,
        &patch::Provenance {
            magisk_version: version.to_string(),
            pristine_sha256: pristine_sha,
            apk_sha256: Some(apk_sha256.to_string()),
            props,
        },
    )?;
    Ok(patched)
}

/// Rebuilds a Full-root device's patched ramdisk after its build props
/// changed (Phase 9f), with the same Magisk APK it was patched from (or the
/// cached pinned release of the same version), and makes the next start a
/// cold boot. Never downloads.
pub(crate) async fn repatch(ctx: &CallCtx, device: &Device) -> JidrResult<Device> {
    let version = device
        .root
        .magisk_version
        .clone()
        .ok_or_else(|| JidrError::usage(format!("{} has no Magisk patch yet", device.name)))?;
    let cache = DownloadCache::new(ctx.daemon.paths().downloads());
    let recorded = device
        .root
        .patched_ramdisk_path
        .as_deref()
        .and_then(patch::read_provenance)
        .and_then(|p| p.apk_sha256);
    let pin = pins::default_pin().ok().filter(|p| p.version == version);
    let mut found = None;
    for sha in recorded.into_iter().chain(pin.map(|p| p.sha256)) {
        if let Some(apk) = cache.get_verified(&sha)? {
            found = Some((apk, sha));
            break;
        }
    }
    let (apk, sha) = found.ok_or_else(|| {
        JidrError::usage(format!(
            "the Magisk {version} APK isn't in the download cache; run `root.set magisk` again"
        ))
    })?;
    ctx.report(None, "Patching the ramdisk with the build props");
    let patched = ensure_patched(ctx, device, &version, &apk, &sha).await?;
    ctx.daemon
        .update_device(
            device.id,
            Box::new(move |d| {
                d.root.patched_ramdisk_path = Some(patched.clone());
                d.launch.ramdisk = Some(patched);
                d.launch.force_cold_boot = true;
            }),
        )
        .await
}

/// Stops the device if it is running, then cold-boots it. The stored
/// `launch.ramdisk` points at the patched copy, so this boots Magisk.
async fn cold_boot(ctx: &CallCtx, device: &Device) -> JidrResult<Device> {
    if device.status.is_active() {
        ctx.report(None, "Stopping before the cold boot");
        ctx.daemon.stop_device(device.id).await?;
    }
    ctx.report(None, "Cold booting");
    ctx.daemon.start_device(device.id, true, &ctx.cancel).await
}

/// Switches a device to Magisk mode: fetch, patch, set state, cold boot, then
/// finish the setup (app install, environment, one reboot, shell grant,
/// verify). On any failure the health is set to `failed` and the device is
/// left in a state revert can recover from.
pub(crate) async fn to_magisk(
    ctx: &CallCtx,
    device: &Device,
    magisk_sha256: Option<&str>,
) -> JidrResult<Value> {
    let changed = device.root.mode != RootMode::Magisk;
    match to_magisk_inner(ctx, device, changed, magisk_sha256).await {
        Ok(v) => Ok(v),
        Err(e) => {
            let _ = ctx
                .daemon
                .update_device(device.id, Box::new(|d| d.root.health = RootHealth::Failed))
                .await;
            Err(e)
        }
    }
}

async fn to_magisk_inner(
    ctx: &CallCtx,
    device: &Device,
    changed: bool,
    magisk_sha256: Option<&str>,
) -> JidrResult<Value> {
    let pin = pins::default_pin()?;
    let adb_root_allowed = device.image.allows_adb_root();

    ctx.report(None, "Resolving the Magisk release");
    let (apk, apk_sha, version) = resolve_apk(ctx, &pin, magisk_sha256).await?;

    let patched = ensure_patched(ctx, device, &version, &apk, &apk_sha).await?;
    let device = ctx
        .daemon
        .update_device(
            device.id,
            Box::new(move |d| {
                d.root.mode = RootMode::Magisk;
                d.root.magisk_version = Some(version);
                d.root.patched_ramdisk_path = Some(patched.clone());
                d.root.health = RootHealth::Unverified;
                d.root.verified_at = None;
                d.launch.ramdisk = Some(patched);
                d.launch.force_cold_boot = true;
            }),
        )
        .await?;

    let device = cold_boot(ctx, &device).await?;

    // Install the manager app (the release APK is also a valid APK).
    ctx.report(None, "Installing the Magisk app");
    verify::device_adb(ctx, &device)
        .await?
        .with_timeout(std::time::Duration::from_secs(600))
        .install(
            &apk,
            InstallOptions {
                replace: true,
                ..Default::default()
            },
        )
        .await?;

    // Environment setup: unpack the app's files into /data/adb/magisk. On
    // images that allow adb root this is headless; Play Store defers to the
    // app (the guided tap below covers it).
    if adb_root_allowed {
        ctx.report(None, "Setting up Magisk's environment");
        let files = env::environment_files(&apk, &device.image.abi)?;
        let adb = verify::device_adb(ctx, &device).await?;
        grant::ensure_adbd_root(&adb).await?;
        let pushed = env::push_environment(&adb, &files).await;
        let _ = grant::drop_adbd_root(&adb).await;
        pushed?;
    } else {
        ctx.report(
            None,
            "Play Store: open the Magisk app to finish its setup, then grant root to Shell",
        );
    }

    // One reboot so magiskd re-reads the environment and enables su.
    ctx.report(None, "Rebooting");
    let device = cold_boot(ctx, &device).await?;

    // Shell su grant: zero-touch on adb-root images, guided tap on Play Store.
    ctx.report(None, "Granting the adb shell su access");
    let adb = verify::device_adb(ctx, &device).await?;
    grant::grant_shell(ctx, &adb, adb_root_allowed).await?;

    ctx.report(None, "Verifying root");
    let verification = verify::verify(ctx, &device).await?;
    Ok(json!({
        "device": device.id,
        "mode": "magisk",
        "changed": changed,
        "applied": "live",
        "magisk_version": verification.magisk_version,
        "verification": verification,
        "beta": !adb_root_allowed,
        "play_integrity": crate::PLAY_INTEGRITY_NOTICE,
    }))
}

/// Clears a Magisk patch: delete the patched file, clear `launch.ramdisk` and
/// the Magisk fields, set `target` as the new mode and cold-boot if running.
pub(crate) async fn clear_patch(
    ctx: &CallCtx,
    device: &Device,
    target: RootMode,
) -> JidrResult<Device> {
    if let Some(patched) = &device.root.patched_ramdisk_path {
        let _ = std::fs::remove_file(patched);
        let _ = std::fs::remove_file(patch::provenance_path(patched));
    }
    // Build props only apply on Full root (Phase 9f); like auto-grant, they
    // don't survive leaving it.
    crate::props::clear(ctx, device);
    let device = ctx
        .daemon
        .update_device(
            device.id,
            Box::new(move |d| {
                d.root.mode = target;
                d.root.magisk_version = None;
                d.root.patched_ramdisk_path = None;
                // D22: leaving Full root also drops "auto-grant all".
                d.root.auto_grant_all = false;
                d.root.health = RootHealth::Unverified;
                d.root.verified_at = None;
                d.launch.ramdisk = None;
                d.launch.force_cold_boot = true;
            }),
        )
        .await?;
    if device.status.is_active() {
        cold_boot(ctx, &device).await
    } else {
        Ok(device)
    }
}

/// The boot hook: for a Magisk device, rebuild the patched ramdisk for the
/// next boot when the pristine image or the pinned version changed. Best
/// effort — it never fails the boot, because the current boot already worked.
pub(crate) async fn on_booted(ctx: &CallCtx, device: &Device) -> JidrResult<()> {
    let Ok(pin) = pins::default_pin() else {
        return Ok(());
    };
    // Re-patch only when the pinned APK is already cached; never fetch here
    // (a boot must not block on, or be slowed by, the network).
    let cache = DownloadCache::new(ctx.daemon.paths().downloads());
    let Ok(Some(apk)) = cache.get_verified(&pin.sha256) else {
        return Ok(());
    };
    let version = pin.version.clone();
    let Ok(patched) = ensure_patched(ctx, device, &version, &apk, &pin.sha256).await else {
        return Ok(());
    };
    // Force the next start to be a cold boot only when the patch actually
    // changed under us (a new image hash or a Magisk upgrade).
    if device.root.magisk_version.as_deref() != Some(version.as_str())
        || device.root.patched_ramdisk_path.as_deref() != Some(patched.as_path())
    {
        let _ = ctx
            .daemon
            .update_device(
                device.id,
                Box::new(move |d| {
                    d.root.magisk_version = Some(version);
                    d.root.patched_ramdisk_path = Some(patched.clone());
                    d.launch.ramdisk = Some(patched);
                    d.launch.force_cold_boot = true;
                }),
            )
            .await;
    }

    // Apply the stored "auto-grant all" flag (D22). Best-effort: on the very
    // first boot of a fresh setup the Magisk app is not installed yet, and a
    // boot must never fail over a preference write.
    if let Ok(adb) = verify::device_adb(ctx, device).await {
        let _ = auto_grant::apply(&adb, device.root.auto_grant_all).await;
    }
    Ok(())
}

/// `root.module.install`: push a module zip, install it through Magisk, reboot
/// and verify. Only available on Magisk devices.
pub(crate) async fn install_module(
    ctx: &CallCtx,
    device: &Device,
    path: &Path,
) -> JidrResult<Value> {
    if device.root.mode != RootMode::Magisk {
        return Err(JidrError::usage(
            "root.module.install needs a Full-root (Magisk) device",
        ));
    }
    if !path.is_file() {
        return Err(JidrError::usage(format!(
            "no such module zip: {}",
            path.display()
        )));
    }
    let adb = verify::device_adb(ctx, device).await?;
    let remote = "/data/local/tmp/jidr-module.zip";
    ctx.report(None, "Pushing the module");
    adb.push(path, remote).await?;

    ctx.report(None, "Installing the module");
    let adb_root_allowed = device.image.allows_adb_root();
    if adb_root_allowed {
        grant::ensure_adbd_root(&adb).await?;
    }
    let result = adb
        .shell(&format!("su -c 'magisk --install-module {remote}'"))
        .await;
    if adb_root_allowed {
        let _ = grant::drop_adbd_root(&adb).await;
    }
    result.map_err(|e| JidrError::root_failed(format!("installing the module: {e}")))?;

    ctx.report(None, "Rebooting");
    let device = cold_boot(ctx, device).await?;
    let verification = verify::verify(ctx, &device).await?;
    Ok(json!({
        "device": device.id,
        "installed": true,
        "verification": verification,
    }))
}
