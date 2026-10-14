//! Built-in hardware profiles with generic names (D24).

use jidr_core::{HardwareProfile, JidrError, JidrResult};

const BUILTIN: &[&str] = &[
    include_str!("../../../assets/profiles/phone-6.1-fhd.toml"),
    include_str!("../../../assets/profiles/phone-6.7-qhd.toml"),
    include_str!("../../../assets/profiles/small-phone-5.toml"),
    include_str!("../../../assets/profiles/tablet-11.toml"),
];

/// The built-in profiles; the first one is the default.
pub fn builtin() -> Vec<HardwareProfile> {
    BUILTIN
        .iter()
        .filter_map(|t| match toml::from_str(t) {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::error!("bad built-in profile: {e}");
                None
            }
        })
        .collect()
}

pub fn find(id: &str) -> JidrResult<HardwareProfile> {
    builtin()
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| JidrError::not_found(format!("no hardware profile {id}")))
}

pub fn default_profile() -> JidrResult<HardwareProfile> {
    builtin()
        .into_iter()
        .next()
        .ok_or_else(|| JidrError::unexpected("no built-in profiles"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_builtin_profiles_parse() {
        let all = builtin();
        assert_eq!(all.len(), BUILTIN.len());
        let mut ids: Vec<&str> = all.iter().map(|p| p.id.as_str()).collect();
        ids.dedup();
        assert_eq!(ids.len(), all.len(), "ids are unique");
        assert_eq!(default_profile().unwrap().id, "phone-6.1-fhd");
        assert_eq!(
            find("tablet-11").unwrap().config["hw.initialOrientation"],
            "landscape"
        );
        assert!(find("nope").is_err());
    }

    #[test]
    fn names_are_generic() {
        for p in builtin() {
            for brand in ["Pixel", "Galaxy", "Nexus", "Samsung", "Google"] {
                assert!(
                    !p.name.contains(brand),
                    "{} uses a brand name (D24)",
                    p.name
                );
            }
        }
    }
}
