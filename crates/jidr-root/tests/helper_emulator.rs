//! The rooted on-device helper (ADR-0003) on a real emulator, Shell root
//! (`adb`): `root.helper.status` pushes and starts the build from
//! `cargo xtask helper`, `network.shaping.set` drops packets with netem, a
//! killed helper is started again with the kernel's shaping intact, and `0`
//! clears it. Then bad DNS (#17): `slow` delays real lookups, `fail` drops
//! them, and loss and bad DNS both come back after a cold boot before they
//! are cleared. On API 30, whose kernel has no netem, it checks the
//! "unsupported" answer for both instead (D44).
//!
//! Build the helper for the image's CPU first: `cargo xtask helper` (CI runs
//! `--arch x86_64`). The image comes from `JIDR_TEST_IMAGE` (default: API 31
//! `google_apis` for the host ABI) in `JIDR_TEST_SDK` (default: Android
//! Studio's SDK, opened read-only). The AVD lives in a temp Jidr home.
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jidr_backend::AospBackend;
use jidr_backend::testing::{AvdGuard, kill_orphans, unique_avd};
use jidr_core::provider::{CallCtx, DaemonContext};
use jidr_core::{ConnectInfo, JidrPaths};
use jidr_daemon::{Daemon, DaemonConfig};
use serde_json::{Value, json};

fn test_sdk() -> PathBuf {
    std::env::var_os("JIDR_TEST_SDK")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Android/sdk")))
        .unwrap()
}

fn test_image() -> String {
    std::env::var("JIDR_TEST_IMAGE").unwrap_or_else(|_| {
        let abi = if cfg!(target_arch = "aarch64") {
            "arm64-v8a"
        } else {
            "x86_64"
        };
        format!("system-images;android-31;google_apis;{abi}")
    })
}

/// The image's API level (`JIDR_TEST_API`, default 31 like the image).
fn test_api() -> u32 {
    std::env::var("JIDR_TEST_API")
        .ok()
        .and_then(|a| a.parse().ok())
        .unwrap_or(31)
}

async fn call(daemon: &Arc<Daemon>, ctx: &CallCtx, method: &str, params: Value) -> Value {
    let (p, _) = daemon.registry().get(method).unwrap();
    p.call(ctx, method, params)
        .await
        .unwrap_or_else(|e| panic!("{method}: {e}"))
}

fn shell(info: &ConnectInfo, cmd: &str) -> String {
    let out = Command::new(&info.adb_path)
        .args(["-s", &info.adb_serial, "shell", cmd])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// How long the guest takes to look up a name nobody looked up before
/// (no resolver cache): `ping` resolves it first, through Android's
/// resolver, and `.invalid` never exists.
fn lookup_secs(info: &ConnectInfo) -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let t = Instant::now();
    let out = shell(info, &format!("ping -c 1 -W 1 jidr-{nanos}.invalid 2>&1"));
    let secs = t.elapsed().as_secs_f64();
    println!("lookup: {secs:.1} s: {}", out.trim());
    secs
}

