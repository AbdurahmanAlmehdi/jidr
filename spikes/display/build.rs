//! Generates the EmulatorController client from the installed SDK's proto,
//! falling back to the copy vendored in `crates/jidr-proto/proto/` (CI runners
//! have an SDK without the emulator).

use std::path::{Path, PathBuf};

fn proto_dir() -> PathBuf {
    let sdk = std::env::var_os("ANDROID_SDK_ROOT")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Android/sdk")));
    sdk.map(|s| s.join("emulator/lib"))
        .filter(|d| d.join("emulator_controller.proto").is_file())
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/jidr-proto/proto"))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-env-changed=ANDROID_SDK_ROOT");
    let dir = proto_dir();
    let proto = dir.join("emulator_controller.proto");
    println!("cargo:rerun-if-changed={}", proto.display());
    tonic_prost_build::configure()
        .build_server(false)
        .compile_protos(&[proto], &[dir])?;
    Ok(())
}
