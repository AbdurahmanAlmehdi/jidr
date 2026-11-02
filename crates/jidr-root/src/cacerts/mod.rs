//! CA certificates as system certs on a Full-root device: what Burp,
//! mitmproxy or Frida-based tooling need before apps accept the proxy.
//!
//! Jidr writes its own small Magisk module, `/data/adb/modules/jidr_cacerts`,
//! in Magisk's documented layout (`module.prop`, a `system/` folder Magisk
//! merges into `/system`, and a `post-fs-data.sh`):
//!
//! - `system/etc/security/cacerts/<hash>.<n>`: Magisk's system overlay puts
//!   the certificates in `/system/etc/security/cacerts`, the store API ≤ 33
//!   reads.
//! - API 34+ read the Conscrypt APEX store, `/apex/com.android.conscrypt/
//!   cacerts`, which no overlay reaches. `post-fs-data.sh` runs before Zygote
//!   starts (Magisk docs), copies that store and our certificates into a
//!   directory on `/dev` (a tmpfs) and bind-mounts it over the APEX store, so
//!   Zygote and every app it forks see it.
//!
//! Changes apply on the next boot, so install and remove cold-boot the device
//! (after a `sync`: unflushed writes on `/data` don't survive a cold boot).
//! Verification reads the store the way Zygote sees it and compares the
//! bytes, as `root.verify` compares uids. The script is Jidr's own; no module
//! code is copied (docs/clean-room.md).

pub mod x509;

use std::path::PathBuf;
use std::time::Duration;

use jidr_adb::Adb;
use jidr_core::provider::CallCtx;
use jidr_core::{Device, ErrorCode, JidrError, JidrResult, ResultExt, RootMode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{magisk, verify};
use x509::CaCert;

/// The module's id (Magisk: `^[a-zA-Z][a-zA-Z0-9._-]+$`) and folder.
pub const MODULE_ID: &str = "jidr_cacerts";
const MODULE: &str = "/data/adb/modules/jidr_cacerts";
const CERTS: &str = "/data/adb/modules/jidr_cacerts/system/etc/security/cacerts";
pub const APEX_STORE: &str = "/apex/com.android.conscrypt/cacerts";
pub const SYSTEM_STORE: &str = "/system/etc/security/cacerts";
/// Where a change is staged on the device before `apply.sh` moves it in.
const STAGING: &str = "/data/local/tmp/jidr-cacerts";
/// Largest certificate file accepted.
const MAX_FILE: u64 = 64 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallParams {
    /// Device name or id (must be in `magisk` mode and running).
    pub device: String,
    /// A CA certificate, PEM or DER (`.pem`, `.crt`, `.cer`, `.der`), e.g.
    /// Burp's or mitmproxy's.
    pub path: PathBuf,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveParams {
    /// Device name or id (must be in `magisk` mode and running).
    pub device: String,
    /// The certificate: its store file name (`a4ac454c.0`), subject hash or
    /// SHA-256 (or a unique prefix of 8+ hex digits). Leave out with `all`.
    #[serde(default)]
    pub cert: Option<String>,
    /// Remove every certificate Jidr installed (and the module).
    #[serde(default)]
    pub all: bool,
}

/// One certificate Jidr installed, as `root.cacert.list` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Installed {
    /// The store file name, `<subject hash>.<n>`.
    pub file: String,
    pub subject: String,
    pub sha256: String,
    /// The system store apps use has this exact certificate (as Zygote
    /// sees it). False until the reboot after an install.
    pub trusted: bool,
}

/// What the device reports: the store apps read and Jidr's certificates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct State {
    /// `/apex/com.android.conscrypt/cacerts` (API 34+) or
    /// `/system/etc/security/cacerts`.
    pub store: String,
    pub certs: Vec<Installed>,
}

fn module_prop() -> String {
    format!(
        "id={MODULE_ID}\nname=Jidr CA certificates\nversion=1\nversionCode=1\nauthor=Jidr\n\
         description=CA certificates Jidr added to the system store (jidr cert). Managed by Jidr.\n"
    )
}

