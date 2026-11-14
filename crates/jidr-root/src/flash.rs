//! Flashable zips (ADR-0004): what a zip is, read from its file list on the
//! Mac, and `file.flash`, which installs a Magisk module, copies the zip to
//! Download, or refuses it with the reason.
//!
//! The layout rules come from Magisk's public module guide (`module.prop` at
//! the zip's root, the id rule, `zygisk/<abi>.so`, the recovery
//! `META-INF/com/google/android/update-binary`); see docs/clean-room.md.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use jidr_core::provider::CallCtx;
use jidr_core::{JidrError, JidrResult, RootMode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{ensure_running, magisk, verify};

/// Where `file.flash` copies a zip it doesn't flash (D35).
const DOWNLOAD: &str = "/sdcard/Download/";
/// Module ids Jidr writes itself (`jidr_cacerts`).
const RESERVED_PREFIX: &str = "jidr_";
/// A `module.prop` larger than this isn't one.
const MAX_PROP_BYTES: u64 = 64 * 1024;

const RECOVERY_BINARY: &str = "META-INF/com/google/android/update-binary";

/// What a zip is, by its file list (ADR-0004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZipKind {
    /// `module.prop` at the root: `magisk --install-module`.
    MagiskModule,
    /// A Magisk module with Zygisk libraries (`zygisk/`).
    ZygiskModule,
    /// A Magisk module for Riru (`riru/`, or `riru-core` itself): refused.
    RiruModule,
    /// A recovery (TWRP) zip without `module.prop`: refused.
    Recovery,
    /// An OTA or factory image: refused.
    Image,
    /// Anything else, including files that aren't zips: copied to Download.
    Other,
}

/// The fields of a module's `module.prop` that Jidr reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModuleInfo {
    pub id: String,
    pub name: Option<String>,
    pub version: Option<String>,
    /// Whether it ships Zygisk libraries.
    pub zygisk: bool,
}

/// The type of a zip and, for a module, its `module.prop`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    pub kind: ZipKind,
    pub module: Option<ModuleInfo>,
}

