//! Host-side Magisk ramdisk patch.
//!
//! The patch mirrors the ramdisk edits Magisk's own `boot_patch.sh` applies
//! (read clean-room, never copied): `init` is replaced by `magiskinit`, the
//! compressed payload lands under `overlay.d/sbin`, and `.backup/.magisk`
//! records the config. The payload binaries are extracted from the pinned APK
//! (a release zip is also a valid APK), which is the same file `adb install`
//! uses for the manager app.
//!
//! We always patch the *pristine* `ramdisk.img` read fresh from the SDK (the
//! shared image is never written, D3), so patches never stack. The result is
//! written to a temp file and renamed into place, so a crash never leaves a
//! half-written image.

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use jidr_core::{ErrorCode, JidrError, JidrResult, ResultExt};
use jidr_cpio::compress::XzCheck;
use jidr_cpio::{Compression, Entry, Ramdisk};
use serde::{Deserialize, Serialize};

/// xz settings matching `magiskboot compress=xz` (preset 6, CRC32).
fn payload_xz() -> Compression {
    Compression::Xz {
        preset: 6,
        check: XzCheck::Crc32,
    }
}

/// What was extracted from the Magisk APK for one ABI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    pub abi: String,
    /// Replaces `/init` inside the ramdisk.
    pub magisk_init: Vec<u8>,
    /// The magisk binary, stored compressed in `overlay.d/sbin`.
    pub magisk: Vec<u8>,
    /// The `LD_PRELOAD` shim for `/init`.
    pub init_ld: Vec<u8>,
    /// The stub APK embedded in the ramdisk.
    pub stub: Vec<u8>,
}

/// Maps a system-image ABI onto the `lib/` directory name inside the APK.
pub fn normalize_abi(abi: &str) -> JidrResult<&'static str> {
    match abi {
        "arm64-v8a" => Ok("arm64-v8a"),
        "x86_64" => Ok("x86_64"),
        "x86" => Ok("x86"),
        "armeabi-v7a" => Ok("armeabi-v7a"),
        other => Err(JidrError::usage(format!(
            "no Magisk support for ABI {other:?} (expected arm64-v8a or x86_64)"
        ))),
    }
}

fn read_entry(archive: &mut zip::ZipArchive<Cursor<Vec<u8>>>, name: &str) -> JidrResult<Vec<u8>> {
    let mut f = archive
        .by_name(name)
        .map_err(|_| JidrError::sdk(format!("{name} is missing from the Magisk APK")))?;
    let mut out = Vec::new();
    f.read_to_end(&mut out).ctx(
        ErrorCode::Sdk,
        format!("reading {name} from the Magisk APK"),
    )?;
    Ok(out)
}

/// The Magisk version declared by the APK's `assets/util_functions.sh`
/// (`MAGISK_VER='30.7'`), used to name the patched ramdisk when a local APK is
/// supplied with `--magisk <path>`.
pub fn version_from_apk(apk: &Path) -> JidrResult<String> {
    let data = std::fs::read(apk).ctx(ErrorCode::Sdk, format!("reading {}", apk.display()))?;
    let mut archive = zip::ZipArchive::new(Cursor::new(data)).ctx(
        ErrorCode::Sdk,
        format!("{} is not a zip/APK", apk.display()),
    )?;
    let script = read_entry(&mut archive, "assets/util_functions.sh")?;
    let text = String::from_utf8_lossy(&script);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MAGISK_VER=") {
            let v = rest.trim().trim_matches(['\'', '"']);
            if !v.is_empty() {
                return Ok(v.to_string());
            }
        }
    }
    Err(JidrError::sdk(
        "the Magisk APK does not declare MAGISK_VER in util_functions.sh",
    ))
}

/// Extracts the binaries a ramdisk patch needs from a Magisk release APK.
///
/// The SHA-256 of the APK was already checked against the pin by the caller,
/// so the contents are trusted; this only verifies the package shape.
pub fn load_payload(apk: &Path, abi: &str) -> JidrResult<Payload> {
    let lib = normalize_abi(abi)?;
    let data = std::fs::read(apk).ctx(ErrorCode::Sdk, format!("reading {}", apk.display()))?;
    let mut archive = zip::ZipArchive::new(Cursor::new(data)).ctx(
        ErrorCode::Sdk,
        format!("{} is not a zip/APK", apk.display()),
    )?;

    let magisk_init = read_entry(&mut archive, &format!("lib/{lib}/libmagiskinit.so"))?;
    let magisk = read_entry(&mut archive, &format!("lib/{lib}/libmagisk.so"))?;
    let init_ld = read_entry(&mut archive, &format!("lib/{lib}/libinit-ld.so"))?;
    let stub = read_entry(&mut archive, "assets/stub.apk")?;

    Ok(Payload {
        abi: lib.to_string(),
        magisk_init,
        magisk,
        init_ld,
        stub,
    })
}