#[tokio::test(flavor = "multi_thread")]
async fn helper_starts_as_root_and_drops_packets() {
    let tmp = tempfile::tempdir().unwrap();
    let daemon = Daemon::builder(
        DaemonConfig {
            paths: JidrPaths::under(tmp.path()),
            studio_sdk: Some(test_sdk()),
            idle_timeout: None,
            sdk_repo: None,
        },
        Arc::new(AospBackend::new()),
    )
    .provider(jidr_root::provider())
    .provider(jidr_widgets::provider())
    .build()
    .unwrap();
    let ctx = CallCtx::new(daemon.state().clone());
    let state = daemon.state().clone();
    kill_orphans("helper-e2e");
    let name = unique_avd("helper-e2e");
    call(
        &daemon,
        &ctx,
        "device.create",
        json!({ "name": name, "image": test_image(), "profile": "small-phone-5", "root_mode": "adb" }),
    )
    .await;
    let _guard = AvdGuard::new(&state.device(&name).await.unwrap().avd);
    let t = Instant::now();
    call(&daemon, &ctx, "device.start", json!({ "device": name })).await;
    println!("cold boot: {:.1} s", t.elapsed().as_secs_f64());
    let info = state
        .connect_info(state.device(&name).await.unwrap().id)
        .await
        .unwrap();

    // 1. Started as root, in the root shell's SELinux domain.
    let t = Instant::now();
    let v = call(
        &daemon,
        &ctx,
        "root.helper.status",
        json!({ "device": name }),
    )
    .await;
    println!(
        "first helper start: {:.1} s: {v}",
        t.elapsed().as_secs_f64()
    );
    assert_eq!(v["helper"]["uid"], 0, "{v}");
    assert_eq!(v["helper"]["protocol"], jidr_helper::rpc::PROTOCOL);
    let pid = v["helper"]["pid"].as_u64().unwrap();
    assert!(
        shell(&info, "ps -A -o PID,ARGS").contains(&format!("{pid} ")),
        "helper pid {pid} isn't running"
    );

    // 2. Wait for a network with a default route (Wi-Fi or mobile data).
    let deadline = Instant::now() + Duration::from_secs(120);
    let get = loop {
        let (p, _) = daemon.registry().get("network.shaping.get").unwrap();
        match p
            .call(&ctx, "network.shaping.get", json!({ "device": name }))
            .await
        {
            Ok(v) => break v,
            Err(e) if Instant::now() < deadline => {
                println!("no network yet: {e}");
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            Err(e) => panic!("network.shaping.get: {e}"),
        }
    };
    println!("before: {get}");
    let ifaces: Vec<String> = get["interfaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap().to_string())
        .collect();
    assert!(!ifaces.is_empty());
    assert_eq!(get["loss_pct"], 0.0, "{get}");

    // API 30's emulator kernel has no netem (D44): the helper says so as a
    // usage error instead of failing, and changes nothing.
    if test_api() < 31 {
        let (p, _) = daemon.registry().get("network.shaping.set").unwrap();
        let e = p
            .call(
                &ctx,
                "network.shaping.set",
                json!({ "device": name, "loss_pct": 50 }),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, jidr_core::ErrorCode::Usage, "{e}");
        assert!(e.message.contains("netem"), "{e}");
        // Bad DNS needs the prio qdisc and netem, which API 30's kernel
        // lacks too: refused the same way (CI, 2026-11-14: "no prio qdisc").
        let e = p
            .call(
                &ctx,
                "network.shaping.set",
                json!({ "device": name, "dns": "fail" }),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, jidr_core::ErrorCode::Usage, "{e}");
        assert!(e.message.contains("this image's kernel has no"), "{e}");
        assert_eq!(
            e.data.as_ref().unwrap()["helper_error"],
            "unsupported",
            "{e}"
        );
        // Nothing unsupported is stored, so no boot tries to re-apply it.
        assert!(
            state.device(&name).await.unwrap().network_shaping.is_off(),
            "a refused setting isn't stored"
        );
        for iface in &ifaces {
            let q = shell(&info, &format!("tc qdisc show dev {iface}"));
            assert!(
                !q.contains("netem") && !q.contains("prio 1d:"),
                "{iface}: {q}"
            );
        }
        call(&daemon, &ctx, "device.stop", json!({ "device": name })).await;
        call(&daemon, &ctx, "device.delete", json!({ "device": name })).await;
        return;
    }

    // 3. 50% loss: the kernel shows netem on every shaped interface.
    let v = call(
        &daemon,
        &ctx,
        "network.shaping.set",
        json!({ "device": name, "loss_pct": 50 }),
    )
    .await;
    assert_eq!(v["loss_pct"], 50.0, "{v}");
    for iface in &ifaces {
        let q = shell(&info, &format!("tc qdisc show dev {iface}"));
        assert!(
            q.contains("netem") && q.contains("loss 50%"),
            "{iface}: {q}"
        );
    }

    // 4. A killed helper is started again; the shaping is kernel state and stays.
    shell(&info, "pkill -x jidr-helper");
    let v = call(
        &daemon,
        &ctx,
        "root.helper.status",
        json!({ "device": name }),
    )
    .await;
    assert_ne!(v["helper"]["pid"].as_u64().unwrap(), pid, "{v}");
    let v = call(
        &daemon,
        &ctx,
        "network.shaping.get",
        json!({ "device": name }),
    )
    .await;
    assert_eq!(v["loss_pct"], 50.0, "{v}");

    // 5. 0 removes Jidr's netem.
    let v = call(
        &daemon,
        &ctx,
        "network.shaping.set",
        json!({ "device": name, "loss_pct": 0 }),
    )
    .await;
    assert_eq!(v["loss_pct"], 0.0, "{v}");
    for iface in &ifaces {
        let q = shell(&info, &format!("tc qdisc show dev {iface}"));
        assert!(!q.contains("netem"), "{iface}: {q}");
    }

    // 6. Slow DNS: a fresh lookup takes the 3 s delay more than before.
    let before = lookup_secs(&info);
    let v = call(
        &daemon,
        &ctx,
        "network.shaping.set",
        json!({ "device": name, "dns": "slow", "dns_delay_ms": 3000 }),
    )
    .await;
    assert_eq!(
        (&v["dns"], &v["dns_delay_ms"]),
        (&json!("slow"), &json!(3000)),
        "{v}"
    );
    for iface in &ifaces {
        let q = shell(&info, &format!("tc qdisc show dev {iface}"));
        assert!(
            q.contains("prio 1d:") && q.contains("delay"),
            "{iface}: {q}"
        );
    }
    let slow = lookup_secs(&info);
    assert!(
        slow >= 2.9 && slow >= before + 2.0,
        "slow DNS: {slow:.1} s, {before:.1} s before"
    );

    // 7. Failing DNS: no answers, so a lookup only ends when the resolver
    // gives up.
    call(
        &daemon,
        &ctx,
        "network.shaping.set",
        json!({ "device": name, "dns": "fail" }),
    )
    .await;
    let fail = lookup_secs(&info);
    assert!(fail >= 4.0, "failing DNS gave up after {fail:.1} s");

    // 8. Loss and slow DNS are stored and come back after a cold boot.
    call(
        &daemon,
        &ctx,
        "network.shaping.set",
        json!({ "device": name, "loss_pct": 20, "dns": "slow", "dns_delay_ms": 1500 }),
    )
    .await;
    call(&daemon, &ctx, "device.stop", json!({ "device": name })).await;
    call(
        &daemon,
        &ctx,
        "device.start",
        json!({ "device": name, "cold_boot": true }),
    )
    .await;
    let info = state
        .connect_info(state.device(&name).await.unwrap().id)
        .await
        .unwrap();
    let v = call(
        &daemon,
        &ctx,
        "network.shaping.get",
        json!({ "device": name }),
    )
    .await;
    println!("after a cold boot: {v}");
    assert_eq!(
        (&v["loss_pct"], &v["dns"], &v["dns_delay_ms"]),
        (&json!(20.0), &json!("slow"), &json!(1500)),
        "{v}"
    );

    // 9. Cleared: the kernel's own qdiscs, and nothing stored.
    let v = call(
        &daemon,
        &ctx,
        "network.shaping.set",
        json!({ "device": name, "loss_pct": 0, "dns": "normal" }),
    )
    .await;
    assert_eq!(
        (&v["loss_pct"], &v["dns"]),
        (&json!(0.0), &json!("normal")),
        "{v}"
    );
    for iface in &ifaces {
        let q = shell(&info, &format!("tc qdisc show dev {iface}"));
        assert!(
            !q.contains("netem") && !q.contains("prio 1d:"),
            "{iface}: {q}"
        );
    }
    assert!(
        state.device(&name).await.unwrap().network_shaping.is_off(),
        "clearing forgets the stored shaping"
    );

    call(&daemon, &ctx, "device.stop", json!({ "device": name })).await;
    call(&daemon, &ctx, "device.delete", json!({ "device": name })).await;
}
