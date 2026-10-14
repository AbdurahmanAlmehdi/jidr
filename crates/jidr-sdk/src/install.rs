//! Native SDK package installer: download → SHA-1 check → unzip into a staging
//! directory → atomic move into the private SDK. No Java, no sdkmanager.
//! Only runs after the package's license has been accepted (D8).

use jidr_core::{Cancel, ErrorCode, JidrError, JidrResult, ResultExt};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::license::ConsentStore;
use crate::repo::{RemotePackage, Repository};
use crate::sdk::Sdk;

/// Extracted size is roughly this many times the archive size.
const EXTRACT_FACTOR: f64 = 2.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Downloading,
    Verifying,
    Extracting,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Progress {
    pub package: String,
    pub phase: Phase,
    pub done: u64,
    pub total: u64,
}

fn check_cancel(cancel: &Cancel) -> JidrResult<()> {
    cancel.check()
}

/// Lowercase hex digests of a download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digests {
    pub sha1: String,
    pub sha256: String,
}

/// Fetches small documents (repository XML) over https or `file://`.
#[derive(Debug, Clone)]
pub struct Fetcher {
    http: reqwest::Client,
}

impl Fetcher {
    pub fn new() -> JidrResult<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("jidr/", env!("CARGO_PKG_VERSION")))
            // A stalled network fails instead of hanging a command forever.
            .connect_timeout(std::time::Duration::from_secs(15))
            .read_timeout(std::time::Duration::from_secs(60))
            .build()
            .ctx(ErrorCode::Unexpected, "building HTTP client")?;
        Ok(Self { http })
    }

    pub async fn text(&self, url: &str) -> JidrResult<String> {
        if let Some(path) = url.strip_prefix("file://") {
            return tokio::fs::read_to_string(path)
                .await
                .ctx(ErrorCode::Sdk, format!("reading {url}"));
        }
        self.http
            .get(url)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .ctx(ErrorCode::Sdk, format!("fetching {url}"))?
            .text()
            .await
            .ctx(ErrorCode::Sdk, format!("reading {url}"))
    }

    /// Downloads `url` to `dest`, hashing as it goes.
    pub async fn download(
        &self,
        url: &str,
        dest: &Path,
        on_bytes: &(dyn Fn(u64, u64) + Send + Sync),
        cancel: &Cancel,
    ) -> JidrResult<Digests> {
        let mut hasher = Sha1::new();
        let mut hasher256 = sha2::Sha256::new();
        let file = std::fs::File::create(dest)
            .ctx(ErrorCode::Sdk, format!("creating {}", dest.display()))?;
        let mut out = std::io::BufWriter::new(file);
        if let Some(path) = url.strip_prefix("file://") {
            let bytes = tokio::fs::read(path)
                .await
                .ctx(ErrorCode::Sdk, format!("reading {url}"))?;
            check_cancel(cancel)?;
            hasher.update(&bytes);
            hasher256.update(&bytes);
            out.write_all(&bytes)
                .ctx(ErrorCode::Sdk, "writing download")?;
            on_bytes(bytes.len() as u64, bytes.len() as u64);
        } else {
            let mut resp = self
                .http
                .get(url)
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .ctx(ErrorCode::Sdk, format!("downloading {url}"))?;
            let total = resp.content_length().unwrap_or(0);
            let mut done = 0u64;
            while let Some(chunk) = resp
                .chunk()
                .await
                .ctx(ErrorCode::Sdk, format!("downloading {url}"))?
            {
                check_cancel(cancel)?;
                hasher.update(&chunk);
                hasher256.update(&chunk);
                out.write_all(&chunk)
                    .ctx(ErrorCode::Sdk, "writing download")?;
                done += chunk.len() as u64;
                on_bytes(done, total);
            }
        }
        out.flush().ctx(ErrorCode::Sdk, "writing download")?;
        Ok(Digests {
            sha1: hex::encode(hasher.finalize()),
            sha256: hex::encode(hasher256.finalize()),
        })
    }

    /// Fetches and merges every listing Jidr uses.
    pub async fn repository(&self, base: &str) -> JidrResult<Repository> {
        let mut repo = Repository::default();
        for url in crate::repo::listing_urls(base) {
            match self.text(&url).await {
                Ok(xml) => repo.merge(Repository::parse(&xml, &url)?),
                // The main listing is required; a missing image listing isn't fatal.
                Err(e) if url.contains("/sys-img/") => tracing::warn!("skipping {url}: {e}"),
                Err(e) => return Err(e),
            }
        }
        Ok(repo)
    }
}

/// Where a package lands inside the SDK: `a;b;c` → `a/b/c`.
pub fn install_dir(sdk: &Sdk, package: &str) -> PathBuf {
    package
        .split(';')
        .fold(sdk.root.clone(), |p, part| p.join(part))
}

/// Marker written into every package Jidr installs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledMarker {
    pub path: String,
    pub revision: String,
    pub sha1: String,
}

pub const MARKER_FILE: &str = ".jidr-package.json";