/// Runs at post-fs-data, before Zygote, under Magisk's BusyBox `sh`.
fn post_fs_data() -> String {
    format!(
        r#"#!/system/bin/sh
# Written by Jidr (jidr cert install); rewritten on every change.
# API 34+ read CA certificates from the Conscrypt APEX, which Magisk's system
# overlay can't reach: serve a copy of that store plus ours over it, before
# Zygote starts, so every app inherits the mount.
MODDIR=${{0%/*}}
APEX={APEX_STORE}
[ -d "$APEX" ] || exit 0
STORE=/dev/{MODULE_ID}
rm -rf "$STORE"
mkdir -p "$STORE"
cp -p "$APEX"/* "$STORE"/
cp "$MODDIR"/system/etc/security/cacerts/* "$STORE"/
chown -R 0:0 "$STORE"
chmod 755 "$STORE"
chmod 644 "$STORE"/*
chcon -R u:object_r:system_security_cacerts_file:s0 "$STORE"
mount --bind "$STORE" "$APEX"
"#
    )
}

/// The device-side script for one change: copy `add` in (from staging),
/// delete `remove`; drop the module when no certificate is left. Ends with
/// `sync` because a cold boot follows.
fn apply_script(add: Option<&str>, remove: &[String]) -> String {
    let mut s = vec![
        "set -e".to_string(),
        format!("mkdir -p {CERTS}"),
        format!("cp {STAGING}/module.prop {STAGING}/post-fs-data.sh {MODULE}/"),
        format!("chmod 755 {MODULE}/post-fs-data.sh"),
    ];
    if let Some(file) = add {
        s.push(format!("cp {STAGING}/{file} {CERTS}/{file}"));
    }
    for file in remove {
        s.push(format!("rm -f {CERTS}/{file}"));
    }
    s.push(format!("chmod 644 {CERTS}/* 2>/dev/null || true"));
    s.push(format!("chown -R 0:0 {MODULE}"));
    // What Magisk's own installer gives module files; /system's store is
    // labelled the same on API <= 33.
    s.push(format!(
        "chcon -R u:object_r:system_file:s0 {MODULE}/system"
    ));
    s.push(format!(
        "if [ -z \"$(ls {CERTS})\" ]; then rm -rf {MODULE}; fi"
    ));
    s.push(format!("rm -rf {STAGING}"));
    s.push("sync".to_string());
    s.join("\n") + "\n"
}

/// Prints the store apps read and, per Jidr certificate, `cert <file> <0|1>`
/// (whether Zygote's view of the store has the same bytes) and its PEM.
fn state_script() -> String {
    format!(
        r#"if [ -d {APEX_STORE} ]; then S={APEX_STORE}; else S={SYSTEM_STORE}; fi
echo "store $S"
Z=$(pidof -s zygote64 || pidof -s zygote)
[ -d {CERTS} ] || exit 0
for f in {CERTS}/*; do
  [ -f "$f" ] || continue
  n=${{f##*/}}
  if [ -n "$Z" ] && nsenter -t "$Z" -m -- cmp -s "$S/$n" "$f"; then t=1; else t=0; fi
  echo "cert $n $t"
  cat "$f"
done
"#
    )
}