/// The `.backup/.magisk` config, as `boot_patch.sh` writes it for an emulator:
/// dm-verity and forced encryption are kept because the AVD relies on them.
fn config() -> Vec<u8> {
    b"KEEPVERITY=true\nKEEPFORCEENCRYPT=true\nRECOVERYMODE=false\nVENDORBOOT=false\n".to_vec()
}

/// Builds a patched ramdisk from pristine bytes and a payload. Non-empty
/// `props` also add the build-prop hook (Phase 9f): `overlay.d/jidr-props.rc`
/// and its script under `overlay.d/sbin`.
///
/// Only the first (generic) segment is edited: that is where the emulator's
/// `init` lives, and Magisk's own patcher operates on one cpio archive. The
/// vendor fragments are left byte-identical.
pub fn patch_ramdisk(
    pristine: &[u8],
    payload: &Payload,
    props: &crate::props::BuildProps,
) -> JidrResult<Vec<u8>> {
    let mut ramdisk = Ramdisk::parse(pristine)
        .map_err(|e| JidrError::root_failed(format!("parsing ramdisk.img: {e}")))?;
    let archive = &mut ramdisk
        .segments
        .first_mut()
        .ok_or_else(|| JidrError::root_failed("ramdisk.img has no cpio segments"))?
        .archive;

    // We always patch pristine; a pre-patched input means a caller handed us a
    // modified image (which would stack patches). Refuse rather than guess.
    if archive.contains(".backup/.magisk") || archive.contains("overlay.d") {
        return Err(JidrError::root_failed(
            "ramdisk already carries a Magisk patch; patch the pristine image only",
        ));
    }

    let compress = |data: &[u8]| -> JidrResult<Vec<u8>> {
        payload_xz()
            .compress(data)
            .map_err(|e| JidrError::root_failed(format!("compressing Magisk payload: {e}")))
    };

    let magisk_xz = compress(&payload.magisk)?;
    let stub_xz = compress(&payload.stub)?;
    let init_ld_xz = compress(&payload.init_ld)?;

    // Snapshot the stock archive before editing it, so the diff can be
    // recorded under `.backup/` — magiskinit re-execs the original `init` from
    // there, so without it the device never reaches adb.
    let original = archive.clone();

    // `insert` replaces `init` in place (keeping its inode) and appends the
    // rest before the trailer — the same edits `magiskboot cpio` performs.
    archive.insert(Entry::file("init", 0o750, payload.magisk_init.clone()));
    archive.insert(Entry::dir("overlay.d", 0o750));
    archive.insert(Entry::dir("overlay.d/sbin", 0o750));
    archive.insert(Entry::file("overlay.d/sbin/magisk.xz", 0o644, magisk_xz));
    archive.insert(Entry::file("overlay.d/sbin/stub.xz", 0o644, stub_xz));
    archive.insert(Entry::file("overlay.d/sbin/init-ld.xz", 0o644, init_ld_xz));
    if !props.is_empty() {
        use crate::props::{RC, SCRIPT, rc};
        archive.insert(Entry::file(
            format!("overlay.d/{RC}"),
            0o644,
            rc().into_bytes(),
        ));
        archive.insert(Entry::file(
            format!("overlay.d/sbin/{SCRIPT}"),
            0o755,
            props.script().into_bytes(),
        ));
    }
    backup(archive, &original)?;
    archive.insert(Entry::dir(".backup", 0));
    archive.insert(Entry::file(".backup/.magisk", 0, config()));

    ramdisk
        .to_bytes()
        .map_err(|e| JidrError::root_failed(format!("serializing the patched ramdisk: {e}")))
}

/// Ports `magiskboot cpio backup ORIG`: records the difference between the
/// patched archive and the stock archive under `.backup/`, so magiskinit can
/// restore the original entries at boot — above all the original `init`, which
/// it re-execs as the real first-stage init. Entries the patch introduced are
/// listed in `.backup/.rmlist` so a Magisk restore can remove them again.
fn backup(patched: &mut jidr_cpio::Archive, original: &jidr_cpio::Archive) -> JidrResult<()> {
    patched.insert(Entry::dir(".backup", 0));

    // Stock entries whose content changed (or which were removed) are stored
    // under `.backup/<name>`, xz-compressed when they are regular files.
    for entry in original.iter() {
        if entry.name.starts_with(".backup") {
            continue;
        }
        let unchanged = patched
            .get(&entry.name)
            .is_some_and(|p| p.data == entry.data);
        if unchanged {
            continue;
        }
        let name = format!(".backup/{}", entry.name);
        if entry.is_file() {
            let compressed = payload_xz().compress(&entry.data).map_err(|e| {
                JidrError::root_failed(format!("compressing {} for backup: {e}", entry.name))
            })?;
            patched.insert(Entry::file(
                format!("{name}.xz"),
                entry.permissions(),
                compressed,
            ));
        } else {
            let mut copy = entry.clone();
            copy.name = name;
            patched.insert(copy);
        }
    }

    // Entries the patch introduced are listed so a restore can delete them.
    let mut rmlist = Vec::new();
    for entry in patched.iter() {
        if entry.name.starts_with(".backup") {
            continue;
        }
        if original.get(&entry.name).is_none() {
            rmlist.extend_from_slice(entry.name.as_bytes());
            rmlist.push(0);
        }
    }
    if !rmlist.is_empty() {
        patched.insert(Entry::file(".backup/.rmlist", 0, rmlist));
    }
    Ok(())
}