/// Installs one package into the private SDK.
pub async fn install(
    fetcher: &Fetcher,
    sdk: &Sdk,
    repo: &Repository,
    consent: &ConsentStore,
    pkg: &RemotePackage,
    progress: &(dyn Fn(Progress) + Send + Sync),
    cancel: &Cancel,
) -> JidrResult<PathBuf> {
    sdk.ensure_writable()?;
    if let Some(id) = &pkg.license
        && !consent.is_accepted(repo.license(id)?)
    {
        return Err(
            JidrError::sdk(format!("license {id} has not been accepted"))
                .with_data(serde_json::json!({"license": id, "package": pkg.path})),
        );
    }
    let archive = pkg
        .archive_for_host()
        .ok_or_else(|| JidrError::sdk(format!("{} has no archive for this host", pkg.path)))?;
    std::fs::create_dir_all(&sdk.root)
        .ctx(ErrorCode::Sdk, format!("creating {}", sdk.root.display()))?;
    crate::host::ensure_free_space(
        &sdk.root,
        archive.size + (archive.size as f64 * EXTRACT_FACTOR) as u64,
    )?;

    let staging_root = sdk.root.join(".jidr-staging");
    std::fs::create_dir_all(&staging_root).ctx(ErrorCode::Sdk, "creating staging dir")?;
    let id = uuid::Uuid::new_v4();
    let zip_path = staging_root.join(format!("{id}.zip"));
    let extract_dir = staging_root.join(id.to_string());
    let result = async {
        let report = |phase, done, total| {
            progress(Progress {
                package: pkg.path.clone(),
                phase,
                done,
                total,
            })
        };
        let size = archive.size;
        let sha1 = fetcher
            .download(
                &archive.url,
                &zip_path,
                &|done, total| {
                    report(
                        Phase::Downloading,
                        done,
                        if total > 0 { total } else { size },
                    )
                },
                cancel,
            )
            .await?
            .sha1;
        report(Phase::Verifying, size, size);
        if sha1 != archive.sha1 {
            return Err(JidrError::sdk(format!(
                "checksum mismatch for {}: expected {}, got {sha1}",
                archive.url, archive.sha1
            )));
        }
        check_cancel(cancel)?;
        report(Phase::Extracting, 0, size);
        let (zp, ed) = (zip_path.clone(), extract_dir.clone());
        let top = tokio::task::spawn_blocking(move || extract_single_root(&zp, &ed))
            .await
            .ctx(ErrorCode::Unexpected, "extract task")??;
        check_cancel(cancel)?;
        let marker = InstalledMarker {
            path: pkg.path.clone(),
            revision: pkg.revision.to_string(),
            sha1,
        };
        let marker_json =
            serde_json::to_vec_pretty(&marker).ctx(ErrorCode::Unexpected, "encoding marker")?;
        std::fs::write(top.join(MARKER_FILE), marker_json)
            .ctx(ErrorCode::Sdk, "writing package marker")?;

        let target = install_dir(sdk, &pkg.path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .ctx(ErrorCode::Sdk, format!("creating {}", parent.display()))?;
        }
        if target.exists() {
            let old = staging_root.join(format!("{id}.old"));
            std::fs::rename(&target, &old)
                .ctx(ErrorCode::Sdk, format!("moving old {}", target.display()))?;
        }
        std::fs::rename(&top, &target)
            .ctx(ErrorCode::Sdk, format!("installing {}", target.display()))?;
        report(Phase::Done, size, size);
        Ok(target)
    }
    .await;
    // This install's staging leftovers (zip, extract dir, replaced package) are
    // always removed; the staging root goes too once no other install uses it.
    let _ = std::fs::remove_file(&zip_path);
    let _ = std::fs::remove_dir_all(&extract_dir);
    let _ = std::fs::remove_dir_all(staging_root.join(format!("{id}.old")));
    let _ = std::fs::remove_dir(&staging_root);
    result
}