impl Inspection {
    fn other() -> Self {
        Self {
            kind: ZipKind::Other,
            module: None,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlashParams {
    /// Device name or id (must be running).
    pub device: String,
    /// A local zip: a Magisk or Zygisk module installs on a Full-root device;
    /// recovery, Riru and image zips are refused; anything else is copied to
    /// /sdcard/Download.
    pub path: PathBuf,
}

/// Reads a zip's file list (and `module.prop`) and says what it is. Never
/// extracts anything else. A module with a fixable problem is a `usage` error
/// that names the fix; refusals by type are [`refusal`]'s job.
pub fn inspect(path: &Path) -> JidrResult<Inspection> {
    if !path.is_file() {
        return Err(JidrError::usage(format!(
            "no such file: {}",
            path.display()
        )));
    }
    let file = File::open(path)
        .map_err(|e| JidrError::usage(format!("can't read {}: {e}", path.display())))?;
    let Ok(mut zip) = zip::ZipArchive::new(file) else {
        return Ok(Inspection::other());
    };
    let names: Vec<String> = zip
        .file_names()
        .filter_map(|n| n.ok().map(|n| n.into_owned()))
        .collect();
    let has = |name: &str| names.iter().any(|n| n == name);
    let under = |dir: &str| names.iter().any(|n| n.starts_with(dir));
    let file_name = display_name(path);

    if has("module.prop") {
        let prop = read_prop(&mut zip, "module.prop", &file_name)?;
        let id = module_id(&prop, &file_name)?;
        let zygisk = under("zygisk/");
        let kind = if under("riru/") || id == "riru-core" {
            ZipKind::RiruModule
        } else if zygisk {
            ZipKind::ZygiskModule
        } else {
            ZipKind::MagiskModule
        };
        return Ok(Inspection {
            kind,
            module: Some(ModuleInfo {
                name: prop_value(&prop, "name"),
                version: prop_value(&prop, "version"),
                id,
                zygisk,
            }),
        });
    }
    if has(RECOVERY_BINARY) {
        return Ok(Inspection {
            kind: ZipKind::Recovery,
            module: None,
        });
    }
    // A zipped module folder (e.g. GitHub's "Download ZIP"): Magisk only looks
    // at the root and fails with "This zip is not a Magisk module!".
    if let Some(folder) = names.iter().find_map(|n| {
        let (dir, rest) = n.split_once('/')?;
        (rest == "module.prop").then(|| dir.to_string())
    }) {
        return Err(JidrError::usage(format!(
            "{file_name} has module.prop inside {folder}/, not at the top of the zip, so Magisk can't install it. Zip the contents of {folder} instead of the folder itself."
        ))
        .with_data(json!({ "kind": ZipKind::MagiskModule })));
    }
    let image = has("payload.bin")
        || has("META-INF/com/android/metadata")
        || has("android-info.txt")
        || names.iter().any(|n| {
            let top = n.split_once('/').map_or(n.as_str(), |(_, rest)| rest);
            top == "flash-all.sh" || (!n.contains('/') && n.ends_with(".img"))
        });
    Ok(Inspection {
        kind: if image {
            ZipKind::Image
        } else {
            ZipKind::Other
        },
        module: None,
    })
}

/// The `usage` error for a zip type Jidr won't flash (ADR-0004), with
/// `error.data.kind`.
pub fn refusal(kind: ZipKind, path: &Path) -> Option<JidrError> {
    let name = display_name(path);
    let message = match kind {
        ZipKind::Recovery => format!(
            "{name} is a recovery (TWRP) zip: it runs from recovery and writes the system partitions, and the emulator has no recovery. Jidr flashes Magisk modules only. Google APIs images already include Play services, and arm64 images run ARM apps without translation. `jidr push` copies the zip to the device if you need the file there."
        ),
        ZipKind::Image => format!(
            "{name} holds partition images (an OTA or factory image): the emulator has no fastboot or partition flashing, and its partitions are the SDK system image's files. Create a device from another system image instead."
        ),
        ZipKind::RiruModule => format!(
            "{name} is a Riru module: Magisk skips Riru modules while Zygisk is on (its default on emulators), and Riru is discontinued. Use the module's Zygisk build."
        ),
        ZipKind::MagiskModule | ZipKind::ZygiskModule | ZipKind::Other => return None,
    };
    Some(JidrError::usage(message).with_data(json!({ "kind": kind })))
}

/// Checks that `path` is a module Magisk can install, for `root.module.install`
/// and `file.flash`.
pub(crate) fn module_to_install(path: &Path) -> JidrResult<ModuleInfo> {
    let found = inspect(path)?;
    if let Some(err) = refusal(found.kind, path) {
        return Err(err);
    }
    found.module.ok_or_else(|| {
        JidrError::usage(format!(
            "{} isn't a Magisk module: it has no module.prop at the top of the zip",
            display_name(path)
        ))
        .with_data(json!({ "kind": found.kind }))
    })
}

/// `file.flash`: route a zip by its type (ADR-0004).
pub(crate) async fn flash(ctx: &CallCtx, p: FlashParams) -> JidrResult<Value> {
    let device = ctx.daemon.device(&p.device).await?;
    ensure_running(&device)?;
    let found = inspect(&p.path)?;
    if let Some(err) = refusal(found.kind, &p.path) {
        return Err(err);
    }
    if let Some(module) = found
        .module
        .filter(|_| device.root.mode == RootMode::Magisk)
    {
        let mut v = magisk::install_module(ctx, &device, &p.path, &module).await?;
        v["kind"] = json!(found.kind);
        v["action"] = json!("module_installed");
        return Ok(v);
    }

    let note = (found.kind != ZipKind::Other)
        .then_some("Magisk modules need Full root, so it was copied to Download.");
    ctx.report(None, "Copying to Download");
    verify::device_adb(ctx, &device)
        .await?
        .with_timeout(Duration::from_secs(600))
        .push(&p.path, DOWNLOAD)
        .await?;
    Ok(json!({
        "device": device.id,
        "kind": found.kind,
        "action": "pushed",
        "remote": format!("{DOWNLOAD}{}", display_name(&p.path)),
        "note": note,
    }))
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn read_prop(zip: &mut zip::ZipArchive<File>, entry: &str, file_name: &str) -> JidrResult<String> {
    let bad = |e: String| JidrError::usage(format!("can't read {entry} in {file_name}: {e}"));
    let f = zip.by_name(entry).map_err(|e| bad(e.to_string()))?;
    let mut text = String::new();
    f.take(MAX_PROP_BYTES)
        .read_to_string(&mut text)
        .map_err(|e| bad(e.to_string()))?;
    Ok(text)
}

/// The first `key=value` line for `key`, as Magisk's own reader takes it.
fn prop_value(prop: &str, key: &str) -> Option<String> {
    prop.lines().find_map(|line| {
        let (k, v) = line.trim_start_matches('\u{feff}').split_once('=')?;
        (k.trim() == key).then(|| v.trim().to_string())
    })
}

/// The module id, checked against Magisk's documented rule (a letter, then at
/// least two letters, digits, `.`, `_` or `-`). Magisk uses it as a directory
/// name without checking it.
fn module_id(prop: &str, file_name: &str) -> JidrResult<String> {
    let err =
        |msg: String| JidrError::usage(msg).with_data(json!({ "kind": ZipKind::MagiskModule }));
    let Some(id) = prop_value(prop, "id").filter(|id| !id.is_empty()) else {
        return Err(err(format!("{file_name}'s module.prop has no id")));
    };
    let mut chars = id.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && id.len() >= 3
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !valid {
        return Err(err(format!(
            "{file_name}'s module id {id:?} isn't a valid Magisk module id (a letter, then at least two letters, digits, '.', '_' or '-')"
        )));
    }
    if id.starts_with(RESERVED_PREFIX) {
        return Err(err(format!(
            "{file_name}'s module id {id} starts with {RESERVED_PREFIX}, which is reserved for Jidr's own modules"
        )));
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_of(dir: &Path, name: &str, entries: &[(&str, &str)]) -> PathBuf {
        let path = dir.join(name);
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        for (entry, body) in entries {
            if entry.ends_with('/') {
                zip.add_directory(*entry, zip::write::SimpleFileOptions::default())
                    .unwrap();
            } else {
                zip.start_file(*entry, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(body.as_bytes()).unwrap();
            }
        }
        zip.finish().unwrap();
        path
    }

    const PROP: &str =
        "id=demo_mod\nname=Demo\nversion=v1\nversionCode=1\nauthor=me\ndescription=d\n";

    #[test]
    fn module_prop_values_take_the_first_line() {
        assert_eq!(prop_value("id=a\nid=b", "id").as_deref(), Some("a"));
        assert_eq!(prop_value("\u{feff}id=x\r\n", "id").as_deref(), Some("x"));
        assert_eq!(prop_value("name=x", "id"), None);
    }

    #[test]
    fn module_ids_follow_magisks_rule() {
        let ok = |id: &str| module_id(&format!("id={id}"), "m.zip");
        assert_eq!(ok("demo_mod").unwrap(), "demo_mod");
        assert_eq!(ok("a.b-c").unwrap(), "a.b-c");
        assert!(ok("ab").is_err(), "too short");
        assert!(ok("1abc").is_err(), "starts with a digit");
        assert!(ok("../evil").is_err(), "path");
        assert!(ok("a b c").is_err(), "space");
        assert!(module_id("name=x", "m.zip").is_err(), "no id");
        let reserved = ok("jidr_cacerts").unwrap_err();
        assert!(reserved.message.contains("reserved"), "{reserved}");
    }

    #[test]
    fn modules_are_sorted_by_their_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let plain = zip_of(
            tmp.path(),
            "plain.zip",
            &[
                ("META-INF/com/google/android/update-binary", "#!/sbin/sh"),
                ("META-INF/com/google/android/updater-script", "#MAGISK"),
                ("module.prop", PROP),
                ("service.sh", "echo"),
            ],
        );
        let found = inspect(&plain).unwrap();
        assert_eq!(
            found.kind,
            ZipKind::MagiskModule,
            "module wins over update-binary"
        );
        let module = found.module.unwrap();
        assert_eq!(module.id, "demo_mod");
        assert_eq!(module.name.as_deref(), Some("Demo"));
        assert_eq!(module.version.as_deref(), Some("v1"));
        assert!(!module.zygisk);

        let zygisk = zip_of(
            tmp.path(),
            "z.zip",
            &[("module.prop", PROP), ("zygisk/arm64-v8a.so", "elf")],
        );
        let found = inspect(&zygisk).unwrap();
        assert_eq!(found.kind, ZipKind::ZygiskModule);
        assert!(found.module.unwrap().zygisk);

        let riru = zip_of(
            tmp.path(),
            "r.zip",
            &[("module.prop", PROP), ("riru/lib64/libx.so", "elf")],
        );
        assert_eq!(inspect(&riru).unwrap().kind, ZipKind::RiruModule);
        let core = zip_of(tmp.path(), "c.zip", &[("module.prop", "id=riru-core")]);
        assert_eq!(inspect(&core).unwrap().kind, ZipKind::RiruModule);
    }

    #[test]
    fn recovery_image_and_other_zips() {
        let tmp = tempfile::tempdir().unwrap();
        let gapps = zip_of(
            tmp.path(),
            "gapps.zip",
            &[
                ("META-INF/com/google/android/update-binary", "#!/sbin/sh"),
                ("META-INF/com/google/android/updater-script", "ui_print"),
                ("Core/gmscore.tar.xz", "x"),
            ],
        );
        assert_eq!(inspect(&gapps).unwrap().kind, ZipKind::Recovery);

        let ota = zip_of(
            tmp.path(),
            "ota.zip",
            &[("payload.bin", "x"), ("META-INF/com/android/metadata", "x")],
        );
        assert_eq!(inspect(&ota).unwrap().kind, ZipKind::Image);
        let factory = zip_of(
            tmp.path(),
            "factory.zip",
            &[("oriole-x/flash-all.sh", "x"), ("oriole-x/image.zip", "x")],
        );
        assert_eq!(inspect(&factory).unwrap().kind, ZipKind::Image);
        let boot = zip_of(tmp.path(), "boot.zip", &[("boot.img", "x")]);
        assert_eq!(inspect(&boot).unwrap().kind, ZipKind::Image);

        let photos = zip_of(
            tmp.path(),
            "photos.zip",
            &[("a.png", "x"), ("b/c.img", "x")],
        );
        assert_eq!(inspect(&photos).unwrap(), Inspection::other());
        let not_zip = tmp.path().join("fake.zip");
        std::fs::write(&not_zip, b"not a zip").unwrap();
        assert_eq!(inspect(&not_zip).unwrap().kind, ZipKind::Other);
        assert!(inspect(&tmp.path().join("missing.zip")).is_err());
    }

    #[test]
    fn a_zipped_module_folder_names_the_fix() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = zip_of(
            tmp.path(),
            "Mod-main.zip",
            &[("Mod-main/", ""), ("Mod-main/module.prop", PROP)],
        );
        let err = inspect(&nested).unwrap_err();
        assert_eq!(err.code, jidr_core::ErrorCode::Usage);
        assert!(
            err.message.contains("Zip the contents of Mod-main"),
            "{err}"
        );
    }

    #[test]
    fn refusals_explain_and_carry_the_kind() {
        let p = Path::new("/x/gapps.zip");
        for kind in [ZipKind::Recovery, ZipKind::Image, ZipKind::RiruModule] {
            let err = refusal(kind, p).unwrap();
            assert_eq!(err.code, jidr_core::ErrorCode::Usage);
            assert!(err.message.starts_with("gapps.zip"), "{err}");
            assert_eq!(err.data.unwrap()["kind"], json!(kind));
        }
        assert!(
            refusal(ZipKind::Recovery, p)
                .unwrap()
                .message
                .contains("no recovery")
        );
        for kind in [ZipKind::MagiskModule, ZipKind::ZygiskModule, ZipKind::Other] {
            assert!(refusal(kind, p).is_none());
        }
    }
}
