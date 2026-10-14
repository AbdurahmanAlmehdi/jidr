//! Parser for Google's SDK repository XML (`repository2-3.xml`,
//! `sys-img/<tag>/sys-img2-3.xml`): packages, archives and license texts.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use jidr_core::{ErrorCode, ImageRef, JidrError, JidrResult, ResultExt};
use serde::{Deserialize, Serialize};

/// Default repository root; override with `JIDR_SDK_REPO` (mirrors, tests).
pub const DEFAULT_REPO_BASE: &str = "https://dl.google.com/android/repository/";

/// Directory names of the system-image listings we read, under `sys-img/`.
pub const SYS_IMG_LISTS: &[&str] = &[
    "android",
    "google_apis",
    "google_apis_playstore",
    "aosp_atd",
];

pub fn repo_base() -> String {
    let base = std::env::var("JIDR_SDK_REPO").unwrap_or_else(|_| DEFAULT_REPO_BASE.to_string());
    if base.ends_with('/') {
        base
    } else {
        format!("{base}/")
    }
}

/// URLs of every listing Jidr reads, relative to `repo_base()`.
pub fn listing_urls(base: &str) -> Vec<String> {
    std::iter::once(format!("{base}repository2-3.xml"))
        .chain(
            SYS_IMG_LISTS
                .iter()
                .map(|t| format!("{base}sys-img/{t}/sys-img2-3.xml")),
        )
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision {
    pub major: u32,
    pub minor: u32,
    pub micro: u32,
    /// Preview number; `None` sorts after any preview of the same version.
    pub preview: Option<u32>,
}

impl Revision {
    pub fn parse(s: &str) -> Option<Self> {
        let (main, preview) = match s.split_once(" rc") {
            Some((m, p)) => (m, p.trim().parse().ok()),
            None => (s, None),
        };
        let mut parts = main.trim().split('.').map(|p| p.parse::<u32>());
        Some(Self {
            major: parts.next()?.ok()?,
            minor: parts.next().unwrap_or(Ok(0)).ok()?,
            micro: parts.next().unwrap_or(Ok(0)).ok()?,
            preview,
        })
    }
}

impl std::fmt::Display for Revision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.micro)?;
        if let Some(p) = self.preview {
            write!(f, " rc{p}")?;
        }
        Ok(())
    }
}

impl Ord for Revision {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.micro)
            .cmp(&(other.major, other.minor, other.micro))
            .then_with(|| match (self.preview, other.preview) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.cmp(&b),
            })
    }
}

