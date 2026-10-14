//! Snapshot manager (Phase 10b): the emulator's `SnapshotService` plus a
//! root-mode tag per snapshot, kept in `<avd>/jidr/snapshots.json`. A
//! snapshot taken under one root mode is refused under another (D29).
//!
//! Jidr picks the on-disk id itself ([`snapshot_id`]): emulator builds differ
//! in how they store names with spaces, so the name the user typed is kept as
//! the tag's `label`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use jidr_core::provider::{CallCtx, CommandSpec, DeviceParams, parse_params};
use jidr_core::{Device, DeviceStatus, JidrError, JidrResult, RootMode};
use jidr_proto::control as pb;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::device::{grpc_err, target};

/// The emulator's own quick-boot snapshot (D29), saved on every stop.
pub const QUICK_BOOT: &str = "default_boot";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tag {
    /// The name as typed, when it differs from the id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub root_mode: RootMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magisk_version: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NameParams {
    pub device: String,
    /// Snapshot name.
    pub name: String,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new::<DeviceParams>(
            "snapshot.list",
            "Snapshots with their root-mode tags, sizes and thumbnails",
        ),
        CommandSpec::new::<NameParams>("snapshot.save", "Save a snapshot of a running device")
            .long_running()
            .locks_device(),
        CommandSpec::new::<NameParams>(
            "snapshot.load",
            "Restore a snapshot (refused when it was taken under another root mode)",
        )
        .long_running()
        .locks_device(),
        CommandSpec::new::<NameParams>("snapshot.delete", "Delete a snapshot")
            .destructive()
            .locks_device(),
    ]
}

fn avd_dir(ctx: &CallCtx, d: &Device) -> PathBuf {
    ctx.daemon.paths().avd_home().join(format!("{}.avd", d.avd))
}

fn tags_path(avd: &Path) -> PathBuf {
    avd.join("jidr").join("snapshots.json")
}

pub fn read_tags(avd: &Path) -> BTreeMap<String, Tag> {
    std::fs::read_to_string(tags_path(avd))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_tags(avd: &Path, tags: &BTreeMap<String, Tag>) -> JidrResult<()> {
    let path = tags_path(avd);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| JidrError::unexpected(format!("{}: {e}", dir.display())))?;
    }
    let tmp = path.with_extension("json.tmp");
    let text =
        serde_json::to_string_pretty(tags).map_err(|e| JidrError::unexpected(e.to_string()))?;
    std::fs::write(&tmp, text)
        .and_then(|()| std::fs::rename(&tmp, &path))
        .map_err(|e| JidrError::unexpected(format!("{}: {e}", path.display())))
}

/// Names users may give: no path separators or control characters.
pub fn check_name(n: &str) -> JidrResult<&str> {
    let n = n.trim();
    if n.is_empty()
        || n.len() > 100
        || n.contains(['/', '\\', ':'])
        || n.chars().any(char::is_control)
        || n.starts_with('.')
    {
        return Err(JidrError::usage(format!("not a snapshot name: {n:?}")));
    }
    if n == QUICK_BOOT {
        return Err(JidrError::usage(
            "default_boot is the quick-boot snapshot; pick another name",
        ));
    }
    Ok(n)
}

/// The snapshot's folder name: ASCII letters, digits, `-`, `.` and `_` stay,
/// other ASCII becomes `_`, and other characters become `x<hex>` so names in
/// other scripts stay distinct.
pub fn snapshot_id(name: &str) -> String {
    let mut id = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_') {
            id.push(c);
        } else if c.is_ascii() {
            id.push('_');
        } else {
            id.push_str(&format!("x{:x}", u32::from(c)));
        }
    }
    id
}

/// The id to save `name` under: [`snapshot_id`], with `-2`, `-3`… when
/// another snapshot already holds that id. Saving the same name again
/// replaces it.
fn free_id(avd: &Path, tags: &BTreeMap<String, Tag>, name: &str) -> String {
    let base = snapshot_id(name);
    let mut id = base.clone();
    for n in 2.. {
        let taken = match tags.get(&id) {
            Some(t) => t.label.as_deref().unwrap_or(&id) != name,
            None => id != name && avd.join("snapshots").join(&id).is_dir(),
        };
        if !taken {
            break;
        }
        id = format!("{base}-{n}");
    }
    id
}

/// The id of the snapshot a user or client named: a tag's label, an existing
/// folder, or the id Jidr would give that name.
pub fn resolve(avd: &Path, tags: &BTreeMap<String, Tag>, name: &str) -> String {
    if let Some((id, _)) = tags.iter().find(|(_, t)| t.label.as_deref() == Some(name)) {
        return id.clone();
    }
    let safe = !name.is_empty() && !name.contains(['/', '\\']) && !name.starts_with('.');
    if safe && avd.join("snapshots").join(name).is_dir() {
        return name.to_string();
    }
    snapshot_id(name)
}

