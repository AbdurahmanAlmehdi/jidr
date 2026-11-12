//! Offline tests of Magisk bases (D42, Phase 13f): `base.create` from a
//! Full-root device and `pool.get` clones that each boot their own copy of
//! the patched ramdisk, on the fake backend and the fake Magisk.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashSet;
use std::path::PathBuf;

use common::Env;
use jidr_core::provider::DaemonContext;
use jidr_core::{ErrorCode, RootMode};
use serde_json::{Value, json};

async fn to_magisk(env: &Env, name: &str) {
    let entry = env
        .call("cache.import", json!({ "path": env.apk }))
        .await
        .unwrap();
    let sha = entry["sha256"].as_str().unwrap().to_string();
    env.call(
        "root.set",
        json!({ "device": name, "mode": "magisk", "magisk_sha256": sha }),
    )
    .await
    .unwrap();
}

/// A running Full-root device `phone`.
async fn magisk_phone(env: &Env) {
    env.create("phone", "35:google_apis", None).await;
    to_magisk(env, "phone").await;
    let d = env.device("phone").await;
    assert!(d.status.is_active(), "root.set leaves it running");
}

fn path(v: &Value) -> PathBuf {
    PathBuf::from(v.as_str().unwrap())
}

#[tokio::test]
async fn magisk_base_clones_each_boot_their_own_ramdisk() {
    let env = Env::start().await;
    magisk_phone(&env).await;
    let pristine = env.pristine_sha256();
    let src = env.device("phone").await;
    let src_ramdisk = env.patched_ramdisk(&src);
    let patched = std::fs::read(&src_ramdisk).unwrap();

    let b = env
        .call(
            "base.create",
            json!({ "from_device": "phone", "name": "MBase" }),
        )
        .await
        .unwrap();
    assert_eq!(b["root"]["mode"], "magisk", "{b}");
    assert_eq!(b["root"]["magisk_version"], common::MAGISK_VERSION);
    let base = env.device("MBase").await;
    let base_ramdisk = env.patched_ramdisk(&base);
    assert_eq!(path(&b["launch"]["ramdisk"]), base_ramdisk, "{b}");
    assert_eq!(path(&b["root"]["patched_ramdisk_path"]), base_ramdisk);
    assert_eq!(std::fs::read(&base_ramdisk).unwrap(), patched);
    assert!(src_ramdisk.is_file(), "the source keeps its own copy");
    assert_eq!(
        env.device("phone").await.launch.ramdisk.as_deref(),
        Some(src_ramdisk.as_path())
    );

    let mut leases = Vec::new();
    let mut ramdisks = HashSet::new();
    for _ in 0..3 {
        let r = env
            .call("pool.get", json!({ "base": "MBase", "ttl_s": 600 }))
            .await
            .unwrap();
        let d = env.device(r["device"]["name"].as_str().unwrap()).await;
        assert_eq!(d.root.mode, RootMode::Magisk, "the root tag carries over");
        assert_eq!(
            d.root.magisk_version.as_deref(),
            Some(common::MAGISK_VERSION)
        );
        let own = env.patched_ramdisk(&d);
        assert_eq!(d.launch.ramdisk.as_deref(), Some(own.as_path()));
        assert_eq!(d.root.patched_ramdisk_path.as_deref(), Some(own.as_path()));
        assert_eq!(std::fs::read(&own).unwrap(), patched);
        assert!(ramdisks.insert(own.clone()), "no two clones share one");
        // Resumed from the base snapshot with its own ramdisk, no cold boot.
        let launches = env.backend.launches();
        let opts = &launches.last().unwrap().1;
        assert_eq!(opts.ramdisk.as_deref(), Some(own.as_path()));
        assert_eq!(opts.snapshot.as_deref(), Some("jidr-base"));
        assert!(!opts.cold_boot);
        assert!(!d.launch.force_cold_boot, "the boot hook left it alone");

        let v = env
            .call("root.verify", json!({ "device": d.name }))
            .await
            .unwrap();
        assert_eq!(v["uid"], 0, "{v}");
        assert_eq!(v["command"], "su -c id");
        assert_eq!(v["magisk_version"], common::MAGISK_VERSION);
        leases.push((r["lease"]["id"].clone(), own));
    }

    for (lease, own) in leases {
        env.call("pool.release", json!({ "lease": lease }))
            .await
            .unwrap();
        assert!(!own.exists(), "the clone's copy goes with it");
    }
    assert!(base_ramdisk.is_file(), "the base keeps its copy");
    assert_eq!(
        env.pristine_sha256(),
        pristine,
        "the SDK image is untouched"
    );
    env.call("base.delete", json!({ "base": "MBase" }))
        .await
        .unwrap();
    assert!(!base_ramdisk.exists());
}

#[tokio::test]
async fn magisk_base_refuses_a_source_whose_su_fails() {
    let env = Env::start().await;
    magisk_phone(&env).await;
    env.revoke_su();
    let e = env
        .call(
            "base.create",
            json!({ "from_device": "phone", "name": "MBase" }),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::RootFailed, "{e}");
    assert!(e.message.contains("root verify"), "{e}");
    assert!(env.daemon.state().device("MBase").await.is_err());
    assert!(
        env.device("phone").await.status.is_active(),
        "the source keeps running"
    );
}

#[tokio::test]
async fn magisk_base_refuses_a_source_without_its_ramdisk() {
    let env = Env::start().await;
    magisk_phone(&env).await;
    let src = env.device("phone").await;
    std::fs::remove_file(env.patched_ramdisk(&src)).unwrap();
    let e = env
        .call(
            "base.create",
            json!({ "from_device": "phone", "name": "MBase" }),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage, "{e}");
    assert!(e.message.contains("no patched ramdisk"), "{e}");
}

#[tokio::test]
async fn magisk_base_that_lost_its_ramdisk_leaves_no_clone() {
    let env = Env::start().await;
    magisk_phone(&env).await;
    env.call(
        "base.create",
        json!({ "from_device": "phone", "name": "MBase" }),
    )
    .await
    .unwrap();
    let base = env.device("MBase").await;
    std::fs::remove_file(env.patched_ramdisk(&base)).unwrap();
    let e = env
        .call("pool.get", json!({ "base": "MBase", "ttl_s": 600 }))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound, "{e}");
    assert!(e.message.contains("make the base again"), "{e}");
    let devices = env.daemon.state().devices().await;
    assert_eq!(devices.len(), 2, "only phone and MBase");
    let avds: Vec<String> = std::fs::read_dir(env.daemon.state().paths().avd_home())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".avd"))
        .collect();
    assert_eq!(avds.len(), 2, "no clone folder left: {avds:?}");
}
