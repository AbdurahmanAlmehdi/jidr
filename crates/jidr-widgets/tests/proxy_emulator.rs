//! The global HTTP proxy on a real emulator: `network.proxy.set` reaches
//! Android's connectivity service (its ProxyTracker broadcasts the new
//! proxy), and `network.proxy.clear` removes it. Run it on an API 34+ image
//! (`JIDR_TEST_IMAGE=system-images;android-35;google_apis;arm64-v8a`).
#![cfg(feature = "emulator")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use serde_json::json;

/// The proxies ConnectivityService's ProxyTracker announced, oldest first:
/// `sending Proxy Broadcast for [10.0.2.2] 8080 xl=`.
fn broadcasts(log: &str) -> Vec<String> {
    log.lines()
        .filter_map(|l| {
            l.split_once("sending Proxy Broadcast for ")
                .map(|(_, p)| p.trim().to_string())
        })
        .collect()
}

const PROXY_LOG: &str = "logcat -d -s ProxyTracker";

#[tokio::test(flavor = "multi_thread")]
async fn proxy_set_and_clear() {
    let env = common::Env::boot("widgets-proxy").await;
    let none = env.call("network.proxy.get", json!({})).await;
    assert!(none["proxy"].is_null(), "{none}");

    let set = env
        .call(
            "network.proxy.set",
            json!({ "host": "10.0.2.2", "port": 8080 }),
        )
        .await;
    assert_eq!(set["proxy"], "10.0.2.2:8080", "{set}");
    assert_eq!(
        env.shell("settings get global http_proxy").trim(),
        "10.0.2.2:8080"
    );
    env.eventually_shell(
        "the connectivity service applies the proxy",
        PROXY_LOG,
        &["Proxy Broadcast"],
        "",
        |out| {
            broadcasts(out)
                .last()
                .is_some_and(|p| p.starts_with("[10.0.2.2] 8080"))
        },
    )
    .await;
    let got = env.call("network.proxy.get", json!({})).await;
    assert_eq!(
        (got["host"].as_str(), got["port"].as_u64()),
        (Some("10.0.2.2"), Some(8080))
    );

    let cleared = env.call("network.proxy.clear", json!({})).await;
    assert!(cleared["proxy"].is_null(), "{cleared}");
    env.eventually_shell(
        "the connectivity service drops the proxy",
        PROXY_LOG,
        &["Proxy Broadcast"],
        "",
        |out| {
            broadcasts(out)
                .last()
                .is_some_and(|p| !p.contains("10.0.2.2"))
        },
    )
    .await;
    assert_eq!(env.shell("settings get global http_proxy").trim(), ":0");
    let got = env.call("network.proxy.get", json!({})).await;
    assert!(got["proxy"].is_null(), "{got}");
    env.stop().await;
}

#[test]
fn reads_proxy_broadcasts() {
    let log = "11-02 18:25:51.200  1936  2374 D ProxyTracker: sending Proxy Broadcast for [10.0.2.2] 8080 xl=\n\
               11-02 18:26:01.000  1936  2374 D ProxyTracker: sending Proxy Broadcast for null\n";
    assert_eq!(broadcasts(log), ["[10.0.2.2] 8080 xl=", "null"]);
    assert!(broadcasts("nothing").is_empty());
}