/// Writes `bytes` to `path` atomically: a temp file in the same directory is
/// renamed into place, so a crash never leaves a half-written image.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> JidrResult<()> {
    let dir = path
        .parent()
        .ok_or_else(|| JidrError::unexpected(format!("{} has no parent", path.display())))?;
    std::fs::create_dir_all(dir).ctx(ErrorCode::Sdk, format!("creating {}", dir.display()))?;
    let tmp = dir.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, bytes).ctx(ErrorCode::Sdk, format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).ctx(ErrorCode::Sdk, format!("writing {}", path.display()))
}

/// The provenance a patched ramdisk records, so a later start can tell whether
/// the pristine image or the pinned release changed (and re-patch).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub magisk_version: String,
    pub pristine_sha256: String,
    /// The APK the payload came from, so a props change re-patches with the
    /// same release (absent in patches made before Phase 9f).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apk_sha256: Option<String>,
    /// The build props baked in (Phase 9f); stock when absent.
    #[serde(default, skip_serializing_if = "crate::props::BuildProps::is_empty")]
    pub props: crate::props::BuildProps,
}

/// The sidecar metadata file next to a patched ramdisk.
pub fn provenance_path(patched: &Path) -> PathBuf {
    let mut name = patched
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "ramdisk.magisk.img".to_string());
    name.push_str(".json");
    patched.with_file_name(name)
}