/// Extracts `zip` into `dest` and returns its single top-level directory.
fn extract_single_root(zip: &Path, dest: &Path) -> JidrResult<PathBuf> {
    let file =
        std::fs::File::open(zip).ctx(ErrorCode::Sdk, format!("opening {}", zip.display()))?;
    let mut archive = zip::ZipArchive::new(file).ctx(ErrorCode::Sdk, "reading zip")?;
    archive
        .extract(dest)
        .ctx(ErrorCode::Sdk, "extracting zip")?;
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dest)
        .ctx(ErrorCode::Sdk, "listing extracted files")?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n != "__MACOSX"))
        .collect();
    match (entries.pop(), entries.is_empty()) {
        (Some(top), true) if top.is_dir() => Ok(top),
        _ => Err(JidrError::sdk(
            "archive must contain exactly one top-level directory",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::Repository;
    use jidr_core::JidrPaths;

    /// Builds `emu.zip` with `emulator/emulator` + `emulator/lib/x.txt` and a
    /// listing pointing at it via file://.
    fn fixture(dir: &Path, corrupt: bool) -> (Repository, String) {
        let zip_path = dir.join("emu.zip");
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let opts = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
        zw.add_directory("emulator/", opts).unwrap();
        zw.start_file("emulator/emulator", opts).unwrap();
        zw.write_all(b"#!/bin/sh\necho emu\n").unwrap();
        zw.start_file("emulator/lib/x.txt", opts).unwrap();
        zw.write_all(b"x").unwrap();
        zw.finish().unwrap();
        let bytes = std::fs::read(&zip_path).unwrap();
        let sha1 = if corrupt {
            "00".repeat(20)
        } else {
            hex::encode(Sha1::digest(&bytes))
        };
        let xml = format!(
            r#"<r><license id="l" type="text">Terms.</license>
            <remotePackage path="emulator"><revision><major>36</major></revision>
            <uses-license ref="l"/><archives><archive><complete><size>{}</size>
            <checksum type="sha1">{sha1}</checksum><url>emu.zip</url></complete></archive></archives>
            </remotePackage></r>"#,
            bytes.len()
        );
        let url = format!("file://{}/repository2-3.xml", dir.display());
        (Repository::parse(&xml, &url).unwrap(), sha1)
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn installs_after_consent_only() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, _) = fixture(dir.path(), false);
        let paths = JidrPaths::under(dir.path().join("jidr"));
        let sdk = Sdk::private(&paths);
        let mut consent = ConsentStore::load(paths.consent_file()).unwrap();
        let pkg = repo.latest("emulator", 0).unwrap();
        let fetcher = Fetcher::new().unwrap();
        let cancel = Cancel::default();
        let phases = std::sync::Mutex::new(Vec::new());
        let progress = |p: Progress| phases.lock().unwrap().push(p.phase);

        let err = rt()
            .block_on(install(
                &fetcher, &sdk, &repo, &consent, pkg, &progress, &cancel,
            ))
            .unwrap_err();
        assert!(err.message.contains("not been accepted"), "{err}");
        assert!(!sdk.root.join("emulator").exists());

        consent.accept(repo.license("l").unwrap()).unwrap();
        let target = rt()
            .block_on(install(
                &fetcher, &sdk, &repo, &consent, pkg, &progress, &cancel,
            ))
            .unwrap();
        assert_eq!(target, sdk.root.join("emulator"));
        assert!(sdk.emulator().is_file());
        assert!(target.join("lib/x.txt").is_file());
        assert!(target.join(MARKER_FILE).is_file());
        assert!(!sdk.root.join(".jidr-staging").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(sdk.emulator())
                .unwrap()
                .permissions()
                .mode();
            assert!(mode & 0o111 != 0, "executable bit kept");
        }
        let seen = phases.lock().unwrap().clone();
        assert_eq!(seen.first(), Some(&Phase::Downloading));
        assert_eq!(seen.last(), Some(&Phase::Done));

        // Reinstall replaces the old directory.
        rt().block_on(install(
            &fetcher, &sdk, &repo, &consent, pkg, &progress, &cancel,
        ))
        .unwrap();
        assert!(sdk.emulator().is_file());
    }

    #[test]
    fn checksum_mismatch_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, _) = fixture(dir.path(), true);
        let paths = JidrPaths::under(dir.path().join("jidr"));
        let sdk = Sdk::private(&paths);
        let mut consent = ConsentStore::load(paths.consent_file()).unwrap();
        consent.accept(repo.license("l").unwrap()).unwrap();
        let pkg = repo.latest("emulator", 0).unwrap();
        let err = rt()
            .block_on(install(
                &Fetcher::new().unwrap(),
                &sdk,
                &repo,
                &consent,
                pkg,
                &|_| {},
                &Cancel::default(),
            ))
            .unwrap_err();
        assert!(err.message.contains("checksum mismatch"), "{err}");
        assert!(!sdk.root.join("emulator").exists());
        assert!(!sdk.root.join(".jidr-staging").exists());
    }

    #[test]
    fn cancel_stops_install() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, _) = fixture(dir.path(), false);
        let paths = JidrPaths::under(dir.path().join("jidr"));
        let sdk = Sdk::private(&paths);
        let mut consent = ConsentStore::load(paths.consent_file()).unwrap();
        consent.accept(repo.license("l").unwrap()).unwrap();
        let cancel = Cancel::new();
        cancel.cancel();
        let pkg = repo.latest("emulator", 0).unwrap();
        let err = rt()
            .block_on(install(
                &Fetcher::new().unwrap(),
                &sdk,
                &repo,
                &consent,
                pkg,
                &|_| {},
                &cancel,
            ))
            .unwrap_err();
        assert_eq!(err.data.unwrap()["cancelled"], true);
        assert!(!sdk.root.join("emulator").exists());
    }

    #[test]
    fn never_installs_into_studio_sdk() {
        let dir = tempfile::tempdir().unwrap();
        let (repo, _) = fixture(dir.path(), false);
        let sdk = Sdk::read_only(dir.path().join("studio"));
        let consent = ConsentStore::load(dir.path().join("c.json")).unwrap();
        let pkg = repo.latest("emulator", 0).unwrap();
        let err = rt()
            .block_on(install(
                &Fetcher::new().unwrap(),
                &sdk,
                &repo,
                &consent,
                pkg,
                &|_| {},
                &Cancel::default(),
            ))
            .unwrap_err();
        assert!(err.message.contains("never writes"), "{err}");
    }
}
