//! Generates the EmulatorController client from the installed SDK's proto.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sdk = std::env::var_os("ANDROID_SDK_ROOT")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Android/sdk")))
        .ok_or("set ANDROID_SDK_ROOT")?;
    let dir = sdk.join("emulator/lib");
    let proto = dir.join("emulator_controller.proto");
    println!("cargo:rerun-if-changed={}", proto.display());
    tonic_prost_build::configure()
        .build_server(false)
        .compile_protos(&[proto], &[dir])?;
    Ok(())
}
