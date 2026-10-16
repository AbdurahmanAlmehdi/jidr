//! Build-prop spoofing (Phase 9f, R5): the identity properties a Full-root
//! device reports (`Build.MODEL`, `BRAND`, `FINGERPRINT`, …).
//!
//! `-prop` only reaches `qemu.*` (disproven in the R5 proof run), so the
//! values are applied by Magisk's `resetprop` from the per-device patched
//! ramdisk. Magisk's documented overlay system appends `overlay.d/*.rc` to
//! `init.rc` (with `${MAGISKTMP}` replaced by its tmpfs) and copies
//! `overlay.d/sbin/*` into that tmpfs. Our rc runs a small script at
//! `post-fs-data`, before zygote starts, which calls `resetprop -n` once per
//! property (the docs' replacement for `setprop` at that stage). Un-patching
//! boots the stock ramdisk, so the stock values come back.
//!
//! Identity properties only: never hardware or HAL properties.

use std::path::PathBuf;

use jidr_core::provider::CallCtx;
use jidr_core::{Device, DeviceStatus, ErrorCode, JidrError, JidrResult, RootMode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::magisk;

/// The values a device reports instead of its image's. `None` keeps stock.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BuildProps {
    /// `Build.MODEL` (`ro.product.model`), e.g. "Pixel 8".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `Build.BRAND` (`ro.product.brand`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    /// `Build.MANUFACTURER` (`ro.product.manufacturer`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    /// `Build.DEVICE` (`ro.product.device`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// `Build.FINGERPRINT` (`ro.build.fingerprint`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// `Build.VERSION.SECURITY_PATCH` (`ro.build.version.security_patch`),
    /// `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security_patch: Option<String>,
}

/// The field names, in form order.
pub const FIELDS: [&str; 6] = [
    "model",
    "brand",
    "manufacturer",
    "device",
    "fingerprint",
    "security_patch",
];

/// Longest value accepted. `ro.*` values may exceed the 91-byte limit of
/// other properties; fingerprints often do.
const MAX_LEN: usize = 256;

/// The partitions whose copies of the product properties Android also reads
/// (`ro.product.<partition>.model`, …); `""` is the plain property.
const PRODUCT_PARTITIONS: [&str; 6] = ["", "system.", "vendor.", "product.", "odm.", "system_ext."];
const BUILD_PARTITIONS: [&str; 6] = [
    "system.",
    "vendor.",
    "product.",
    "odm.",
    "system_ext.",
    "bootimage.",
];

impl BuildProps {
    pub fn get(&self, field: &str) -> Option<&str> {
        match field {
            "model" => self.model.as_deref(),
            "brand" => self.brand.as_deref(),
            "manufacturer" => self.manufacturer.as_deref(),
            "device" => self.device.as_deref(),
            "fingerprint" => self.fingerprint.as_deref(),
            "security_patch" => self.security_patch.as_deref(),
            _ => None,
        }
    }

    fn fields_mut(&mut self) -> [(&'static str, &mut Option<String>); 6] {
        [
            ("model", &mut self.model),
            ("brand", &mut self.brand),
            ("manufacturer", &mut self.manufacturer),
            ("device", &mut self.device),
            ("fingerprint", &mut self.fingerprint),
            ("security_patch", &mut self.security_patch),
        ]
    }

    pub fn is_empty(&self) -> bool {
        FIELDS.iter().all(|f| self.get(f).is_none())
    }

    /// Trims every value (an empty one means stock) and checks it. The error
    /// is `usage` with `data.field` naming the first bad field.
    pub fn normalized(mut self) -> JidrResult<Self> {
        for (name, value) in self.fields_mut() {
            *value = value
                .take()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty());
            if let Some(v) = value.as_deref() {
                check(name, v)?;
            }
        }
        Ok(self)
    }

    /// Every property to set, in a stable order.
    pub fn properties(&self) -> Vec<(String, &str)> {
        let mut out = Vec::new();
        for (field, key) in [
            ("model", "model"),
            ("brand", "brand"),
            ("manufacturer", "manufacturer"),
            ("device", "device"),
        ] {
            if let Some(v) = self.get(field) {
                for p in PRODUCT_PARTITIONS {
                    out.push((format!("ro.product.{p}{key}"), v));
                }
            }
        }
        if let Some(v) = self.fingerprint.as_deref() {
            out.push(("ro.build.fingerprint".to_string(), v));
            for p in BUILD_PARTITIONS {
                out.push((format!("ro.{p}build.fingerprint"), v));
            }
        }
        if let Some(v) = self.security_patch.as_deref() {
            out.push(("ro.build.version.security_patch".to_string(), v));
            out.push(("ro.vendor.build.security_patch".to_string(), v));
        }
        out
    }

