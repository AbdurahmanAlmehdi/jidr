//! Android SDK locations. Jidr installs into its private SDK; Android Studio's
//! SDK is only ever read (D3).

use std::path::{Path, PathBuf};

use jidr_core::{ImageRef, JidrError, JidrPaths, JidrResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SdkKind {
    /// `~/Library/Application Support/Jidr/sdk`, owned by Jidr.
    Private,
    /// Android Studio's SDK: read-only.
    StudioReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Sdk {
    pub root: PathBuf,
    pub kind: SdkKind,
}

impl Sdk {
    pub fn private(paths: &JidrPaths) -> Self {
        Self {
            root: paths.sdk(),
            kind: SdkKind::Private,
        }
    }

    /// Android Studio's SDK, if present: `ANDROID_HOME`, `ANDROID_SDK_ROOT`,
    /// then the platform default.
    pub fn studio() -> Option<Self> {
        let candidates = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
            .iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from)
            .chain(dirs::home_dir().map(|h| {
                if cfg!(target_os = "macos") {
                    h.join("Library/Android/sdk")
                } else {
                    h.join("Android/Sdk")
                }
            }));
        candidates
            .into_iter()
            .find(|p| p.is_dir())
            .map(Self::read_only)
    }

    /// Any SDK root, read-only.
    pub fn read_only(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            kind: SdkKind::StudioReadOnly,
        }
    }

    pub fn emulator(&self) -> PathBuf {
        self.root.join("emulator/emulator")
    }

    pub fn adb(&self) -> PathBuf {
        self.root.join("platform-tools/adb")
    }

    pub fn has_emulator(&self) -> bool {
        self.emulator().is_file()
    }

    pub fn has_adb(&self) -> bool {
        self.adb().is_file()
    }

    /// Emulator version from `emulator/source.properties`.
    pub fn emulator_version(&self) -> Option<String> {
        read_properties(&self.root.join("emulator/source.properties"))
            .into_iter()
            .find(|(k, _)| k == "Pkg.Revision")
            .map(|(_, v)| v)
    }

    pub fn image_dir(&self, image: &ImageRef) -> PathBuf {
        self.root.join(image.sysdir())
    }

    /// True if the image directory is complete (not an empty or partial install).
    pub fn has_image(&self, image: &ImageRef) -> bool {
        is_complete_image(&self.image_dir(image))
    }

    /// All complete system images in this SDK.
    pub fn installed_images(&self) -> Vec<ImageRef> {
        let mut out = Vec::new();
        let Ok(apis) = std::fs::read_dir(self.root.join("system-images")) else {
            return out;
        };
        for api_dir in apis.flatten() {
            let name = api_dir.file_name().to_string_lossy().into_owned();
            let Some(api) = name.strip_prefix("android-").and_then(|a| a.parse().ok()) else {
                continue;
            };
            for tag in read_dirs(&api_dir.path()) {
                for abi in read_dirs(&tag) {
                    if !is_complete_image(&abi) {
                        continue;
                    }
                    if let (Some(t), Some(a)) = (file_name(&tag), file_name(&abi)) {
                        out.push(ImageRef {
                            api,
                            tag: t,
                            abi: a,
                        });
                    }
                }
            }
        }
        out.sort();
        out
    }

    /// Refuses writes to an SDK Jidr doesn't own.
    pub fn ensure_writable(&self) -> JidrResult<()> {
        match self.kind {
            SdkKind::Private => Ok(()),
            SdkKind::StudioReadOnly => Err(JidrError::sdk(format!(
                "{} is Android Studio's SDK; Jidr never writes to it",
                self.root.display()
            ))),
        }
    }
}

/// The SDKs Jidr can use: its own, plus Studio's read-only.
#[derive(Debug, Clone)]
pub struct SdkSet {
    pub private: Sdk,
    pub studio: Option<Sdk>,
}

impl SdkSet {
    pub fn discover(paths: &JidrPaths) -> Self {
        Self {
            private: Sdk::private(paths),
            studio: Sdk::studio(),
        }
    }

    /// The SDK that provides `emulator` and `adb`: the private one if complete,
    /// else Studio's.
    pub fn tools(&self) -> JidrResult<&Sdk> {
        std::iter::once(&self.private)
            .chain(self.studio.as_ref())
            .find(|s| s.has_emulator() && s.has_adb())
            .ok_or_else(|| JidrError::sdk("no Android emulator installed; run `jidr sdk install`"))
    }

    /// The SDK holding `image`, preferring the private one.
    pub fn find_image(&self, image: &ImageRef) -> Option<&Sdk> {
        std::iter::once(&self.private)
            .chain(self.studio.as_ref())
            .find(|s| s.has_image(image))
    }
}

fn is_complete_image(dir: &Path) -> bool {
    ["source.properties", "system.img", "ramdisk.img"]
        .iter()
        .all(|f| dir.join(f).is_file())
}

fn read_dirs(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

fn file_name(p: &Path) -> Option<String> {
    p.file_name().map(|n| n.to_string_lossy().into_owned())
}

/// Reads a Java-style `key=value` properties file; missing file → empty.
pub fn read_properties(path: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(path)
        .map(|t| {
            t.lines()
                .filter(|l| !l.trim_start().starts_with('#'))
                .filter_map(|l| l.split_once('='))
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_image(root: &Path, image: &ImageRef, complete: bool) {
        let dir = root.join(image.sysdir());
        std::fs::create_dir_all(&dir).unwrap();
        if complete {
            for f in ["source.properties", "system.img", "ramdisk.img"] {
                std::fs::write(dir.join(f), b"x").unwrap();
            }
        }
    }

    fn img(api: u32, tag: &str) -> ImageRef {
        ImageRef {
            api,
            tag: tag.into(),
            abi: "arm64-v8a".into(),
        }
    }

    #[test]
    fn lists_only_complete_images() {
        let dir = tempfile::tempdir().unwrap();
        fake_image(dir.path(), &img(31, "google_apis"), true);
        fake_image(dir.path(), &img(36, "google_apis"), false);
        let sdk = Sdk::read_only(dir.path());
        assert_eq!(sdk.installed_images(), vec![img(31, "google_apis")]);
        assert!(sdk.has_image(&img(31, "google_apis")));
        assert!(!sdk.has_image(&img(36, "google_apis")));
    }

    #[test]
    fn studio_sdk_is_never_writable() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Sdk::read_only(dir.path()).ensure_writable().is_err());
        let paths = JidrPaths::under(dir.path());
        assert!(Sdk::private(&paths).ensure_writable().is_ok());
    }

    #[test]
    fn find_image_prefers_private() {
        let dir = tempfile::tempdir().unwrap();
        let paths = JidrPaths::under(dir.path().join("jidr"));
        let studio = dir.path().join("studio");
        fake_image(&studio, &img(31, "google_apis"), true);
        let set = SdkSet {
            private: Sdk::private(&paths),
            studio: Some(Sdk::read_only(&studio)),
        };
        assert_eq!(
            set.find_image(&img(31, "google_apis")).map(|s| s.kind),
            Some(SdkKind::StudioReadOnly)
        );
        fake_image(&paths.sdk(), &img(31, "google_apis"), true);
        assert_eq!(
            set.find_image(&img(31, "google_apis")).map(|s| s.kind),
            Some(SdkKind::Private)
        );
        assert!(set.tools().is_err());
    }
}
