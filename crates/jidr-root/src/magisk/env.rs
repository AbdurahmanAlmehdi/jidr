//! Magisk's runtime environment under `/data/adb/magisk`.
//!
//! A patched ramdisk carries Magisk's init and daemon and nothing else. The
//! rest (the `magisk` binary, `magiskpolicy`, `busybox`, the manager scripts)
//! lives in the app and is unpacked into `/data/adb/magisk` before `su` works.
//! We reproduce that unpack on the host and push it up, so the setup stays
//! headless on `google_apis` images (which allow `adb root`).

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use jidr_adb::Adb;
use jidr_core::{ErrorCode, JidrError, JidrResult, ResultExt};

/// Where Magisk's runtime files live on the device.
pub const MAGISK_ENV_DIR: &str = "/data/adb/magisk";
/// A staging path the files are pushed to before being installed, so a failed
/// transfer cannot leave `/data/adb/magisk` half written.
pub const MAGISK_ENV_STAGING: &str = "/data/local/tmp/jidr-magisk-env";

/// One file of Magisk's runtime environment, named as it must appear on the
/// device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvFile {
    pub name: String,
    pub data: Vec<u8>,
}

/// Assets copied verbatim (named without the `assets/` prefix), as the manager
/// app does.
const ENV_ASSETS: &[&str] = &[
    "assets/util_functions.sh",
    "assets/boot_patch.sh",
    "assets/addon.d.sh",
    "assets/stub.apk",
];

/// ChromeOS flashing tools, in a subdirectory.
const ENV_CHROMEOS: &[&str] = &["futility", "kernel_data_key.vbprivk", "kernel.keyblock"];

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

/// Extracts the files the manager app unpacks into `/data/adb/magisk`.
///
/// The rule for native libraries is the app's own: `lib/<abi>/libmagisk.so`
/// becomes `magisk` (drop the `lib` prefix and the `.so` suffix). Four assets
/// and the ChromeOS tools are copied as they are.
pub fn environment_files(apk: &Path, abi: &str) -> JidrResult<Vec<EnvFile>> {
    let lib = super::patch::normalize_abi(abi)?;
    let data = std::fs::read(apk).ctx(ErrorCode::Sdk, format!("reading {}", apk.display()))?;
    let mut archive = zip::ZipArchive::new(Cursor::new(data)).ctx(
        ErrorCode::Sdk,
        format!("{} is not a zip/APK", apk.display()),
    )?;

    let mut files = Vec::new();
    let prefix = format!("lib/{lib}/");
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .ctx(ErrorCode::Sdk, "listing the Magisk APK")?;
        let entry_name = entry
            .name()
            .map_err(|e| JidrError::sdk(format!("reading a zip entry name: {e}")))?
            .into_owned();
        if entry.is_dir() || !entry_name.starts_with(&prefix) || !entry_name.ends_with(".so") {
            continue;
        }
        let trimmed = entry_name
            .strip_prefix(&prefix)
            .and_then(|n| n.strip_suffix(".so"))
            .unwrap_or_default()
            .to_string();
        // The app keeps only names that start with `lib` and are longer than
        // it; `libbootctl.so` → `bootctl`.
        if trimmed.len() <= 3 || !trimmed.starts_with("lib") {
            continue;
        }
        let name = trimmed[3..].to_string();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).ctx(
            ErrorCode::Sdk,
            format!("reading {entry_name} from the Magisk APK"),
        )?;
        files.push(EnvFile { name, data });
    }

    if files.is_empty() {
        return Err(JidrError::sdk(format!(
            "the Magisk APK carries no {lib} binaries, so the environment cannot be built"
        )));
    }
    for asset in ENV_ASSETS {
        let data = read_entry(&mut archive, asset)?;
        files.push(EnvFile {
            name: asset.strip_prefix("assets/").unwrap_or(asset).to_string(),
            data,
        });
    }
    for tool in ENV_CHROMEOS {
        let path = format!("assets/chromeos/{tool}");
        let data = read_entry(&mut archive, &path)?;
        files.push(EnvFile {
            name: format!("chromeos/{tool}"),
            data,
        });
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(files)
}