    /// The boot script copied into Magisk's tmpfs: one `resetprop -n` per
    /// property. `$1` is that tmpfs (the rc passes `${MAGISKTMP}`), where
    /// `magisk` lives.
    pub fn script(&self) -> String {
        let mut s = String::from("#!/system/bin/sh\n# Jidr build props (Phase 9f).\n");
        for (name, value) in self.properties() {
            s.push_str(&format!(
                "\"$1/magisk\" resetprop -n {name} {}\n",
                quote(value)
            ));
        }
        s
    }
}

/// The `overlay.d` rc that runs [`BuildProps::script`] at `post-fs-data`,
/// before zygote (and so every app) reads the properties.
pub fn rc() -> String {
    format!(
        "# Jidr build props (Phase 9f).\n\
         on post-fs-data\n    \
         exec u:r:magisk:s0 root root -- /system/bin/sh ${{MAGISKTMP}}/{SCRIPT} ${{MAGISKTMP}}\n"
    )
}

/// Names inside `overlay.d` (the script goes under `overlay.d/sbin`).
pub const RC: &str = "jidr-props.rc";
pub const SCRIPT: &str = "jidr-props.sh";

/// Single-quotes `v` for `sh`.
fn quote(v: &str) -> String {
    format!("'{}'", v.replace('\'', r"'\''"))
}

fn check(field: &str, v: &str) -> JidrResult<()> {
    let bad = |why: &str| {
        Err(JidrError::usage(format!("{field}: {why}")).with_data(json!({ "field": field })))
    };
    if v.chars().count() > MAX_LEN {
        return bad(&format!("at most {MAX_LEN} characters"));
    }
    if v.chars().any(char::is_control) {
        return bad("no line breaks or control characters");
    }
    if field == "security_patch" && !is_date(v) {
        return bad("a date like 2026-10-05");
    }
    Ok(())
}

