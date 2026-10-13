//! Fuzzes the JSON-RPC parser and params validation through the real socket:
//! 10,000 generated lines (garbage, broken requests, every real method with
//! random params). Each must get exactly one well-formed answer with the
//! right id; a panic in the daemon would show up as a missing answer.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Harness;
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, TestRng, TestRunner};
use serde_json::{Map, Value, json};

const CASES: usize = 10_000;

fn json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<f64>()
            .prop_filter("finite", |f| f.is_finite())
            .prop_map(Value::from),
        "[a-zA-Z0-9 _:;./-]{0,24}".prop_map(Value::from),
        Just(Value::from("35:google_apis")),
        Just(Value::from("00000000-0000-0000-0000-000000000000")),
    ];
    leaf.prop_recursive(3, 32, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::from),
            prop::collection::btree_map("[a-z_]{1,12}", inner, 0..5)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

#[derive(Debug, Clone)]
enum Case {
    /// A raw line that isn't a well-formed request.
    Raw(String),
    /// A request with an id that must be answered.
    Request(Value),
}

/// Params built from a command's schema: real keys with random values, plus
/// sometimes a bogus key or a missing required one.
fn params_for(schema: Value) -> impl Strategy<Value = Value> {
    let keys: Vec<String> = schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|p| p.keys().cloned().collect())
        .unwrap_or_default();
    let n = keys.len();
    (
        prop::collection::vec((0..n.max(1), json_value()), 0..(n + 2)),
        prop::option::of("[a-z]{1,8}"),
        json_value(),
        0..10u8,
    )
        .prop_map(move |(fields, bogus, raw, mode)| {
            if mode == 0 {
                return raw;
            }
            let mut obj = Map::new();
            for (i, v) in fields {
                if let Some(k) = keys.get(i) {
                    obj.insert(k.clone(), v);
                }
            }
            if let (Some(b), true) = (bogus, mode == 1) {
                obj.insert(b, Value::from(1));
            }
            Value::Object(obj)
        })
}

fn case(methods: Vec<(String, Value)>) -> impl Strategy<Value = Case> {
    let raw = prop_oneof![
        "[^\\n\\r]{1,80}"
            .prop_filter("not blank", |s| !s.trim().is_empty())
            .prop_map(Case::Raw),
        json_value()
            .prop_filter("not a request", |v| !v.is_object())
            .prop_map(|v| Case::Raw(v.to_string())),
    ];
    let broken = (
        any::<u32>(),
        json_value(),
        prop_oneof![Just("1.0"), Just("2"), Just("")],
    )
        .prop_map(|(id, params, version)| {
            Case::Request(
                json!({ "jsonrpc": version, "id": id, "method": "device.list", "params": params }),
            )
        });
    let no_method =
        any::<u32>().prop_map(|id| Case::Request(json!({ "jsonrpc": "2.0", "id": id })));
    let unknown = (any::<u32>(), "[a-z.]{1,20}", json_value()).prop_map(|(id, m, p)| {
        Case::Request(json!({ "jsonrpc": "2.0", "id": id, "method": m, "params": p }))
    });
    let real = (any::<u32>(), prop::sample::select(methods)).prop_flat_map(|(id, (method, schema))| {
        params_for(schema).prop_map(move |params| {
            Case::Request(json!({ "jsonrpc": "2.0", "id": id, "method": method.clone(), "params": params }))
        })
    });
    prop_oneof![2 => raw, 1 => broken, 1 => no_method, 1 => unknown, 5 => real]
}

/// Reads until a response (not an event notification) arrives; panics
/// naming the case if none comes.
async fn answer(c: &mut common::Conn, case: &dyn std::fmt::Debug) -> Value {
    let read = async {
        loop {
            let v = c.read().await;
            if v.get("result").is_some() || v.get("error").is_some() {
                return v;
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(8), read)
        .await
        .unwrap_or_else(|_| panic!("no answer for {case:?}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn fuzz_ten_thousand_cases() {
    let h = Harness::start().await;
    let mut c = h.client().await;
    let list = c.ok("commands.list", json!({})).await;
    let mut methods: Vec<(String, Value)> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["id"].as_str().unwrap().to_string(), s["params"].clone()))
        .collect();
    // Connection-scoped methods too (not daemon.shutdown: it would end the test).
    methods.push((
        "events.subscribe".into(),
        json!({ "properties": { "topics": {} } }),
    ));
    methods.push((
        "logcat.follow".into(),
        json!({ "properties": { "device": {}, "filter": {} } }),
    ));
    methods.push((
        "hello".into(),
        json!({ "properties": { "version": {}, "client": {} } }),
    ));

    let strategy = case(methods);
    let mut runner = TestRunner::new_with_rng(
        Config::default(),
        TestRng::deterministic_rng(proptest::test_runner::RngAlgorithm::ChaCha),
    );
    let (mut answered, mut errors) = (0usize, 0usize);
    for i in 0..CASES {
        let tree = strategy.new_tree(&mut runner).unwrap();
        match tree.current() {
            Case::Raw(line) => {
                c.send_raw(&line).await;
                let v = answer(&mut c, &line).await;
                let code = v["error"]["code"]
                    .as_i64()
                    .unwrap_or_else(|| panic!("case {i}: {line:?} → {v}"));
                assert!(code == -32700 || code == -32600, "case {i}: {line:?} → {v}");
                errors += 1;
            }
            Case::Request(req) => {
                c.send_raw(&req.to_string()).await;
                let v = answer(&mut c, &req).await;
                assert_eq!(v["jsonrpc"], "2.0", "case {i}: {req} → {v}");
                assert_eq!(v["id"], req["id"], "case {i}: {req} → {v}");
                assert!(
                    v.get("result").is_some() != v.get("error").is_some(),
                    "case {i}: {v}"
                );
                if let Some(e) = v.get("error") {
                    assert!(
                        e["code"].is_i64() && e["message"].is_string(),
                        "case {i}: {v}"
                    );
                    errors += 1;
                }
            }
        }
        answered += 1;
    }
    assert_eq!(answered, CASES);
    assert!(
        errors > CASES / 2,
        "most fuzz cases should be rejected ({errors})"
    );
    // Still healthy.
    c.ok("daemon.status", json!({})).await;
    h.stop().await;
}

proptest! {
    #![proptest_config(Config { cases: 2_000, .. Config::default() })]

    /// The validator itself never panics, whatever the schema's params.
    #[test]
    fn validator_never_panics(params in json_value()) {
        let tmp = tempfile::tempdir().unwrap();
        let daemon = jidr_daemon::Daemon::builder(
            jidr_daemon::DaemonConfig { paths: jidr_core::JidrPaths::under(tmp.path()), studio_sdk: None, idle_timeout: None, sdk_repo: None, },
            std::sync::Arc::new(jidr_backend::fake::FakeBackend::new()),
        ).build().unwrap();
        for spec in daemon.registry().list() {
            let _ = jidr_daemon::validate::validate(&spec.params, &params);
        }
    }
}
