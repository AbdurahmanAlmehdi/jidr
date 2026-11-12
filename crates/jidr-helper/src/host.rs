//! The daemon's side of the helper (ADR-0003): find the build for the
//! guest's CPU, push it and start it as root, reach it through
//! `adb forward`, and call it.
//!
//! Stateless on purpose: every [`Helper::connect`] finds the device's
//! existing forward (or makes one) and pings. Only when nothing answers, or
//! a helper of another build or protocol does, does it push this build and
//! run `jidr-helper start` as root, which replaces the old one. So reboots,
//! snapshot restores and daemon restarts need no bookkeeping: the next call
//! starts the helper again.

use std::path::{Path, PathBuf};
use std::time::Duration;

use jidr_adb::Adb;
use jidr_core::provider::CallCtx;
use jidr_core::{Device, JidrError, JidrResult, RootMode};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::rpc::{self, DEVICE_PATH, Method, PROTOCOL, Ping, Pong, Request, Response, SOCKET};

/// Overrides where the helper builds are looked for (tests, development).
pub const HELPER_DIR_ENV: &str = "JIDR_HELPER_DIR";

/// How long one call may take on the guest.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// How long connecting and the first `ping` may take.
const PING_TIMEOUT: Duration = Duration::from_secs(5);

/// The helper build for an Android ABI (`ro.product.cpu.abi`).
pub fn arch_for_abi(abi: &str) -> Option<&'static str> {
    match abi.trim() {
        "arm64-v8a" => Some("aarch64"),
        "x86_64" => Some("x86_64"),
        _ => None,
    }
}

/// Where helper builds may be, as `<dir>/<arch>/jidr-helper`, first match
/// wins: `$JIDR_HELPER_DIR`; `Jidr.app/Contents/Resources/helper` next to
/// the app's `Contents/Helpers/jidr`; a `helper` folder next to the binary
/// (the MCP bundle); and `target/jidr-helper` from `cargo xtask helper` for
/// development builds.
pub fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os(HELPER_DIR_ENV) {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(exe_dir) = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        dirs.push(exe_dir.join("../Resources/helper"));
        dirs.push(exe_dir.join("helper"));
        dirs.push(exe_dir.join("../jidr-helper"));
        dirs.push(exe_dir.join("../../jidr-helper"));
    }
    dirs
}

/// A helper build on the host and its SHA-256.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binary {
    pub path: PathBuf,
    pub sha256: String,
}