/// Parses [`state_script`]'s output.
pub fn parse_state(out: &str) -> JidrResult<State> {
    let mut store = None;
    let mut certs = Vec::new();
    let mut current: Option<(String, bool, String)> = None;
    let finish = |c: Option<(String, bool, String)>, certs: &mut Vec<Installed>| {
        if let Some((file, trusted, pem)) = c {
            let (subject, sha256) = match x509::parse(pem.as_bytes()) {
                Ok(c) => (c.subject, c.sha256),
                Err(_) => (file.clone(), String::new()),
            };
            certs.push(Installed {
                file,
                subject,
                sha256,
                trusted,
            });
        }
    };
    for line in out.lines() {
        if let Some(s) = line.strip_prefix("store ") {
            store = Some(s.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("cert ") {
            finish(current.take(), &mut certs);
            let mut parts = rest.split_whitespace();
            let file = parts.next().unwrap_or_default().to_string();
            current = Some((file, parts.next() == Some("1"), String::new()));
        } else if let Some((_, _, pem)) = current.as_mut() {
            pem.push_str(line);
            pem.push('\n');
        }
    }
    finish(current, &mut certs);
    let store = store.ok_or_else(|| {
        JidrError::root_failed(format!(
            "reading the CA store printed no store: {}",
            out.trim()
        ))
    })?;
    Ok(State { store, certs })
}

fn ensure_full_root(device: &Device) -> JidrResult<()> {
    if device.root.mode != RootMode::Magisk {
        return Err(JidrError::usage(format!(
            "{} isn't Full-root (Magisk); system CA certificates need `jidr root set {} magisk`",
            device.name, device.name
        )));
    }
    crate::ensure_running(device)
}

/// A file staged on the host, pushed to [`STAGING`], removed when dropped.
struct Staging(PathBuf);

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn stage(files: &[(&str, &[u8])]) -> JidrResult<Staging> {
    let dir = std::env::temp_dir().join(format!("jidr-cacerts-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir)
        .ctx(ErrorCode::Unexpected, format!("creating {}", dir.display()))?;
    let staging = Staging(dir);
    for (name, data) in files {
        let path = staging.0.join(name);
        std::fs::write(&path, data)
            .ctx(ErrorCode::Unexpected, format!("writing {}", path.display()))?;
    }
    Ok(staging)
}

/// Pushes `files` to [`STAGING`] and runs `script` (one of them) as root.
async fn run_as_root(adb: &Adb, files: &[(&str, &[u8])], script: &str) -> JidrResult<String> {
    let staging = stage(files)?;
    adb.shell(&format!("rm -rf {STAGING}")).await?;
    adb.push(&staging.0.join("."), STAGING).await?;
    let (status, out) = adb
        .clone()
        .with_timeout(Duration::from_secs(60))
        .shell_status(&format!("su -c 'sh {STAGING}/{script}'"))
        .await?;
    if status == 0 {
        Ok(out)
    } else {
        Err(JidrError::root_failed(format!(
            "`{script}` failed: {}",
            out.trim()
        )))
    }
}

/// Reads the store and Jidr's certificates (`su`).
async fn state(adb: &Adb) -> JidrResult<State> {
    let script = state_script();
    let out = run_as_root(adb, &[("state.sh", script.as_bytes())], "state.sh").await?;
    parse_state(&out)
}

/// The store file name for `cert`: `<hash>.<n>` with the lowest `n` not
/// taken by a different certificate; `(name, already installed)`.
pub fn file_name(cert: &CaCert, installed: &[Installed]) -> (String, bool) {
    let mut n = 0;
    loop {
        let name = format!("{}.{n}", cert.subject_hash);
        match installed.iter().find(|i| i.file == name) {
            Some(i) if i.sha256 == cert.sha256 => return (name, true),
            Some(_) => n += 1,
            None => return (name, false),
        }
    }
}

/// The certificates `selector` names: a file name, a subject hash, or a
/// SHA-256 (prefix of 8+ hex digits).
pub fn select<'a>(installed: &'a [Installed], selector: &str) -> JidrResult<Vec<&'a Installed>> {
    let s = selector.trim().to_ascii_lowercase();
    let hits: Vec<&Installed> = installed
        .iter()
        .filter(|i| {
            i.file == s
                || i.file.split('.').next() == Some(s.as_str())
                || (s.len() >= 8 && i.sha256.starts_with(&s))
        })
        .collect();
    if hits.is_empty() {
        let have: Vec<&str> = installed.iter().map(|i| i.file.as_str()).collect();
        return Err(JidrError::not_found(format!(
            "no Jidr CA certificate matches {selector:?} (installed: {})",
            if have.is_empty() {
                "none".to_string()
            } else {
                have.join(", ")
            }
        )));
    }
    Ok(hits)
}

/// Cold-boots, then waits until `su` answers again.
async fn reboot(ctx: &CallCtx, device: &Device) -> JidrResult<Device> {
    ctx.report(None, "Rebooting to apply");
    let device = magisk::cold_boot(ctx, device).await?;
    let adb = verify::device_adb(ctx, &device).await?;
    crate::adb_ready::until(
        &adb,
        Some(&ctx.cancel),
        Duration::from_secs(90),
        "for su after the reboot",
        "su -c id",
        |out| verify::parse_uid(out) == Some(0),
    )
    .await?;
    Ok(device)
}

/// Fails with `root_failed` unless every Jidr certificate is in the store.
fn require_trusted(device: &Device, state: &State) -> JidrResult<()> {
    let missing: Vec<&str> = state
        .certs
        .iter()
        .filter(|c| !c.trusted)
        .map(|c| c.file.as_str())
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(JidrError::root_failed(format!(
            "{}: {} not in the system store {} yet",
            device.name,
            missing.join(", "),
            state.store
        ))
        .with_data(serde_json::to_value(state).unwrap_or_default()))
    }
}

/// `root.cacert.install`.
pub async fn install(ctx: &CallCtx, p: InstallParams) -> JidrResult<Value> {
    let device = ctx.daemon.device(&p.device).await?;
    ensure_full_root(&device)?;
    let size = std::fs::metadata(&p.path)
        .map_err(|_| JidrError::usage(format!("no such certificate file: {}", p.path.display())))?
        .len();
    if size > MAX_FILE {
        return Err(JidrError::usage(format!(
            "{} is {size} bytes; a CA certificate is a few KB",
            p.path.display()
        )));
    }
    let bytes =
        std::fs::read(&p.path).ctx(ErrorCode::Usage, format!("reading {}", p.path.display()))?;
    let cert = x509::parse(&bytes)?;

    let adb = verify::device_adb(ctx, &device).await?;
    ctx.report(None, "Reading the system CA store");
    let before = state(&adb).await?;
    let (file, already) = file_name(&cert, &before.certs);
    let entry = |trusted| Installed {
        file: file.clone(),
        subject: cert.subject.clone(),
        sha256: cert.sha256.clone(),
        trusted,
    };
    if already && before.certs.iter().any(|c| c.file == file && c.trusted) {
        return Ok(json!({
            "device": device.id,
            "cert": entry(true),
            "changed": false,
            "store": before.store,
        }));
    }

    ctx.report(None, &format!("Adding {} ({file})", cert.subject));
    let pem = x509::to_pem(&cert.der);
    let apply = apply_script(Some(&file), &[]);
    run_as_root(
        &adb,
        &[
            ("module.prop", module_prop().as_bytes()),
            ("post-fs-data.sh", post_fs_data().as_bytes()),
            (file.as_str(), pem.as_bytes()),
            ("apply.sh", apply.as_bytes()),
        ],
        "apply.sh",
    )
    .await?;

    let device = reboot(ctx, &device).await?;
    ctx.report(None, "Verifying the system CA store");
    let adb = verify::device_adb(ctx, &device).await?;
    let after = state(&adb).await?;
    require_trusted(&device, &after)?;
    Ok(json!({
        "device": device.id,
        "cert": entry(true),
        "changed": true,
        "store": after.store,
    }))
}

/// `root.cacert.list`: Jidr's certificates and whether each is trusted.
pub async fn list(ctx: &CallCtx, selector: &str) -> JidrResult<Value> {
    let device = ctx.daemon.device(selector).await?;
    ensure_full_root(&device)?;
    let s = state(&verify::device_adb(ctx, &device).await?).await?;
    Ok(json!({ "device": device.id, "store": s.store, "certs": s.certs }))
}

/// `root.cacert.verify`: like `list`, but fails unless all are trusted.
pub async fn verify(ctx: &CallCtx, selector: &str) -> JidrResult<Value> {
    let device = ctx.daemon.device(selector).await?;
    ensure_full_root(&device)?;
    let s = state(&verify::device_adb(ctx, &device).await?).await?;
    require_trusted(&device, &s)?;
    Ok(json!({ "device": device.id, "ok": true, "store": s.store, "certs": s.certs }))
}

/// Whether Zygote's view of `store` still has `file`.
async fn still_in_store(adb: &Adb, store: &str, file: &str) -> JidrResult<bool> {
    let script = format!(
        "Z=$(pidof -s zygote64 || pidof -s zygote)\nnsenter -t \"$Z\" -m -- ls {store}/{file} >/dev/null 2>&1 && echo present || echo absent\n"
    );
    let out = run_as_root(adb, &[("check.sh", script.as_bytes())], "check.sh").await?;
    Ok(out.contains("present"))
}

/// `root.cacert.remove`.
pub async fn remove(ctx: &CallCtx, p: RemoveParams) -> JidrResult<Value> {
    let device = ctx.daemon.device(&p.device).await?;
    ensure_full_root(&device)?;
    if p.all == p.cert.is_some() {
        return Err(JidrError::usage("give either cert or all"));
    }
    let adb = verify::device_adb(ctx, &device).await?;
    let before = state(&adb).await?;
    let gone: Vec<Installed> = match &p.cert {
        Some(sel) => select(&before.certs, sel)?.into_iter().cloned().collect(),
        None => before.certs.clone(),
    };
    if gone.is_empty() {
        return Ok(json!({ "device": device.id, "removed": [], "changed": false }));
    }
    let files: Vec<String> = gone.iter().map(|c| c.file.clone()).collect();
    ctx.report(None, &format!("Removing {}", files.join(", ")));
    let apply = apply_script(None, &files);
    run_as_root(
        &adb,
        &[
            ("module.prop", module_prop().as_bytes()),
            ("post-fs-data.sh", post_fs_data().as_bytes()),
            ("apply.sh", apply.as_bytes()),
        ],
        "apply.sh",
    )
    .await?;

    let device = reboot(ctx, &device).await?;
    ctx.report(None, "Verifying the system CA store");
    let adb = verify::device_adb(ctx, &device).await?;
    let after = state(&adb).await?;
    for f in &files {
        if still_in_store(&adb, &after.store, f).await? {
            return Err(JidrError::root_failed(format!(
                "{}: {f} is still in the system store {} after the reboot",
                device.name, after.store
            )));
        }
    }
    require_trusted(&device, &after)?;
    Ok(json!({
        "device": device.id,
        "removed": gone,
        "changed": true,
        "store": after.store,
        "certs": after.certs,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEM: &str = include_str!("../../tests/fixtures/jidr-test-ca.pem");
    const RENEWED: &str = include_str!("../../tests/fixtures/jidr-test-ca-renewed.pem");

    fn installed(file: &str, pem: &str, trusted: bool) -> Installed {
        let c = x509::parse(pem.as_bytes()).unwrap();
        Installed {
            file: file.into(),
            subject: c.subject,
            sha256: c.sha256,
            trusted,
        }
    }

    #[test]
    fn reads_the_device_state() {
        let out = format!(
            "store /apex/com.android.conscrypt/cacerts\ncert a4ac454c.0 1\n{PEM}cert a4ac454c.1 0\n{RENEWED}"
        );
        let s = parse_state(&out).unwrap();
        assert_eq!(s.store, APEX_STORE);
        assert_eq!(
            s.certs,
            [
                installed("a4ac454c.0", PEM, true),
                installed("a4ac454c.1", RENEWED, false)
            ]
        );
        let none = parse_state("store /system/etc/security/cacerts\n").unwrap();
        assert!(none.certs.is_empty());
        assert!(parse_state("su: denied").is_err());
    }

    #[test]
    fn same_subject_gets_the_next_index() {
        let cert = x509::parse(PEM.as_bytes()).unwrap();
        let renewed = x509::parse(RENEWED.as_bytes()).unwrap();
        assert_eq!(file_name(&cert, &[]), ("a4ac454c.0".into(), false));
        let have = [installed("a4ac454c.0", PEM, true)];
        assert_eq!(file_name(&cert, &have), ("a4ac454c.0".into(), true));
        assert_eq!(file_name(&renewed, &have), ("a4ac454c.1".into(), false));
    }

    #[test]
    fn selects_by_file_hash_or_sha256() {
        let have = [
            installed("a4ac454c.0", PEM, true),
            installed("a4ac454c.1", RENEWED, true),
        ];
        assert_eq!(select(&have, "a4ac454c.1").unwrap().len(), 1);
        assert_eq!(
            select(&have, "A4AC454C").unwrap().len(),
            2,
            "the hash names both"
        );
        assert_eq!(select(&have, "3fe6832f").unwrap()[0].file, "a4ac454c.0");
        assert_eq!(
            select(&have, "3fe6").unwrap_err().code,
            jidr_core::ErrorCode::NotFound,
            "short prefixes don't match"
        );
        assert!(
            select(&have, "nope")
                .unwrap_err()
                .message
                .contains("a4ac454c.0")
        );
    }

    #[test]
    fn scripts_stay_in_jidrs_module() {
        let add = apply_script(Some("a4ac454c.0"), &[]);
        assert!(add.contains(&format!("cp {STAGING}/a4ac454c.0 {CERTS}/a4ac454c.0")));
        assert!(add.trim_end().ends_with("sync"), "a cold boot follows");
        let rm = apply_script(None, &["a4ac454c.0".into()]);
        assert!(rm.contains(&format!("rm -f {CERTS}/a4ac454c.0")));
        assert!(
            rm.contains(&format!("rm -rf {MODULE}; fi")),
            "the last one drops the module"
        );
        let pfd = post_fs_data();
        assert!(pfd.contains("mount --bind \"$STORE\" \"$APEX\""));
        assert!(pfd.contains("system_security_cacerts_file"));
        assert!(module_prop().starts_with("id=jidr_cacerts\n"));
    }
}