/// D29: may this snapshot be restored on a device in `mode`? Untagged
/// snapshots (made outside Jidr) and the quick-boot one restore as usual.
pub fn check_root(name: &str, tag: Option<&Tag>, mode: RootMode) -> JidrResult<()> {
    match tag {
        Some(t) if t.root_mode != mode => Err(JidrError::usage(format!(
            "“{name}” was taken with {} and the device is {} now; set the root mode back first",
            mode_name(t.root_mode),
            mode_name(mode)
        ))
        .with_data(json!({ "refused": "root_mode", "snapshot_root_mode": t.root_mode, "device_root_mode": mode }))),
        _ => Ok(()),
    }
}

pub fn mode_name(m: RootMode) -> &'static str {
    match m {
        RootMode::None => "Not rooted",
        RootMode::Adb => "Shell root",
        RootMode::Magisk => "Full root",
    }
}

fn dir_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

/// A snapshot folder: name, size, created, thumbnail.
type DiskSnapshot = (String, u64, Option<DateTime<Utc>>, Option<PathBuf>);

/// Snapshots on disk: `<avd>/snapshots/<name>/` (works while stopped).
fn on_disk(avd: &Path) -> Vec<DiskSnapshot> {
    let dir = avd.join("snapshots");
    let mut v: Vec<_> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            let created = std::fs::metadata(p.join("snapshot.pb"))
                .and_then(|m| m.modified())
                .ok()
                .map(DateTime::<Utc>::from);
            let thumb = Some(p.join("screenshot.png")).filter(|t| t.is_file());
            (name, dir_size(&p), created, thumb)
        })
        .collect();
    v.sort_by_key(|s| s.2);
    v
}

fn failed(what: &str, pkg: &pb::SnapshotPackage) -> JidrResult<()> {
    if pkg.success {
        Ok(())
    } else {
        Err(JidrError::sdk(format!(
            "{what}: {}",
            String::from_utf8_lossy(&pkg.err).trim()
        )))
    }
}

