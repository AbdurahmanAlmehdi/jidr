//! `audio.inject` / `audio.stop` (#70) through a daemon with the fake
//! backend: params, file checks, an unreachable emulator, and the
//! one-injection-per-device bookkeeping. Packet streaming and pacing are
//! unit-tested in `jidr_backend::audio`; a real emulator in
//! `audio_emulator.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use jidr_client::Client;
use jidr_core::ErrorCode;
use jidr_daemon::testing::TestDaemon;
use serde_json::{Value, json};

/// A 16-bit PCM WAV of `frames` frames of silence.
fn wav16(rate: u32, channels: u16, frames: u32) -> Vec<u8> {
    let align = channels * 2;
    let data_len = frames * u32::from(align);
    let mut b = Vec::new();
    b.extend(b"RIFF");
    b.extend((36 + data_len).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(channels.to_le_bytes());
    b.extend(rate.to_le_bytes());
    b.extend((rate * u32::from(align)).to_le_bytes());
    b.extend(align.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(data_len.to_le_bytes());
    b.resize(b.len() + data_len as usize, 0);
    b
}

async fn inject(c: &Client, params: Value) -> jidr_core::JidrError {
    c.run("audio.inject", params, |_| {}).await.unwrap_err()
}

#[tokio::test]
async fn inject_checks_its_input_and_reports_an_unreachable_emulator() {
    let tmp = tempfile::tempdir().unwrap();
    let d = TestDaemon::start_with_providers(tmp.path(), vec![jidr_widgets::provider()])
        .await
        .unwrap();
    // A port nothing listens on: the emulator's gRPC is unreachable.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    d.backend.set_grpc_port(port);
    let c = Client::connect(&d.paths.socket(), "test").await.unwrap();

    let list = c.call("commands.list", json!({})).await.unwrap();
    let spec = |id: &str| {
        list.as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("{id} not listed"))
    };
    let inj = spec("audio.inject");
    assert_eq!(inj["longRunning"], true);
    assert_eq!(inj["destructive"], false);
    for key in ["device", "path", "loop", "sample_rate", "channels"] {
        assert!(
            inj["params"]["properties"].get(key).is_some(),
            "{key}: {inj}"
        );
    }
    assert_eq!(spec("audio.stop")["longRunning"], false);

    c.call("device.create", json!({ "name": "P", "image": "35" }))
        .await
        .unwrap();
    let wav = tmp.path().join("tone.wav");
    std::fs::write(&wav, wav16(16_000, 1, 1_600)).unwrap();

    // The device isn't running.
    let e = inject(&c, json!({ "device": "P", "path": wav })).await;
    assert_eq!(e.code, ErrorCode::NotFound, "{}", e.message);

    c.run("device.start", json!({ "device": "P" }), |_| {})
        .await
        .unwrap();

    // Files and params the microphone can't take are usage errors.
    let mp3 = tmp.path().join("song.mp3");
    std::fs::write(&mp3, b"ID3\x04\0\0\0\0\0\0").unwrap();
    let e = inject(&c, json!({ "device": "P", "path": mp3 })).await;
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("isn't a WAV file"), "{}", e.message);
    let hi = tmp.path().join("hi.wav");
    std::fs::write(&hi, wav16(96_000, 2, 96)).unwrap();
    let e = inject(&c, json!({ "device": "P", "path": hi })).await;
    assert_eq!(e.code, ErrorCode::Usage);
    assert!(e.message.contains("96000 Hz"), "{}", e.message);
    let e = inject(&c, json!({ "device": "P", "path": wav, "channels": 2 })).await;
    assert_eq!(e.code, ErrorCode::Usage);
    let e = inject(
        &c,
        json!({ "device": "P", "path": tmp.path().join("missing.wav") }),
    )
    .await;
    assert_eq!(e.code, ErrorCode::Usage);
    let e = inject(&c, json!({ "device": "P", "file": wav })).await;
    assert_eq!(e.code, ErrorCode::Usage);

    // A good file, but no emulator answers: `sdk`, and the injection is
    // forgotten (the next attempt isn't `busy`).
    for _ in 0..2 {
        let e = inject(&c, json!({ "device": "P", "path": wav })).await;
        assert_eq!(e.code, ErrorCode::Sdk, "{}", e.message);
    }
    let pcm = tmp.path().join("raw.pcm");
    std::fs::write(&pcm, [0u8; 3_200]).unwrap();
    let e = inject(
        &c,
        json!({ "device": "P", "path": pcm, "sample_rate": 16000 }),
    )
    .await;
    assert_eq!(e.code, ErrorCode::Sdk, "{}", e.message);

    // Nothing is playing.
    let v = c
        .call("audio.stop", json!({ "device": "P" }))
        .await
        .unwrap();
    assert_eq!(v["stopped"], false);
    let e = c
        .call("audio.stop", json!({ "device": "nope" }))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);

    d.stop().await;
}