/// Removes a staging directory when dropped.
struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Pushes `files` to the device and installs them under `/data/adb/magisk`.
///
/// The caller must have already run `adb root` (writing `/data/adb` needs it);
/// on images that refuse `adb root` this returns an error and the guided-tap
/// path takes over.
pub async fn push_environment(adb: &Adb, files: &[EnvFile]) -> JidrResult<()> {
    let staging = std::env::temp_dir().join(format!("jidr-magisk-env-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&staging)
        .ctx(ErrorCode::Sdk, format!("creating {}", staging.display()))?;
    let _cleanup = Cleanup(staging.clone());

    for f in files {
        let path = staging.join(&f.name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .ctx(ErrorCode::Sdk, format!("creating {}", parent.display()))?;
        }
        std::fs::write(&path, &f.data)
            .ctx(ErrorCode::Sdk, format!("writing {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .ctx(ErrorCode::Sdk, format!("chmod {}", path.display()))?;
        }
    }

    adb.push(&staging.join("."), MAGISK_ENV_STAGING).await?;

    let script = [
        "set -e",
        &format!("rm -rf {MAGISK_ENV_DIR}"),
        &format!("mkdir -p {MAGISK_ENV_DIR}"),
        "chmod 700 /data/adb",
        &format!("cp -af {MAGISK_ENV_STAGING}/. {MAGISK_ENV_DIR}/"),
        &format!("chmod -R 755 {MAGISK_ENV_DIR}"),
        &format!("chown -R 0:0 {MAGISK_ENV_DIR}"),
        "mkdir -p /data/adb/modules /data/adb/post-fs-data.d /data/adb/service.d",
        &format!("rm -rf {MAGISK_ENV_STAGING}"),
        // The caller cold-boots next; without this the files may only exist in
        // the page cache and vanish right when magiskd needs them.
        "sync",
    ]
    .join(" && ");

    let out = adb
        .shell(&script)
        .await
        .map_err(|e| JidrError::root_failed(format!("installing Magisk's environment: {e}")))?;
    if out.trim().is_empty() {
        Ok(())
    } else {
        // `shell` only errors on non-zero exit; any stray output is noise.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic Magisk-like APK (zip) with the lib files and assets the
    /// environment extractor needs.
    fn fake_apk(dir: &Path) -> PathBuf {
        let path = dir.join("Magisk-v30.7.apk");
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();
        for name in [
            "lib/arm64-v8a/libmagisk.so",
            "lib/arm64-v8a/libmagiskinit.so",
            "lib/arm64-v8a/libmagiskpolicy.so",
            "lib/arm64-v8a/libbootctl.so",
            "assets/util_functions.sh",
            "assets/boot_patch.sh",
            "assets/addon.d.sh",
            "assets/stub.apk",
            "assets/chromeos/futility",
            "assets/chromeos/kernel_data_key.vbprivk",
            "assets/chromeos/kernel.keyblock",
        ] {
            zip.start_file(name, opts).unwrap();
            use std::io::Write;
            zip.write_all(b"x").unwrap();
        }
        zip.finish().unwrap();
        path
    }

    #[test]
    fn extracts_native_binaries_and_assets() {
        let dir = tempfile::tempdir().unwrap();
        let apk = fake_apk(dir.path());
        let files = environment_files(&apk, "arm64-v8a").unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"magisk"));
        assert!(names.contains(&"magiskinit"));
        assert!(names.contains(&"magiskpolicy"));
        assert!(names.contains(&"bootctl"));
        assert!(names.contains(&"util_functions.sh"));
        assert!(names.contains(&"boot_patch.sh"));
        assert!(names.contains(&"stub.apk"));
        assert!(names.contains(&"chromeos/futility"));
        // The sorted order is deterministic.
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
    }

    #[test]
    fn rejects_a_wrong_abi_with_no_binaries() {
        let dir = tempfile::tempdir().unwrap();
        let apk = fake_apk(dir.path());
        assert!(environment_files(&apk, "x86_64").is_err());
    }
}