pub async fn call(ctx: &CallCtx, method: &str, params: Value) -> JidrResult<Value> {
    match method {
        "snapshot.list" => {
            let p: DeviceParams = parse_params(params)?;
            let d = ctx.daemon.device(&p.device).await?;
            let avd = avd_dir(ctx, &d);
            let tags = read_tags(&avd);
            let list: Vec<Value> = on_disk(&avd)
                .into_iter()
                .map(|(id, size, created, thumb)| {
                    let tag = tags.get(&id);
                    let name = tag
                        .and_then(|t| t.label.clone())
                        .unwrap_or_else(|| id.clone());
                    json!({
                        "name": name,
                        "id": id,
                        "quick_boot": id == QUICK_BOOT,
                        "root_mode": tag.map(|t| t.root_mode),
                        "magisk_version": tag.and_then(|t| t.magisk_version.clone()),
                        "created_at": tag.map(|t| t.created_at).or(created),
                        "size_bytes": size,
                        "thumbnail": thumb,
                        "restorable": check_root(&name, tag, d.root.mode).is_ok(),
                    })
                })
                .collect();
            Ok(json!({ "device_root_mode": d.root.mode, "snapshots": list }))
        }
        "snapshot.save" => {
            let p: NameParams = parse_params(params)?;
            let name = check_name(&p.name)?.to_string();
            let t = target(ctx, &p.device).await?;
            let avd = avd_dir(ctx, &t.device);
            let mut tags = read_tags(&avd);
            let id = free_id(&avd, &tags, &name);
            if id == QUICK_BOOT {
                return Err(JidrError::usage(
                    "default_boot is the quick-boot snapshot; pick another name",
                ));
            }
            ctx.report(None, "Saving memory and disk");
            let r = t
                .snapshots()
                .await?
                .save_snapshot(pb::SnapshotPackage {
                    snapshot_id: id.clone(),
                    ..Default::default()
                })
                .await
                .map_err(grpc_err("SaveSnapshot"))?
                .into_inner();
            failed("saving the snapshot", &r)?;
            tags.insert(
                id.clone(),
                Tag {
                    label: Some(name.clone()).filter(|n| *n != id),
                    root_mode: t.device.root.mode,
                    magisk_version: t.device.root.magisk_version.clone(),
                    created_at: Utc::now(),
                },
            );
            write_tags(&avd, &tags)?;
            Ok(json!({ "name": name, "id": id, "root_mode": t.device.root.mode }))
        }
        "snapshot.load" => {
            let p: NameParams = parse_params(params)?;
            let d = ctx.daemon.device(&p.device).await?;
            let avd = avd_dir(ctx, &d);
            let tags = read_tags(&avd);
            let id = resolve(&avd, &tags, &p.name);
            check_root(&p.name, tags.get(&id), d.root.mode)?;
            if !matches!(d.status, DeviceStatus::Running) {
                return Err(JidrError::not_found(format!(
                    "start {} to restore a snapshot",
                    d.name
                )));
            }
            let t = target(ctx, &p.device).await?;
            ctx.report(None, "Restoring");
            let r = t
                .snapshots()
                .await?
                .load_snapshot(pb::SnapshotPackage {
                    snapshot_id: id,
                    ..Default::default()
                })
                .await
                .map_err(grpc_err("LoadSnapshot"))?
                .into_inner();
            failed("restoring the snapshot", &r)?;
            Ok(json!({ "name": p.name }))
        }
        "snapshot.delete" => {
            let p: NameParams = parse_params(params)?;
            let d = ctx.daemon.device(&p.device).await?;
            let avd = avd_dir(ctx, &d);
            let mut tags = read_tags(&avd);
            let id = resolve(&avd, &tags, &p.name);
            if matches!(d.status, DeviceStatus::Running) {
                let t = target(ctx, &p.device).await?;
                let r = t
                    .snapshots()
                    .await?
                    .delete_snapshot(pb::SnapshotPackage {
                        snapshot_id: id.clone(),
                        ..Default::default()
                    })
                    .await
                    .map_err(grpc_err("DeleteSnapshot"))?
                    .into_inner();
                failed("deleting the snapshot", &r)?;
            } else {
                let dir = avd.join("snapshots").join(&id);
                if !dir.starts_with(avd.join("snapshots")) || !dir.is_dir() {
                    return Err(JidrError::not_found(format!("no snapshot {:?}", p.name)));
                }
                std::fs::remove_dir_all(&dir)
                    .map_err(|e| JidrError::unexpected(format!("{}: {e}", dir.display())))?;
            }
            if tags.remove(&id).is_some() {
                write_tags(&avd, &tags)?;
            }
            Ok(json!({ "deleted": p.name }))
        }
        _ => Err(JidrError::usage(format!("unknown method {method}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(m: RootMode) -> Tag {
        Tag {
            label: None,
            root_mode: m,
            magisk_version: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn cross_mode_restore_is_refused() {
        assert!(
            check_root(
                "Clean install",
                Some(&tag(RootMode::Magisk)),
                RootMode::Magisk
            )
            .is_ok()
        );
        let e = check_root(
            "Clean install",
            Some(&tag(RootMode::Magisk)),
            RootMode::None,
        )
        .unwrap_err();
        assert!(
            e.message.contains("Full root") && e.message.contains("Not rooted"),
            "{e}"
        );
        assert_eq!(e.data.unwrap()["refused"], "root_mode");
        assert!(check_root("old", None, RootMode::Adb).is_ok());
    }

    #[test]
    fn names_are_checked() {
        assert!(check_name("Logged in test account").is_ok());
        for bad in ["", "a/b", "../x", ".hidden", "default_boot", "a\nb"] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
    }

    /// Regression: the Linux emulator stored "Clean install" as
    /// `Clean_install`, so the tag keyed by the typed name went missing.
    #[test]
    fn ids_are_folder_safe_and_names_resolve_through_labels() {
        assert_eq!(snapshot_id("Clean install"), "Clean_install");
        assert_eq!(snapshot_id("v1.2-beta_3"), "v1.2-beta_3");
        assert_eq!(snapshot_id("لقطة"), "x644x642x637x629");
        assert_ne!(snapshot_id("لقطة"), snapshot_id("قطة"));
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("snapshots/made outside")).unwrap();
        let mut t = BTreeMap::new();
        t.insert(
            "Clean_install".to_string(),
            Tag {
                label: Some("Clean install".into()),
                ..tag(RootMode::None)
            },
        );
        assert_eq!(resolve(tmp.path(), &t, "Clean install"), "Clean_install");
        assert_eq!(resolve(tmp.path(), &t, "made outside"), "made outside");
        assert_eq!(resolve(tmp.path(), &t, "Logged in"), "Logged_in");
        assert_eq!(resolve(tmp.path(), &t, "../x"), ".._x");

        // Saving the same name replaces it; a different name gets a suffix.
        assert_eq!(free_id(tmp.path(), &t, "Clean install"), "Clean_install");
        assert_eq!(free_id(tmp.path(), &t, "Clean_install"), "Clean_install-2");
        assert_eq!(free_id(tmp.path(), &t, "made outside"), "made_outside");
    }

    #[test]
    fn tags_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut t = BTreeMap::new();
        t.insert("Clean install".to_string(), tag(RootMode::Adb));
        write_tags(tmp.path(), &t).unwrap();
        assert_eq!(read_tags(tmp.path()), t);
    }
}
