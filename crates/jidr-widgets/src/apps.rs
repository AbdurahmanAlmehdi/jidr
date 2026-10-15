//! Apps on the device: inspect an APK, launch, uninstall. The player uses
//! these after a dropped APK installs (`file.install` is a core command).

use std::path::PathBuf;

use jidr_core::provider::{CallCtx, CommandSpec, parse_params};
use jidr_core::{JidrError, JidrResult};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::apk;
use crate::device::{Target, target};

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

/// The component `cmd package resolve-activity --brief` printed (its last
/// line), or `None` when nothing resolves.
pub fn parse_resolved(out: &str) -> Option<&str> {
    let last = out.lines().map(str::trim).rfind(|l| !l.is_empty())?;
    let ok = last.contains('/')
        && last
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '$'));
    ok.then_some(last)
}

/// `am start -W` succeeded: `Status: ok` (or `Complete` on images that
/// don't print a status). `Err` carries adb's error line.
pub fn parse_am_start(out: &str) -> Result<(), String> {
    let lines: Vec<&str> = out.lines().map(str::trim).collect();
    if let Some(e) = lines.iter().find(|l| l.starts_with("Error")) {
        return Err((*e).to_string());
    }
    if lines.iter().any(|l| *l == "Status: ok" || *l == "Complete") {
        return Ok(());
    }
    Err(lines
        .iter()
        .rfind(|l| !l.is_empty())
        .map_or("am start printed nothing".into(), |l| (*l).to_string()))
}

/// Does `dumpsys window` show `package` in front?
pub fn focused_on(dumpsys: &str, package: &str) -> bool {
    let needle = format!("{package}/");
    dumpsys
        .lines()
        .any(|l| (l.contains("mCurrentFocus") || l.contains("mFocusedApp")) && l.contains(&needle))
}

/// Starts `package`'s launcher activity with `am start -W` and waits up to
/// 5 s for it to come to the front, starting it once more if it doesn't.
/// Falls back to `monkey` when no launcher activity resolves. Returns
/// whether the app was seen in front.
async fn launch(t: &Target, package: &str) -> JidrResult<bool> {
    let adb = t.adb();
    let resolved = adb
        .shell_status(&format!(
            "cmd package resolve-activity --brief -c android.intent.category.LAUNCHER {package}"
        ))
        .await?
        .1;
    let Some(component) = parse_resolved(&resolved).map(str::to_string) else {
        let (code, out) = adb
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
        return Ok(false);
    };
    for _ in 0..2 {
        let out = adb
            .shell_status(&format!("am start -W -n {component}"))
            .await?
            .1;
        parse_am_start(&out).map_err(|e| {
            JidrError::sdk(format!(
                "couldn't start {component} on {}: {e}",
                t.device.name
            ))
        })?;
        for _ in 0..10 {
            if focused_on(
                &adb.shell("dumpsys window").await.unwrap_or_default(),
                package,
            ) {
                return Ok(true);
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
    Ok(false)
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
            let focused = launch(&t, &package).await?;
            Ok(json!({ "launched": package, "focused": focused }))
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

    /// Regression (PR #7 CI, API 35): `monkey` only injected an event, so
    /// app.launch said "ok" while Settings never came to the front.
    #[test]
    fn launch_output_is_parsed() {
        let resolved = "priority=0 preferredOrder=0 match=0x108000 specificIndex=-1 isDefault=false\n\
                        com.android.settings/.Settings\n";
        assert_eq!(
            parse_resolved(resolved),
            Some("com.android.settings/.Settings")
        );
        assert_eq!(parse_resolved("No activity found\n"), None);
        assert_eq!(parse_resolved(""), None);
        assert_eq!(parse_resolved("a/b; reboot"), None);

        let ok = "Starting: Intent { cmp=com.android.settings/.Settings }\n\
                  Status: ok\nLaunchState: COLD\nActivity: com.android.settings/.Settings\n\
                  TotalTime: 523\nWaitTime: 526\nComplete\n";
        assert_eq!(parse_am_start(ok), Ok(()));
        let delivered = "Starting: Intent { cmp=com.android.settings/.Settings }\n\
                         Warning: Activity not started, intent has been delivered to currently running top-most instance.\n\
                         Status: ok\nLaunchState: UNKNOWN (0)\nComplete\n";
        assert_eq!(parse_am_start(delivered), Ok(()));
        assert_eq!(parse_am_start("Starting: Intent {}\nComplete\n"), Ok(()));
        let missing = "Starting: Intent { cmp=x.y/.Z }\n\
                       Error type 3\nError: Activity class {x.y/x.y.Z} does not exist.\n";
        assert_eq!(parse_am_start(missing), Err("Error type 3".into()));
        assert!(parse_am_start("").is_err());

        let front = "  mCurrentFocus=Window{1 u0 com.android.settings/com.android.settings.Settings}\n\
                       mFocusedApp=ActivityRecord{2 u0 com.android.settings/.Settings t9}\n";
        assert!(focused_on(front, "com.android.settings"));
        let launcher = "  mCurrentFocus=Window{1 u0 com.google.android.apps.nexuslauncher/.NexusLauncherActivity}\n";
        assert!(!focused_on(launcher, "com.android.settings"));
        assert!(!focused_on(front, "com.android.set"));
    }

    #[test]
    fn package_names_are_checked() {
        assert!(check_package("com.example.shop").is_ok());
        assert!(check_package("a_b.c1").is_ok());
        for bad in ["", "com..x", "com.x;rm -rf", "com.x y", ".a"] {
            assert!(check_package(bad).is_err(), "{bad}");
        }
    }
}
