//! Byte-identical round-trips: decompress → parse → write → compress must
//! reproduce the input exactly, for fixtures made by the reference tools
//! (tests/fixtures/make.sh) and for every ramdisk in the local SDK.

// Test helpers outside #[test] fns aren't covered by clippy.toml's allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use jidr_cpio::{Compression, Entry, Ramdisk};

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

fn assert_round_trip(bytes: &[u8]) -> Ramdisk {
    let ramdisk = Ramdisk::parse(bytes).unwrap();
    let written = ramdisk.to_bytes().unwrap();
    assert!(
        written == bytes,
        "round-trip differs ({} vs {} bytes)",
        written.len(),
        bytes.len()
    );
    ramdisk
}

#[test]
fn reference_lz4_legacy_with_two_blocks() {
    let bytes = fixture("newc.cpio.lz4");
    let ramdisk = assert_round_trip(&bytes);
    assert_eq!(ramdisk.segments.len(), 1);
    assert_eq!(ramdisk.segments[0].compression, Compression::lz4_legacy());
    let big = ramdisk.segments[0].archive.get("system/big.txt").unwrap();
    assert_eq!(big.data.len(), 9 << 20);
}

#[test]
fn reference_gzip() {
    let ramdisk = assert_round_trip(&fixture("newc.cpio.gz"));
    assert!(matches!(
        ramdisk.segments[0].compression,
        Compression::Gzip { level: 6, .. }
    ));
}

#[test]
fn reference_xz() {
    let ramdisk = assert_round_trip(&fixture("newc.cpio.xz"));
    assert_eq!(ramdisk.segments[0].compression, Compression::xz());
}

#[test]
fn reference_archive_contents() {
    let ramdisk = assert_round_trip(&fixture("newc.cpio.lz4"));
    let archive = &ramdisk.segments[0].archive;
    let init = archive.get("init").unwrap();
    assert!(init.is_file());
    assert_eq!(init.permissions(), 0o750);
    assert_eq!(init.data, b"#!/system/bin/sh\necho jidr fixture\n");
    let sh = archive.get("system/bin/sh").unwrap();
    assert!(sh.is_symlink());
    assert_eq!(sh.data, b"/system/bin/sh");
    assert!(archive.get("dev").unwrap().is_dir());
}

#[test]
fn emulator_layout_of_concatenated_streams() {
    let ramdisk = assert_round_trip(&fixture("two-streams.img"));
    assert_eq!(ramdisk.segments.len(), 2);
    assert_eq!(ramdisk.get("init").unwrap().0, 0);
    assert_eq!(ramdisk.get("lib/modules/virtio.ko").unwrap().0, 1);
}

#[test]
fn magisk_style_edit_keeps_the_rest_byte_identical() {
    let bytes = fixture("two-streams.img");
    let mut ramdisk = Ramdisk::parse(&bytes).unwrap();
    let vendor_before = ramdisk.segments[1].to_bytes().unwrap();

    let first = &mut ramdisk.segments[0].archive;
    let original_init = first.get("init").unwrap().data.clone();
    first.insert(Entry::dir(".backup", 0o000));
    first.rename("init", ".backup/init").unwrap();
    first.insert(Entry::file("init", 0o750, b"new init".to_vec()));
    first.insert(Entry::dir("overlay.d", 0o750));
    first.insert(Entry::file("overlay.d/sbin/x.xz", 0o644, vec![1, 2, 3]));

    let patched = Ramdisk::parse(&ramdisk.to_bytes().unwrap()).unwrap();
    let first = &patched.segments[0].archive;
    assert_eq!(first.get(".backup/init").unwrap().data, original_init);
    assert_eq!(first.get("init").unwrap().data, b"new init");
    assert!(first.get("overlay.d").unwrap().is_dir());
    assert_eq!(patched.segments[1].to_bytes().unwrap(), vendor_before);
}

/// Every `ramdisk.img` under `$JIDR_TEST_SDK/system-images` (default: Android
/// Studio's SDK, opened read-only). Passes with a note when no SDK exists, so
/// the fixtures above carry the guarantee on CI.
#[test]
fn every_sdk_ramdisk_round_trips_byte_identically() {
    let sdk = std::env::var_os("JIDR_TEST_SDK")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Android/sdk")));
    let Some(images) = sdk.map(|s| s.join("system-images")).filter(|p| p.is_dir()) else {
        println!("no SDK system-images found; skipping real-image round-trips");
        return;
    };
    let mut checked = Vec::new();
    for api in read_dirs(&images) {
        for tag in read_dirs(&api) {
            for abi in read_dirs(&tag) {
                let path = abi.join("ramdisk.img");
                if !path.is_file() {
                    continue;
                }
                let bytes = std::fs::read(&path).unwrap();
                let ramdisk = assert_round_trip(&bytes);
                println!(
                    "{}: {} segment(s), {}",
                    path.display(),
                    ramdisk.segments.len(),
                    ramdisk.segments[0].compression.name()
                );
                checked.push(path);
            }
        }
    }
    println!(
        "{} SDK ramdisk(s) round-tripped byte-identically",
        checked.len()
    );
}

fn read_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<_> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs
}
