//! The system-image catalog shown to users: remote listings merged with what
//! is installed, filtered to the host ABI and the supported API range (D23).

use std::collections::BTreeMap;

use jidr_core::{ImageRef, Support, SystemImage};

use crate::repo::Repository;
use crate::sdk::SdkSet;

#[derive(Debug, Clone)]
pub struct CatalogFilter {
    /// Only images with this ABI (the host ABI by default).
    pub abi: String,
    /// Include API 28 (best effort).
    pub include_best_effort: bool,
    /// Highest channel to include (0 = stable).
    pub max_channel: u32,
}

impl Default for CatalogFilter {
    fn default() -> Self {
        Self {
            abi: crate::host::host_abi().to_string(),
            include_best_effort: true,
            max_channel: 0,
        }
    }
}

impl CatalogFilter {
    fn accepts(&self, image: &ImageRef) -> bool {
        image.abi == self.abi
            && match Support::for_api(image.api) {
                Support::Supported => true,
                Support::BestEffort => self.include_best_effort,
                Support::Unsupported => false,
            }
    }
}

/// Builds the catalog. `repo` may be empty (offline): installed images still show.
pub fn image_catalog(repo: &Repository, sdks: &SdkSet, filter: &CatalogFilter) -> Vec<SystemImage> {
    let mut by_ref: BTreeMap<ImageRef, SystemImage> = BTreeMap::new();
    for pkg in repo
        .packages
        .iter()
        .filter(|p| p.channel <= filter.max_channel)
    {
        let Some(image) = &pkg.image else { continue };
        if !filter.accepts(image) {
            continue;
        }
        let newer = by_ref.get(image).is_none_or(|cur| {
            crate::repo::Revision::parse(&cur.revision).is_none_or(|r| pkg.revision > r)
        });
        if newer {
            by_ref.insert(
                image.clone(),
                SystemImage {
                    image: image.clone(),
                    display: pkg.display.clone(),
                    revision: pkg.revision.to_string(),
                    support: Support::for_api(image.api),
                    size: pkg.archive_for_host().map(|a| a.size).unwrap_or(0),
                    license: pkg.license.clone(),
                    installed_in: None,
                    pristine_ramdisk_sha256: None,
                },
            );
        }
    }
    // Installed images, including ones no longer listed remotely.
    for sdk in std::iter::once(&sdks.private).chain(sdks.studio.as_ref()) {
        for image in sdk
            .installed_images()
            .into_iter()
            .filter(|i| filter.accepts(i))
        {
            let entry = by_ref.entry(image.clone()).or_insert_with(|| SystemImage {
                display: format!("Android API {} ({})", image.api, image.tag),
                revision: String::new(),
                support: Support::for_api(image.api),
                size: 0,
                license: None,
                installed_in: None,
                pristine_ramdisk_sha256: None,
                image,
            });
            if entry.installed_in.is_none() {
                entry.installed_in = Some(sdk.root.clone());
            }
        }
    }
    by_ref.into_values().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdk::Sdk;
    use jidr_core::JidrPaths;

    const XML: &str = r#"<r>
      <remotePackage path="system-images;android-35;google_apis;arm64-v8a"><revision><major>3</major></revision><archives/></remotePackage>
      <remotePackage path="system-images;android-35;google_apis;arm64-v8a"><revision><major>5</major></revision><archives/></remotePackage>
      <remotePackage path="system-images;android-35;google_apis;x86_64"><revision><major>5</major></revision><archives/></remotePackage>
      <remotePackage path="system-images;android-27;google_apis;arm64-v8a"><revision><major>1</major></revision><archives/></remotePackage>
      <remotePackage path="system-images;android-28;default;arm64-v8a"><revision><major>1</major></revision><archives/></remotePackage>
      <remotePackage path="system-images;android-36;google_apis;arm64-v8a"><revision><major>1</major></revision><channelRef ref="channel-2"/><archives/></remotePackage>
    </r>"#;

    #[test]
    fn filters_abi_api_channel_and_keeps_latest() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::parse(XML, "file:///x.xml").unwrap();
        let sdks = SdkSet {
            private: Sdk::private(&JidrPaths::under(dir.path())),
            studio: None,
        };
        let filter = CatalogFilter {
            abi: "arm64-v8a".into(),
            include_best_effort: true,
            max_channel: 0,
        };
        let cat = image_catalog(&repo, &sdks, &filter);
        let ids: Vec<(u32, &str)> = cat
            .iter()
            .map(|i| (i.image.api, i.revision.as_str()))
            .collect();
        assert_eq!(ids, vec![(35, "5.0.0"), (28, "1.0.0")]);

        let strict = CatalogFilter {
            include_best_effort: false,
            ..filter
        };
        assert_eq!(image_catalog(&repo, &sdks, &strict).len(), 1);
    }

    #[test]
    fn installed_images_show_offline() {
        let dir = tempfile::tempdir().unwrap();
        let studio = dir.path().join("studio");
        let img = ImageRef {
            api: 31,
            tag: "google_apis".into(),
            abi: "arm64-v8a".into(),
        };
        let d = studio.join(img.sysdir());
        std::fs::create_dir_all(&d).unwrap();
        for f in ["source.properties", "system.img", "ramdisk.img"] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        let sdks = SdkSet {
            private: Sdk::private(&JidrPaths::under(dir.path().join("j"))),
            studio: Some(Sdk::read_only(&studio)),
        };
        let filter = CatalogFilter {
            abi: "arm64-v8a".into(),
            ..Default::default()
        };
        let cat = image_catalog(&Repository::default(), &sdks, &filter);
        assert_eq!(cat.len(), 1);
        assert_eq!(cat[0].installed_in.as_deref(), Some(studio.as_path()));
    }
}
