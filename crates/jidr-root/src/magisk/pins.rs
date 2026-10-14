//! The pinned Magisk releases the root engine may use (D30).
//!
//! The pin file is compiled into the binary, so `root.set magisk` works
//! offline once the APK is in the download cache. Magisk is GPL-3.0-only, so
//! Jidr never bundles it: the release is downloaded at runtime from the
//! official GitHub release and its SHA-256 is checked before use.

use jidr_core::{JidrError, JidrResult};
use serde::Deserialize;

const PINS: &str = include_str!("../../../../assets/magisk-pins.toml");

/// One entry of `assets/magisk-pins.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MagiskPin {
    /// Human-readable version, e.g. `30.7` (matches the release tag without `v`).
    pub version: String,
    /// The official GitHub release URL of the APK.
    pub url: String,
    /// Lowercase SHA-256 hex of the APK.
    pub sha256: String,
    /// The licence the release is under.
    pub license: String,
    /// Where the source lives.
    pub source: String,
}

#[derive(Debug, Deserialize)]
struct PinsFile {
    #[serde(default)]
    magisk: Vec<MagiskPin>,
}

/// All pinned releases, in preference order (first is the default).
pub fn pins() -> JidrResult<Vec<MagiskPin>> {
    let file: PinsFile = toml::from_str(PINS)
        .map_err(|e| JidrError::unexpected(format!("parsing magisk-pins.toml: {e}")))?;
    if file.magisk.is_empty() {
        return Err(JidrError::unexpected("magisk-pins.toml has no entries"));
    }
    Ok(file.magisk)
}

/// The release used when no `--magisk` override is supplied.
pub fn default_pin() -> JidrResult<MagiskPin> {
    pins()?
        .into_iter()
        .next()
        .ok_or_else(|| JidrError::unexpected("magisk-pins.toml has no entries"))
}

/// The licence + source notice shown once before the first download.
pub fn notice(pin: &MagiskPin) -> String {
    format!(
        "Magisk {} is {}; source: {}. Jidr downloads it at runtime and never bundles it.",
        pin.version, pin.license, pin.source
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_pin_is_a_valid_release() {
        let pin = default_pin().unwrap();
        assert!(!pin.version.is_empty());
        assert!(
            pin.url
                .starts_with("https://github.com/topjohnwu/Magisk/releases/download/")
        );
        assert_eq!(pin.sha256.len(), 64);
        assert!(pin.sha256.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!pin.license.is_empty());
        assert!(pin.source.contains("topjohnwu/Magisk"));
    }

    #[test]
    fn notice_names_the_licence_and_source() {
        let n = notice(&default_pin().unwrap());
        assert!(n.contains("GPL"));
        assert!(n.contains("github.com/topjohnwu/Magisk"));
    }
}