pub fn read_provenance(patched: &Path) -> Option<Provenance> {
    let text = std::fs::read_to_string(provenance_path(patched)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_provenance(patched: &Path, provenance: &Provenance) -> JidrResult<()> {
    let text = serde_json::to_vec_pretty(provenance)
        .map_err(|e| JidrError::unexpected(format!("encoding provenance: {e}")))?;
    write_atomic(&provenance_path(patched), &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal pristine ramdisk (a single uncompressed cpio segment).
    fn pristine_ramdisk() -> Vec<u8> {
        let mut a = jidr_cpio::Archive::new();
        a.insert(Entry::file("init", 0o750, b"#!stock init".to_vec()));
        a.insert(Entry::dir("dev", 0o755));
        let ramdisk = Ramdisk {
            segments: vec![jidr_cpio::Segment {
                compression: Compression::None,
                archive: a,
            }],
            trailing: Vec::new(),
        };
        ramdisk.to_bytes().unwrap()
    }

    fn payload() -> Payload {
        Payload {
            abi: "arm64-v8a".into(),
            magisk_init: b"MAGISKINIT".to_vec(),
            magisk: b"MAGISK".to_vec(),
            init_ld: b"INITLD".to_vec(),
            stub: b"STUBAPK".to_vec(),
        }
    }

    #[test]
    fn patch_replaces_init_and_adds_magisk_entries() {
        let patched = patch_ramdisk(&pristine_ramdisk(), &payload(), &Default::default()).unwrap();
        let ramdisk = Ramdisk::parse(&patched).unwrap();
        let (idx, init) = ramdisk.get("init").unwrap();
        assert_eq!(idx, 0);
        assert_eq!(init.data, b"MAGISKINIT");
        assert_eq!(init.permissions(), 0o750);

        let (_, overlay) = ramdisk.get("overlay.d").unwrap();
        assert!(overlay.is_dir());
        assert_eq!(overlay.permissions(), 0o750);

        let (_, magisk) = ramdisk.get("overlay.d/sbin/magisk.xz").unwrap();
        assert!(magisk.is_file());
        // The payload is xz-compressed inside the ramdisk.
        assert!(
            magisk
                .data
                .starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00])
        );

        let (_, cfg) = ramdisk.get(".backup/.magisk").unwrap();
        assert!(String::from_utf8_lossy(&cfg.data).contains("KEEPVERITY=true"));
    }

    #[test]
    fn patch_backs_up_the_original_init_for_magiskinit() {
        // Regression: without `.backup/init.xz`, magiskinit's restore step
        // falls back to symlinking `/init` -> `/system/bin/init`, which skips
        // the emulator's real first-stage init and the device never reaches
        // adb (Phase 9d e2e hang).
        let pristine = pristine_ramdisk();
        let patched = patch_ramdisk(&pristine, &payload(), &Default::default()).unwrap();
        let ramdisk = Ramdisk::parse(&patched).unwrap();

        let (_, backup) = ramdisk.get(".backup/init.xz").unwrap();
        assert!(backup.is_file());
        assert!(
            backup
                .data
                .starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]),
            "the backup init is xz-compressed"
        );
        let decoded = jidr_cpio::compress::decompress(&backup.data).unwrap();
        assert_eq!(decoded.data, b"#!stock init", "the original init is kept");

        // Entries the patch added are listed for restore.
        let (_, rmlist) = ramdisk.get(".backup/.rmlist").unwrap();
        let mut names: Vec<&str> = rmlist
            .data
            .split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| std::str::from_utf8(s).unwrap())
            .collect();
        names.sort_unstable();
        assert!(names.contains(&"overlay.d"), "{names:?}");
        assert!(names.contains(&"overlay.d/sbin/magisk.xz"), "{names:?}");

        // The stock entries that were not touched stay out of the backup.
        assert!(ramdisk.get(".backup/dev.xz").is_none());
    }

    #[test]
    fn patch_is_idempotent_and_deterministic() {
        let a = patch_ramdisk(&pristine_ramdisk(), &payload(), &Default::default()).unwrap();
        let b = patch_ramdisk(&pristine_ramdisk(), &payload(), &Default::default()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn patch_refuses_an_already_patched_ramdisk() {
        let once = patch_ramdisk(&pristine_ramdisk(), &payload(), &Default::default()).unwrap();
        let err = patch_ramdisk(&once, &payload(), &Default::default()).unwrap_err();
        assert_eq!(err.code, ErrorCode::RootFailed);
        assert!(err.message.contains("pristine"), "{err}");
    }

    #[test]
    fn atomic_write_and_provenance_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ramdisk.magisk-30.7.img");
        write_atomic(&path, b"hello").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");

        let prov = Provenance {
            magisk_version: "30.7".into(),
            pristine_sha256: "a".repeat(64),
            apk_sha256: Some("b".repeat(64)),
            props: crate::props::BuildProps {
                model: Some("Pixel 8".into()),
                ..Default::default()
            },
        };
        write_provenance(&path, &prov).unwrap();
        assert_eq!(read_provenance(&path).as_ref(), Some(&prov));
        assert!(provenance_path(&path).ends_with("ramdisk.magisk-30.7.img.json"));
        assert!(read_provenance(&dir.path().join("missing.img")).is_none());
    }

    /// Phase 9f: props add the rc hook and its script, both listed in
    /// `.rmlist` so a Magisk restore removes them; stock adds neither.
    #[test]
    fn props_add_the_boot_hook() {
        let props = crate::props::BuildProps {
            model: Some("Pixel 8".into()),
            ..Default::default()
        };
        let patched = patch_ramdisk(&pristine_ramdisk(), &payload(), &props).unwrap();
        let ramdisk = Ramdisk::parse(&patched).unwrap();
        let a = &ramdisk.segments[0].archive;
        let rc = a.get("overlay.d/jidr-props.rc").unwrap();
        assert!(String::from_utf8_lossy(&rc.data).contains("on post-fs-data"));
        let script = a.get("overlay.d/sbin/jidr-props.sh").unwrap();
        assert_eq!(script.permissions(), 0o755);
        assert!(String::from_utf8_lossy(&script.data).contains("ro.product.model 'Pixel 8'"));
        let rmlist = String::from_utf8_lossy(&a.get(".backup/.rmlist").unwrap().data).into_owned();
        assert!(rmlist.contains("overlay.d/jidr-props.rc"), "{rmlist:?}");
        assert!(
            rmlist.contains("overlay.d/sbin/jidr-props.sh"),
            "{rmlist:?}"
        );

        let stock = patch_ramdisk(&pristine_ramdisk(), &payload(), &Default::default()).unwrap();
        let stock = Ramdisk::parse(&stock).unwrap();
        assert!(
            !stock.segments[0]
                .archive
                .contains("overlay.d/jidr-props.rc")
        );
    }

    #[test]
    fn abi_mapping() {
        assert_eq!(normalize_abi("arm64-v8a").unwrap(), "arm64-v8a");
        assert_eq!(normalize_abi("x86_64").unwrap(), "x86_64");
        assert!(normalize_abi("mips").is_err());
    }
}
