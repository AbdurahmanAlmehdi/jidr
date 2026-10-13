//! Generates the gRPC client/server code from the installed SDK emulator's
//! protos, falling back to the vendored copies in `proto/`.

use std::path::{Path, PathBuf};

const FILES: &[&str] = &["emulator_controller.proto", "snapshot_service.proto"];

fn sdk_proto_dir() -> Option<PathBuf> {
    if std::env::var_os("JIDR_PROTO_VENDORED").is_some_and(|v| v == "1") {
        return None;
    }
    let roots = ["ANDROID_SDK_ROOT", "ANDROID_HOME"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .chain(std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Android/sdk")));
    roots.map(|r| r.join("emulator/lib")).find(|d| {
        FILES
            .iter()
            .chain(["snapshot.proto"].iter())
            .all(|f| d.join(f).is_file())
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-env-changed=JIDR_PROTO_VENDORED");
    println!("cargo:rerun-if-env-changed=ANDROID_SDK_ROOT");
    println!("cargo:rerun-if-env-changed=ANDROID_HOME");
    let vendored = Path::new(env!("CARGO_MANIFEST_DIR")).join("proto");
    let (dir, source) = match sdk_proto_dir() {
        Some(d) => (d, "sdk"),
        None => (vendored, "vendored"),
    };
    for f in FILES.iter().chain(["snapshot.proto"].iter()) {
        println!("cargo:rerun-if-changed={}", dir.join(f).display());
    }
    println!("cargo:rustc-env=JIDR_PROTO_SOURCE={source}");
    println!("cargo:rustc-env=JIDR_PROTO_DIR={}", dir.display());
    let files: Vec<PathBuf> = FILES.iter().map(|f| dir.join(f)).collect();
    tonic_prost_build::configure()
        .build_server(true)
        .compile_protos(&files, &[dir])?;
    Ok(())
}
