//! License consent (D8): the user sees the actual license text and accepts
//! it; we record the id and the SHA-256 of that exact text. A changed text has
//! a new hash and needs consent again. Nothing is ever accepted implicitly.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use jidr_core::{ErrorCode, JidrError, JidrResult, ResultExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::repo::{License, RemotePackage, Repository};

pub fn license_sha256(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Consent {
    pub id: String,
    pub sha256: String,
    pub accepted_at: DateTime<Utc>,
}

/// A license the user still has to accept, with the text to show them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PendingLicense {
    pub id: String,
    pub sha256: String,
    pub text: String,
    /// Packages that need it.
    pub packages: Vec<String>,
}

#[derive(Debug)]
pub struct ConsentStore {
    path: PathBuf,
    records: Vec<Consent>,
}

impl ConsentStore {
    /// Loads the store; a missing file is an empty store.
    pub fn load(path: impl Into<PathBuf>) -> JidrResult<Self> {
        let path = path.into();
        let records = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .ctx(ErrorCode::Sdk, format!("reading {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(JidrError::sdk(format!("reading {}: {e}", path.display()))),
        };
        Ok(Self { path, records })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn records(&self) -> &[Consent] {
        &self.records
    }

    pub fn is_accepted(&self, license: &License) -> bool {
        let hash = license_sha256(&license.text);
        self.records
            .iter()
            .any(|r| r.id == license.id && r.sha256 == hash)
    }

    /// Records consent for this exact text and saves the store.
    pub fn accept(&mut self, license: &License) -> JidrResult<Consent> {
        let consent = Consent {
            id: license.id.clone(),
            sha256: license_sha256(&license.text),
            accepted_at: Utc::now(),
        };
        if !self.is_accepted(license) {
            self.records.push(consent.clone());
            self.save()?;
        }
        Ok(consent)
    }

    /// Accepts a license by id + hash (the headless `--accept-licenses` path
    /// prints each hash first; the hash must match the current text).
    pub fn accept_hash(
        &mut self,
        repo: &Repository,
        id: &str,
        sha256: &str,
    ) -> JidrResult<Consent> {
        let license = repo.license(id)?;
        let current = license_sha256(&license.text);
        if current != sha256 {
            return Err(JidrError::usage(format!(
                "license {id} text changed: current hash is {current}, you accepted {sha256}"
            )));
        }
        self.accept(license)
    }

    fn save(&self) -> JidrResult<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)
                .ctx(ErrorCode::Sdk, format!("creating {}", dir.display()))?;
        }
        let json = serde_json::to_vec_pretty(&self.records)
            .ctx(ErrorCode::Unexpected, "encoding consent")?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json).ctx(ErrorCode::Sdk, format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .ctx(ErrorCode::Sdk, format!("writing {}", self.path.display()))
    }

    /// Licenses still needed before `packages` can be installed.
    pub fn pending(
        &self,
        repo: &Repository,
        packages: &[&RemotePackage],
    ) -> JidrResult<Vec<PendingLicense>> {
        let mut out: Vec<PendingLicense> = Vec::new();
        for pkg in packages {
            let Some(id) = &pkg.license else { continue };
            let license = repo.license(id)?;
            if self.is_accepted(license) {
                continue;
            }
            match out.iter_mut().find(|p| &p.id == id) {
                Some(p) => p.packages.push(pkg.path.clone()),
                None => out.push(PendingLicense {
                    id: id.clone(),
                    sha256: license_sha256(&license.text),
                    text: license.text.clone(),
                    packages: vec![pkg.path.clone()],
                }),
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::tests::FIXTURE;

    fn repo() -> Repository {
        Repository::parse(FIXTURE, "https://x/repo.xml").unwrap()
    }

    #[test]
    fn consent_round_trip_and_rehash_on_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("consent.json");
        let repo = repo();
        let lic = repo.license("test-license").unwrap().clone();

        let mut store = ConsentStore::load(&path).unwrap();
        assert!(!store.is_accepted(&lic));
        store.accept(&lic).unwrap();
        store.accept(&lic).unwrap();
        assert_eq!(store.records().len(), 1, "accepting twice records once");

        let reloaded = ConsentStore::load(&path).unwrap();
        assert!(reloaded.is_accepted(&lic));

        let changed = License {
            id: lic.id.clone(),
            text: format!("{} (amended)", lic.text),
        };
        assert!(
            !reloaded.is_accepted(&changed),
            "a new text needs new consent"
        );
    }

    #[test]
    fn pending_groups_packages_by_license() {
        let dir = tempfile::tempdir().unwrap();
        let repo = repo();
        let pkgs: Vec<&RemotePackage> = repo
            .packages
            .iter()
            .filter(|p| p.license.is_some())
            .collect();
        let mut store = ConsentStore::load(dir.path().join("c.json")).unwrap();
        let pending = store.pending(&repo, &pkgs).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].packages.len(), 2);

        let hash = pending[0].sha256.clone();
        assert_eq!(
            store
                .accept_hash(&repo, "test-license", "bad")
                .unwrap_err()
                .code,
            ErrorCode::Usage
        );
        store.accept_hash(&repo, "test-license", &hash).unwrap();
        assert!(store.pending(&repo, &pkgs).unwrap().is_empty());
    }

    #[test]
    fn corrupt_store_is_an_error_not_a_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        std::fs::write(&path, b"{not json").unwrap();
        assert_eq!(ConsentStore::load(&path).unwrap_err().code, ErrorCode::Sdk);
    }
}