/// `YYYY-MM-DD` with a plausible month and day.
fn is_date(v: &str) -> bool {
    let b = v.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || !digits(0..4)
        || !digits(5..7)
        || !digits(8..10)
    {
        return false;
    }
    let month = (b[5] - b'0') * 10 + (b[6] - b'0');
    let day = (b[8] - b'0') * 10 + (b[9] - b'0');
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

/// `<avd>/jidr/props.json`, next to the patched ramdisk.
pub fn path(ctx: &CallCtx, device: &Device) -> PathBuf {
    ctx.daemon
        .paths()
        .avd_home()
        .join(format!("{}.avd", device.avd))
        .join("jidr")
        .join("props.json")
}

/// The stored props; stock when there are none (or the file is unreadable).
pub fn load(ctx: &CallCtx, device: &Device) -> BuildProps {
    std::fs::read_to_string(path(ctx, device))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(ctx: &CallCtx, device: &Device, props: &BuildProps) -> JidrResult<()> {
    let p = path(ctx, device);
    if props.is_empty() {
        return match std::fs::remove_file(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(JidrError::new(
                ErrorCode::Sdk,
                format!("removing {}: {e}", p.display()),
            )),
            _ => Ok(()),
        };
    }
    let text = serde_json::to_vec_pretty(props)
        .map_err(|e| JidrError::unexpected(format!("encoding props: {e}")))?;
    magisk::patch::write_atomic(&p, &text)
}

/// Drops the stored props (leaving Full root).
pub fn clear(ctx: &CallCtx, device: &Device) {
    let _ = std::fs::remove_file(path(ctx, device));
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetParams {
    /// Device name or id (must be in `magisk` mode).
    pub device: String,
    /// The whole set: fields left out (or empty) report the stock value.
    #[serde(default)]
    pub props: BuildProps,
}

/// The result of `device.props.get` / `.set`.
fn view(ctx: &CallCtx, device: &Device, props: &BuildProps) -> Value {
    let running = matches!(device.status, DeviceStatus::Running);
    json!({
        "device": device.id,
        "props": props,
        // Only a Full-root boot applies them.
        "applies": device.root.mode == RootMode::Magisk,
        // Saved since this boot: the next start (a cold boot) applies them.
        "restart_required": running && device.launch.force_cold_boot,
        "path": path(ctx, device),
    })
}

/// `device.props.get`.
pub async fn get(ctx: &CallCtx, selector: &str) -> JidrResult<Value> {
    let device = ctx.daemon.device(selector).await?;
    let props = load(ctx, &device);
    Ok(view(ctx, &device, &props))
}

/// `device.props.set`: Full root only. Stores the set, re-patches the
/// per-device ramdisk and makes the next start a cold boot.
pub async fn set(ctx: &CallCtx, p: SetParams) -> JidrResult<Value> {
    let device = ctx.daemon.device(&p.device).await?;
    if device.root.mode != RootMode::Magisk {
        return Err(JidrError::usage(format!(
            "{} isn't Full-root (Magisk); build props apply to Magisk devices only",
            device.name
        )));
    }
    let props = p.props.normalized()?;
    let before = load(ctx, &device);
    save(ctx, &device, &props)?;
    if props == before {
        return Ok(view(ctx, &device, &props));
    }
    let device = match magisk::repatch(ctx, &device).await {
        Ok(d) => d,
        Err(e) => {
            // Keep what the ramdisk actually carries.
            let _ = save(ctx, &device, &before);
            return Err(e);
        }
    };
    Ok(view(ctx, &device, &props))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props() -> BuildProps {
        BuildProps {
            model: Some("Pixel 8".into()),
            brand: Some("google".into()),
            security_patch: Some("2026-10-05".into()),
            ..Default::default()
        }
    }

    #[test]
    fn empty_values_mean_stock_and_values_are_trimmed() {
        let p = BuildProps {
            model: Some("  Pixel 8 ".into()),
            brand: Some("   ".into()),
            ..Default::default()
        }
        .normalized()
        .unwrap();
        assert_eq!(p.model.as_deref(), Some("Pixel 8"));
        assert_eq!(p.brand, None);
        assert!(BuildProps::default().normalized().unwrap().is_empty());
    }

    #[test]
    fn bad_values_name_their_field() {
        for (p, field) in [
            (
                BuildProps {
                    security_patch: Some("Oct 2026".into()),
                    ..Default::default()
                },
                "security_patch",
            ),
            (
                BuildProps {
                    security_patch: Some("2026-13-01".into()),
                    ..Default::default()
                },
                "security_patch",
            ),
            (
                BuildProps {
                    model: Some("a\nb".into()),
                    ..Default::default()
                },
                "model",
            ),
            (
                BuildProps {
                    fingerprint: Some("x".repeat(MAX_LEN + 1)),
                    ..Default::default()
                },
                "fingerprint",
            ),
        ] {
            let e = p.normalized().unwrap_err();
            assert_eq!(e.code, ErrorCode::Usage);
            assert_eq!(e.data.unwrap()["field"], field);
        }
    }

    #[test]
    fn each_field_sets_its_property_and_partition_copies() {
        let names: Vec<String> = props().properties().into_iter().map(|(n, _)| n).collect();
        assert!(names.contains(&"ro.product.model".to_string()));
        assert!(names.contains(&"ro.product.vendor.model".to_string()));
        assert!(names.contains(&"ro.product.system.brand".to_string()));
        assert!(names.contains(&"ro.build.version.security_patch".to_string()));
        assert!(!names.iter().any(|n| n.contains("manufacturer")));
        assert!(!names.iter().any(|n| n.contains("hardware")));
        let fp = BuildProps {
            fingerprint: Some("google/husky/husky:15/AP4A/1:user/release-keys".into()),
            ..Default::default()
        };
        let names: Vec<String> = fp.properties().into_iter().map(|(n, _)| n).collect();
        assert!(names.contains(&"ro.build.fingerprint".to_string()));
        assert!(names.contains(&"ro.vendor.build.fingerprint".to_string()));
        assert!(names.contains(&"ro.bootimage.build.fingerprint".to_string()));
    }

    #[test]
    fn the_script_quotes_values_and_uses_the_magisk_tmpfs() {
        let p = BuildProps {
            model: Some("Bob's \"phone\" $HOME".into()),
            ..Default::default()
        };
        let s = p.script();
        assert!(s.starts_with("#!/system/bin/sh\n"));
        assert!(
            s.contains(r#""$1/magisk" resetprop -n ro.product.model 'Bob'\''s "phone" $HOME'"#),
            "{s}"
        );
        let rc = rc();
        assert!(rc.contains("on post-fs-data\n"));
        assert!(
            rc.contains("${MAGISKTMP}/jidr-props.sh ${MAGISKTMP}"),
            "{rc}"
        );
        assert!(!rc.contains("/sbin/"), "never hardcode /sbin: {rc}");
    }

    #[test]
    fn dates() {
        assert!(is_date("2026-10-05"));
        assert!(!is_date("2026-1-05"));
        assert!(!is_date("2026-00-05"));
        assert!(!is_date("2026-10-32"));
    }
}