impl PartialOrd for Revision {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Archive {
    /// Absolute URL.
    pub url: String,
    pub sha1: String,
    pub size: u64,
    pub host_os: Option<String>,
    pub host_arch: Option<String>,
}

impl Archive {
    fn matches(&self, os: &str, arch: &str) -> bool {
        self.host_os.as_deref().is_none_or(|o| o == os)
            && self.host_arch.as_deref().is_none_or(|a| a == arch)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemotePackage {
    /// SDK path, e.g. `emulator` or `system-images;android-35;google_apis;arm64-v8a`.
    pub path: String,
    pub display: String,
    pub revision: Revision,
    pub license: Option<String>,
    /// 0 = stable, 1 = beta, 2 = dev, 3 = canary.
    pub channel: u32,
    pub archives: Vec<Archive>,
    pub image: Option<ImageRef>,
}

impl RemotePackage {
    pub fn archive_for(&self, os: &str, arch: &str) -> Option<&Archive> {
        self.archives.iter().find(|a| a.matches(os, arch))
    }

    pub fn archive_for_host(&self) -> Option<&Archive> {
        self.archive_for(crate::host::host_os(), crate::host::host_arch())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct License {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct Repository {
    pub packages: Vec<RemotePackage>,
    pub licenses: BTreeMap<String, License>,
}

impl Repository {
    /// Parses one listing. Relative archive URLs resolve against `listing_url`.
    pub fn parse(xml: &str, listing_url: &str) -> JidrResult<Self> {
        let doc =
            roxmltree::Document::parse(xml).ctx(ErrorCode::Sdk, "parsing SDK repository XML")?;
        let base = listing_url
            .rfind('/')
            .map(|i| &listing_url[..=i])
            .unwrap_or_default();
        let root = doc.root_element();
        let mut repo = Self::default();
        for node in root.children().filter(|n| n.is_element()) {
            match node.tag_name().name() {
                "license" => {
                    if let Some(id) = node.attribute("id") {
                        let text = node.text().unwrap_or_default().to_string();
                        repo.licenses.insert(
                            id.to_string(),
                            License {
                                id: id.to_string(),
                                text,
                            },
                        );
                    }
                }
                "remotePackage" => {
                    if let Some(p) = parse_package(node, base) {
                        repo.packages.push(p);
                    }
                }
                _ => {}
            }
        }
        Ok(repo)
    }

    pub fn merge(&mut self, other: Repository) {
        self.packages.extend(other.packages);
        for (id, l) in other.licenses {
            self.licenses.entry(id).or_insert(l);
        }
    }

    /// The newest revision of `path` on a channel no less stable than `max_channel`.
    pub fn latest(&self, path: &str, max_channel: u32) -> Option<&RemotePackage> {
        self.packages
            .iter()
            .filter(|p| p.path == path && p.channel <= max_channel)
            .max_by(|a, b| a.revision.cmp(&b.revision))
    }

    pub fn license(&self, id: &str) -> JidrResult<&License> {
        self.licenses
            .get(id)
            .ok_or_else(|| JidrError::sdk(format!("license {id} is not in the SDK repository")))
    }
}

fn child<'a, 'i>(node: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
}

fn child_text(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    child(node, name)
        .and_then(|c| c.text())
        .map(|t| t.trim().to_string())
}

fn parse_package(node: roxmltree::Node<'_, '_>, base: &str) -> Option<RemotePackage> {
    let path = node.attribute("path")?.to_string();
    let rev = child(node, "revision")?;
    let num = |n: &str| child_text(rev, n).and_then(|t| t.parse().ok());
    let revision = Revision {
        major: num("major")?,
        minor: num("minor").unwrap_or(0),
        micro: num("micro").unwrap_or(0),
        preview: num("preview"),
    };
    let channel = child(node, "channelRef")
        .and_then(|c| c.attribute("ref"))
        .and_then(|r| r.strip_prefix("channel-"))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let license = child(node, "uses-license")
        .and_then(|l| l.attribute("ref"))
        .map(str::to_string);
    let archives = child(node, "archives")
        .map(|a| {
            a.children()
                .filter(|c| c.is_element() && c.tag_name().name() == "archive")
                .filter_map(|arc| {
                    let complete = child(arc, "complete")?;
                    let url = child_text(complete, "url")?;
                    Some(Archive {
                        url: if url.contains("://") {
                            url
                        } else {
                            format!("{base}{url}")
                        },
                        sha1: child_text(complete, "checksum")?.to_lowercase(),
                        size: child_text(complete, "size")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0),
                        host_os: child_text(arc, "host-os"),
                        host_arch: child_text(arc, "host-arch"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let image = ImageRef::parse_package(&path);
    Some(RemotePackage {
        display: child_text(node, "display-name").unwrap_or_else(|| path.clone()),
        path,
        revision,
        license,
        channel,
        archives,
        image,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A tiny synthetic listing in the published format (not Google's text).
    pub const FIXTURE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<sdk:sdk-repository xmlns:sdk="http://schemas.android.com/sdk/android/repo/repository2/03"
                    xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <license id="test-license" type="text">Line one.
Line two &amp; more.</license>
  <channel id="channel-0">stable</channel>
  <remotePackage path="emulator">
    <type-details xsi:type="generic:genericDetailsType"/>
    <revision><major>36</major><minor>1</minor><micro>2</micro></revision>
    <display-name>Emulator</display-name>
    <uses-license ref="test-license"/>
    <channelRef ref="channel-0"/>
    <archives>
      <archive>
        <complete><size>10</size><checksum type="sha1">ABCDEF</checksum><url>emu-mac-arm.zip</url></complete>
        <host-os>macosx</host-os><host-arch>aarch64</host-arch>
      </archive>
      <archive>
        <complete><size>11</size><checksum type="sha1">012345</checksum><url>https://mirror.example/emu-linux.zip</url></complete>
        <host-os>linux</host-os>
      </archive>
    </archives>
  </remotePackage>
  <remotePackage path="emulator">
    <revision><major>37</major><minor>0</minor><micro>0</micro></revision>
    <display-name>Emulator canary</display-name>
    <channelRef ref="channel-3"/>
    <archives/>
  </remotePackage>
  <remotePackage path="system-images;android-31;google_apis;arm64-v8a">
    <revision><major>11</major></revision>
    <display-name>Google APIs ARM 64 v8a System Image</display-name>
    <uses-license ref="test-license"/>
    <channelRef ref="channel-0"/>
    <archives>
      <archive><complete><size>99</size><checksum type="sha1">aa</checksum><url>arm64-v8a-31_r11.zip</url></complete></archive>
    </archives>
  </remotePackage>
</sdk:sdk-repository>"#;

    #[test]
    fn parses_packages_archives_and_licenses() {
        let repo = Repository::parse(FIXTURE, "https://dl.example/repo/repository2-3.xml").unwrap();
        assert_eq!(repo.packages.len(), 3);
        assert_eq!(
            repo.licenses["test-license"].text,
            "Line one.\nLine two & more."
        );
        let emu = repo.latest("emulator", 0).unwrap();
        assert_eq!(emu.revision.to_string(), "36.1.2");
        let arc = emu.archive_for("macosx", "aarch64").unwrap();
        assert_eq!(arc.url, "https://dl.example/repo/emu-mac-arm.zip");
        assert_eq!(arc.sha1, "abcdef");
        assert_eq!(
            emu.archive_for("linux", "x64").unwrap().url,
            "https://mirror.example/emu-linux.zip"
        );
        assert!(emu.archive_for("windows", "x64").is_none());
        assert_eq!(repo.latest("emulator", 3).unwrap().revision.major, 37);
        let img = repo
            .latest("system-images;android-31;google_apis;arm64-v8a", 0)
            .unwrap();
        assert_eq!(img.image.as_ref().unwrap().api, 31);
        assert!(
            img.archive_for("macosx", "aarch64").is_some(),
            "host-less archive matches any host"
        );
    }

    #[test]
    fn revision_ordering() {
        let r = |s: &str| Revision::parse(s).unwrap();
        assert!(r("36.1.2") > r("36.1.1"));
        assert!(r("36.1.2") > r("36.1.2 rc3"));
        assert!(r("36.1.2 rc4") > r("36.1.2 rc3"));
        assert_eq!(
            r("11"),
            Revision {
                major: 11,
                ..Default::default()
            }
        );
    }

    #[test]
    fn bad_xml_is_an_sdk_error() {
        assert_eq!(
            Repository::parse("<nope", "x").unwrap_err().code,
            ErrorCode::Sdk
        );
    }
}
