//! Apps on the device: inspect an APK, launch, uninstall. The player uses
//! these after a dropped APK installs (`file.install` is a core command).

use std::path::PathBuf;

use jidr_core::provider::{CallCtx, CommandSpec, parse_params};
use jidr_core::{JidrError, JidrResult};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::apk;
use crate::device::target;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectParams {
    /// Path of the APK on this machine.
    pub apk: PathBuf,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LaunchParams {
    pub device: String,
    /// Package to launch, e.g. `com.example.shop`.
    #[serde(default)]
    pub package: Option<String>,
    /// Or an APK on this machine whose package to launch.
    #[serde(default)]
    pub apk: Option<PathBuf>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageParams {
    pub device: String,
    /// Package name, e.g. `com.example.shop`.
    pub package: String,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new::<InspectParams>("app.inspect", "Package name of an APK on this machine"),
        CommandSpec::new::<LaunchParams>(
            "app.launch",
            "Launch an installed app by package (or APK)",
        ),
        CommandSpec::new::<PackageParams>("app.uninstall", "Uninstall an app and its data")
            .destructive()
            .locks_device(),
    ]
}

/// A package name as `pm`/`monkey` accept it (no shell metacharacters).
pub fn check_package(p: &str) -> JidrResult<&str> {
    let ok = !p.is_empty()
        && p.split('.').all(|seg| {
            !seg.is_empty() && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
    if ok {
        Ok(p)
    } else {
        Err(JidrError::usage(format!("not a package name: {p:?}")))
    }
}

pub async fn call(ctx: &CallCtx, method: &str, params: Value) -> JidrResult<Value> {
    match method {
        "app.inspect" => {
            let p: InspectParams = parse_params(params)?;
            Ok(json!({ "package": apk::package_name(&p.apk)? }))
        }
        "app.launch" => {
            let p: LaunchParams = parse_params(params)?;
            let package = match (p.package, p.apk) {
                (Some(pkg), _) => pkg,
                (None, Some(apk)) => apk::package_name(&apk)?,
                (None, None) => return Err(JidrError::usage("give `package` or `apk`")),
            };
            check_package(&package)?;
            let t = target(ctx, &p.device).await?;
            let (code, out) = t
                .adb()
                .shell_status(&format!(
                    "monkey -p {package} -c android.intent.category.LAUNCHER 1"
                ))
                .await?;
            if code != 0 || out.contains("No activities found") || out.contains("monkey aborted") {
                return Err(JidrError::not_found(format!(
                    "{package} has no launcher activity on {}",
                    t.device.name
                )));
            }
            Ok(json!({ "launched": package }))
        }
        "app.uninstall" => {
            let p: PackageParams = parse_params(params)?;
            check_package(&p.package)?;
            let t = target(ctx, &p.device).await?;
            t.adb().uninstall(&p.package).await?;
            Ok(json!({ "uninstalled": p.package }))
        }
        _ => Err(JidrError::usage(format!("unknown method {method}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names_are_checked() {
        assert!(check_package("com.example.shop").is_ok());
        assert!(check_package("a_b.c1").is_ok());
        for bad in ["", "com..x", "com.x;rm -rf", "com.x y", ".a"] {
            assert!(check_package(bad).is_err(), "{bad}");
        }
    }
}