impl Binary {
    pub fn locate(arch: &str, dirs: &[PathBuf]) -> JidrResult<Binary> {
        let path = dirs
            .iter()
            .map(|d| d.join(arch).join("jidr-helper"))
            .find(|p| p.is_file())
            .ok_or_else(|| {
                JidrError::not_found(format!(
                    "no on-device helper built for {arch} (looked in {}); build it with `cargo xtask helper`",
                    dirs.iter()
                        .map(|d| d.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
                .with_data(json!({ "arch": arch }))
            })?;
        let sha256 = crate::sha256_file(&path)
            .map_err(|e| JidrError::unexpected(format!("reading {}: {e}", path.display())))?;
        Ok(Binary { path, sha256 })
    }
}

/// The port of `serial`'s existing forward to the helper, from
/// `adb forward --list` (`emulator-5554 tcp:51234 localabstract:jidr-helper`).
pub fn parse_forward(list: &str, serial: &str) -> Option<u16> {
    let remote = format!("localabstract:{SOCKET}");
    list.lines().find_map(|l| {
        let mut w = l.split_whitespace();
        let (s, local, r) = (w.next()?, w.next()?, w.next()?);
        (s == serial && r == remote)
            .then(|| local.strip_prefix("tcp:")?.parse().ok())
            .flatten()
    })
}

async fn forward(adb: &Adb) -> JidrResult<u16> {
    let serial = adb
        .serial()
        .ok_or_else(|| JidrError::unexpected("the helper needs a device serial"))?
        .to_string();
    let list = adb.run(&["forward", "--list"]).await?.stdout;
    if let Some(port) = parse_forward(&list, &serial) {
        return Ok(port);
    }
    let out = adb
        .run(&["forward", "tcp:0", &format!("localabstract:{SOCKET}")])
        .await?;
    out.stdout.trim().parse().map_err(|_| {
        JidrError::unexpected(format!(
            "adb forward printed no port: {:?}",
            out.stdout.trim()
        ))
    })
}

/// Maps a helper error onto Jidr's codes.
fn to_jidr(e: rpc::Error) -> JidrError {
    let data = json!({ "helper_error": e.code });
    let message = format!("on-device helper: {}", e.message);
    match e.code {
        rpc::ErrorCode::Usage | rpc::ErrorCode::Unsupported => JidrError::usage(message),
        rpc::ErrorCode::Failed => JidrError::unexpected(message),
    }
    .with_data(data)
}

/// The shell command that starts the helper as root in `mode`.
pub fn start_command(mode: RootMode) -> JidrResult<String> {
    let start = format!("{DEVICE_PATH} start");
    match mode {
        RootMode::Adb => Ok(start),
        RootMode::Magisk => Ok(format!("su -c '{start}'")),
        RootMode::None => Err(JidrError::usage(
            "this needs root: set the device's root mode to adb (Shell root) or magisk (Full root)",
        )),
    }
}

/// An open connection to a device's helper.
#[derive(Debug)]
pub struct Helper {
    stream: BufReader<TcpStream>,
    next_id: u64,
    /// What the helper said about itself when this connection opened.
    pub pong: Pong,
    /// The host port `adb forward` gave it.
    pub port: u16,
}

impl Helper {
    /// Connects to the helper on `device` (running, root mode `adb` or
    /// `magisk`), starting it first if needed.
    pub async fn for_device(ctx: &CallCtx, device: &Device) -> JidrResult<Helper> {
        start_command(device.root.mode)?;
        let info = ctx.daemon.connect_info(device.id).await.map_err(|e| {
            if e.code == jidr_core::ErrorCode::NotFound {
                JidrError::not_found(format!("{} isn't running; start it first", device.name))
            } else {
                e
            }
        })?;
        let adb = Adb::new(info.adb_path)
            .device(info.adb_serial)
            .with_timeout(Duration::from_secs(30));
        Helper::connect(&adb, device.root.mode, &search_dirs()).await
    }

    /// Connects through `adb` (aimed at one device), pushing and starting
    /// the build found in `dirs` when nothing of that build answers.
    pub async fn connect(adb: &Adb, mode: RootMode, dirs: &[PathBuf]) -> JidrResult<Helper> {
        let start = start_command(mode)?;
        let abi = adb.getprop("ro.product.cpu.abi").await?;
        let arch = arch_for_abi(&abi).ok_or_else(|| {
            JidrError::usage(format!("the on-device helper isn't built for {abi:?}"))
        })?;
        let binary = Binary::locate(arch, dirs)?;
        let port = forward(adb).await?;
        match Helper::open(port).await {
            Ok(h)
                if h.pong.protocol == PROTOCOL
                    && h.pong.sha256 == binary.sha256
                    && h.pong.uid == 0 =>
            {
                return Ok(h);
            }
            Ok(h) => tracing::info!(
                "replacing on-device helper {} (protocol {}) with this build",
                h.pong.version,
                h.pong.protocol
            ),
            Err(e) => tracing::debug!("no on-device helper answering yet: {e}"),
        }

        let tmp = format!("{DEVICE_PATH}.new");
        adb.push(&binary.path, &tmp).await?;
        adb.shell(&format!("chmod 755 {tmp} && mv -f {tmp} {DEVICE_PATH}"))
            .await?;
        let out = adb.raw(&["shell", &start]).await?;
        if !out.success() {
            return Err(JidrError::root_failed(format!(
                "starting the on-device helper failed: {}",
                format!("{}{}", out.stderr, out.stdout).trim()
            )));
        }
        let h = Helper::open(port).await.map_err(|e| {
            JidrError::root_failed(format!(
                "the on-device helper started but doesn't answer through adb forward: {}",
                e.message
            ))
        })?;
        if h.pong.uid != 0 {
            return Err(JidrError::root_failed(format!(
                "the on-device helper runs as uid {}, not root; check root with `jidr root verify`",
                h.pong.uid
            )));
        }
        if h.pong.sha256 != binary.sha256 || h.pong.protocol != PROTOCOL {
            return Err(JidrError::root_failed(format!(
                "the on-device helper is still another build ({}, protocol {})",
                h.pong.version, h.pong.protocol
            )));
        }
        Ok(h)
    }

    /// Opens a connection and pings.
    pub async fn open(port: u16) -> JidrResult<Helper> {
        let stream = tokio::time::timeout(PING_TIMEOUT, TcpStream::connect(("127.0.0.1", port)))
            .await
            .map_err(|_| JidrError::unexpected("connecting to the helper timed out"))?
            .map_err(|e| JidrError::unexpected(format!("connecting to the helper: {e}")))?;
        let mut h = Helper {
            stream: BufReader::new(stream),
            next_id: 0,
            pong: Pong {
                version: String::new(),
                protocol: 0,
                sha256: String::new(),
                pid: 0,
                uid: u32::MAX,
                selinux: None,
                arch: String::new(),
            },
            port,
        };
        h.pong = h.call_timeout(&Ping {}, PING_TIMEOUT).await?;
        Ok(h)
    }

    /// Calls a typed method.
    pub async fn call<M: Method>(&mut self, params: &M) -> JidrResult<M::Output> {
        self.call_timeout(params, CALL_TIMEOUT).await
    }

    async fn call_timeout<M: Method>(
        &mut self,
        params: &M,
        timeout: Duration,
    ) -> JidrResult<M::Output> {
        let params = serde_json::to_value(params)
            .map_err(|e| JidrError::unexpected(format!("serializing {}: {e}", M::NAME)))?;
        let v = tokio::time::timeout(timeout, self.call_raw(M::NAME, params))
            .await
            .map_err(|_| {
                JidrError::unexpected(format!(
                    "the on-device helper didn't answer {} in {} s",
                    M::NAME,
                    timeout.as_secs()
                ))
            })??;
        serde_json::from_value(v)
            .map_err(|e| JidrError::unexpected(format!("the helper's {} result: {e}", M::NAME)))
    }

    async fn call_raw(&mut self, method: &str, params: Value) -> JidrResult<Value> {
        self.next_id += 1;
        let id = self.next_id;
        let req = Request {
            id,
            method: method.to_string(),
            params,
        };
        let mut line = serde_json::to_vec(&req)
            .map_err(|e| JidrError::unexpected(format!("serializing {method}: {e}")))?;
        line.push(b'\n');
        let io = |e: std::io::Error| JidrError::unexpected(format!("helper connection: {e}"));
        self.stream.get_mut().write_all(&line).await.map_err(io)?;
        let mut answer = String::new();
        if self.stream.read_line(&mut answer).await.map_err(io)? == 0 {
            return Err(JidrError::unexpected(
                "the helper connection closed (nothing listening on the device)",
            ));
        }
        let resp: Response = serde_json::from_str(&answer)
            .map_err(|e| JidrError::unexpected(format!("the helper's answer to {method}: {e}")))?;
        if resp.id != id {
            return Err(JidrError::unexpected(format!(
                "the helper answered request {} instead of {id}",
                resp.id
            )));
        }
        match (resp.result, resp.error) {
            (_, Some(e)) => Err(to_jidr(e)),
            (Some(v), None) => Ok(v),
            (None, None) => Ok(Value::Null),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abis_map_to_builds() {
        assert_eq!(arch_for_abi("arm64-v8a\n"), Some("aarch64"));
        assert_eq!(arch_for_abi("x86_64"), Some("x86_64"));
        assert_eq!(arch_for_abi("armeabi-v7a"), None);
    }

    #[test]
    fn the_devices_own_forward_is_reused() {
        let list = "\
emulator-5554 tcp:41000 localabstract:jidr-helper
emulator-5556 tcp:41001 tcp:8080
emulator-5556 tcp:41002 localabstract:jidr-helper
";
        assert_eq!(parse_forward(list, "emulator-5556"), Some(41002));
        assert_eq!(parse_forward(list, "emulator-5554"), Some(41000));
        assert_eq!(parse_forward(list, "emulator-5558"), None);
        assert_eq!(parse_forward("", "emulator-5554"), None);
    }

    #[test]
    fn root_modes_start_it_as_root_or_refuse() {
        assert_eq!(
            start_command(RootMode::Adb).unwrap(),
            "/data/local/tmp/jidr-helper start"
        );
        assert_eq!(
            start_command(RootMode::Magisk).unwrap(),
            "su -c '/data/local/tmp/jidr-helper start'"
        );
        assert_eq!(
            start_command(RootMode::None).unwrap_err().code,
            jidr_core::ErrorCode::Usage
        );
    }

    #[test]
    fn a_missing_build_says_how_to_make_one() {
        let tmp = tempfile::tempdir().unwrap();
        let e = Binary::locate("aarch64", &[tmp.path().to_path_buf()]).unwrap_err();
        assert_eq!(e.code, jidr_core::ErrorCode::NotFound);
        assert!(e.message.contains("cargo xtask helper"), "{}", e.message);
        std::fs::create_dir_all(tmp.path().join("aarch64")).unwrap();
        std::fs::write(tmp.path().join("aarch64/jidr-helper"), b"elf").unwrap();
        let b = Binary::locate("aarch64", &[tmp.path().to_path_buf()]).unwrap();
        assert_eq!(b.sha256.len(), 64);
    }
}
