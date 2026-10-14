//! Content-addressed download cache (D30): files are stored and looked up by
//! SHA-256, so a pinned Magisk APK is downloaded once, verified every time
//! and usable offline (`--magisk <path>` imports a local copy).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use jidr_core::{Cancel, ErrorCode, JidrError, JidrResult, ResultExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::install::Fetcher;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CacheEntry {
    pub sha256: String,
    pub path: PathBuf,
    pub size: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PruneReport {
    pub removed: usize,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
pub struct DownloadCache {
    dir: PathBuf,
}

fn valid_hex(sha256: &str) -> JidrResult<String> {
    let h = sha256.trim().to_ascii_lowercase();
    if h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(h)
    } else {
        Err(JidrError::usage(format!(
            "{sha256:?} is not a SHA-256 hex digest"
        )))
    }
}

/// SHA-256 of a file, streamed.
pub fn sha256_file(path: &Path) -> JidrResult<String> {
    let mut f =
        std::fs::File::open(path).ctx(ErrorCode::Usage, format!("opening {}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f
            .read(&mut buf)
            .ctx(ErrorCode::Sdk, format!("reading {}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

impl DownloadCache {
    /// The cache under `paths.downloads()`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn root(&self) -> PathBuf {
        self.dir.join("sha256")
    }

    /// Where a file with this hash lives (the extension is cosmetic).
    pub fn path_for(&self, sha256: &str, ext: Option<&str>) -> JidrResult<PathBuf> {
        let h = valid_hex(sha256)?;
        let name =
            match ext.filter(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric())) {
                Some(e) => format!("{h}.{e}"),
                None => h,
            };
        Ok(self.root().join(name))
    }

    /// A cached file with this hash, if present.
    pub fn get(&self, sha256: &str) -> JidrResult<Option<PathBuf>> {
        let h = valid_hex(sha256)?;
        let Ok(rd) = std::fs::read_dir(self.root()) else {
            return Ok(None);
        };
        Ok(rd.flatten().map(|e| e.path()).find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == h || n.split('.').next() == Some(h.as_str()))
        }))
    }

    /// A cached file with this hash whose content still matches it.
    pub fn get_verified(&self, sha256: &str) -> JidrResult<Option<PathBuf>> {
        let Some(p) = self.get(sha256)? else {
            return Ok(None);
        };
        if sha256_file(&p)? == valid_hex(sha256)? {
            Ok(Some(p))
        } else {
            tracing::warn!(path = %p.display(), "corrupt cache entry removed");
            let _ = std::fs::remove_file(&p);
            Ok(None)
        }
    }

    /// Downloads `url` unless a verified copy is cached; the content must hash
    /// to `sha256` or nothing is stored.
    pub async fn fetch(
        &self,
        fetcher: &Fetcher,
        url: &str,
        sha256: &str,
        on_bytes: &(dyn Fn(u64, u64) + Send + Sync),
        cancel: &Cancel,
    ) -> JidrResult<PathBuf> {
        let want = valid_hex(sha256)?;
        if let Some(p) = self.get_verified(&want)? {
            return Ok(p);
        }
        std::fs::create_dir_all(self.root()).ctx(ErrorCode::Sdk, "creating download cache")?;
        let ext = Path::new(url.split(['?', '#']).next().unwrap_or(url))
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_string);
        let tmp = self.root().join(format!(".{}.part", uuid::Uuid::new_v4()));
        let result = fetcher.download(url, &tmp, on_bytes, cancel).await;
        let digests = match result {
            Ok(d) => d,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        };
        if digests.sha256 != want {
            let _ = std::fs::remove_file(&tmp);
            return Err(JidrError::sdk(format!(
                "checksum mismatch for {url}: expected sha256 {want}, got {}",
                digests.sha256
            )));
        }
        let dest = self.path_for(&want, ext.as_deref())?;
        std::fs::rename(&tmp, &dest).ctx(ErrorCode::Sdk, "storing download")?;
        Ok(dest)
    }

    /// Copies a local file into the cache (offline installs). If `expected`
    /// is given, the file must hash to it.
    pub fn import(&self, path: &Path, expected: Option<&str>) -> JidrResult<CacheEntry> {
        let actual = sha256_file(path)?;
        if let Some(e) = expected {
            let e = valid_hex(e)?;
            if e != actual {
                return Err(JidrError::usage(format!(
                    "{} has sha256 {actual}, expected {e}",
                    path.display()
                )));
            }
        }
        std::fs::create_dir_all(self.root()).ctx(ErrorCode::Sdk, "creating download cache")?;
        let ext = path.extension().and_then(|e| e.to_str());
        let dest = match self.get(&actual)? {
            Some(p) => p,
            None => {
                let dest = self.path_for(&actual, ext)?;
                let tmp = self.root().join(format!(".{}.part", uuid::Uuid::new_v4()));
                std::fs::copy(path, &tmp)
                    .ctx(ErrorCode::Sdk, format!("copying {}", path.display()))?;
                std::fs::rename(&tmp, &dest).ctx(ErrorCode::Sdk, "storing import")?;
                dest
            }
        };
        let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
        Ok(CacheEntry {
            sha256: actual,
            path: dest,
            size,
        })
    }

    pub fn list(&self) -> Vec<CacheEntry> {
        let Ok(rd) = std::fs::read_dir(self.root()) else {
            return Vec::new();
        };
        let mut out: Vec<CacheEntry> = rd
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let name = path.file_name()?.to_str()?.to_string();
                let sha256 = name.split('.').next()?.to_string();
                valid_hex(&sha256).ok()?;
                Some(CacheEntry {
                    size: e.metadata().map(|m| m.len()).unwrap_or(0),
                    sha256,
                    path,
                })
            })
            .collect();
        out.sort_by(|a, b| a.sha256.cmp(&b.sha256));
        out
    }

    /// Removes entries (and stale partial downloads) not modified within
    /// `older_than`; `None` removes everything.
    pub fn prune(&self, older_than: Option<Duration>) -> PruneReport {
        let mut report = PruneReport::default();
        let Ok(rd) = std::fs::read_dir(self.root()) else {
            return report;
        };
        let now = SystemTime::now();
        for e in rd.flatten() {
            let Ok(meta) = e.metadata() else { continue };
            let old_enough = match older_than {
                None => true,
                Some(age) => meta
                    .modified()
                    .ok()
                    .and_then(|m| now.duration_since(m).ok())
                    .is_some_and(|d| d >= age),
            };
            if old_enough && meta.is_file() && std::fs::remove_file(e.path()).is_ok() {
                report.removed += 1;
                report.bytes += meta.len();
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(b: &[u8]) -> String {
        hex::encode(Sha256::digest(b))
    }

    #[tokio::test]
    async fn fetch_verifies_and_reuses() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("Magisk-v27.0.apk");
        std::fs::write(&src, b"magisk bytes").unwrap();
        let url = format!("file://{}", src.display());
        let cache = DownloadCache::new(dir.path().join("cache"));
        let f = Fetcher::new().unwrap();
        let p = cache
            .fetch(&f, &url, &sha(b"magisk bytes"), &|_, _| {}, &Cancel::new())
            .await
            .unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"magisk bytes");
        assert_eq!(p.extension().unwrap(), "apk");
        // Second fetch is served from the cache even if the source is gone.
        std::fs::remove_file(&src).unwrap();
        let again = cache
            .fetch(&f, &url, &sha(b"magisk bytes"), &|_, _| {}, &Cancel::new())
            .await
            .unwrap();
        assert_eq!(again, p);
    }

    #[tokio::test]
    async fn wrong_hash_stores_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("x.apk");
        std::fs::write(&src, b"tampered").unwrap();
        let cache = DownloadCache::new(dir.path().join("cache"));
        let err = cache
            .fetch(
                &Fetcher::new().unwrap(),
                &format!("file://{}", src.display()),
                &sha(b"original"),
                &|_, _| {},
                &Cancel::new(),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("checksum mismatch"), "{err}");
        assert!(cache.list().is_empty());
        assert_eq!(cache.prune(None).removed, 0, "no partial file left");
    }

    #[test]
    fn import_get_list_prune() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("local.apk");
        std::fs::write(&src, b"offline").unwrap();
        let cache = DownloadCache::new(dir.path().join("cache"));
        assert_eq!(
            cache.import(&src, Some(&sha(b"other"))).unwrap_err().code,
            ErrorCode::Usage
        );
        let e = cache.import(&src, Some(&sha(b"offline"))).unwrap();
        assert_eq!(e.sha256, sha(b"offline"));
        assert_eq!(cache.import(&src, None).unwrap().path, e.path, "idempotent");
        assert_eq!(cache.get(&e.sha256).unwrap(), Some(e.path.clone()));
        assert_eq!(cache.list().len(), 1);

        // A corrupted entry is detected and dropped.
        std::fs::write(&e.path, b"bitrot").unwrap();
        assert_eq!(cache.get_verified(&e.sha256).unwrap(), None);
        assert!(cache.list().is_empty());

        cache.import(&src, None).unwrap();
        assert_eq!(
            cache.prune(Some(Duration::from_secs(3600))).removed,
            0,
            "too new"
        );
        let r = cache.prune(None);
        assert_eq!((r.removed, r.bytes), (1, 7));
        assert!(cache.get("nothex").is_err());
    }
}
