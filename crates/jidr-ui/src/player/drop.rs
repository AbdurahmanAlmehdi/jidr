//! What a file dropped on the device does (plan 7c, D35, ADR-0004).

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropAction {
    /// `file.install` (replace, keep data), then launch it.
    InstallApk(PathBuf),
    /// Any zip: `file.flash`, which looks inside it (ADR-0004). A module
    /// installs on a Full-root device and is copied to Download otherwise;
    /// recovery and image zips are refused; other zips are copied.
    FlashZip { path: PathBuf, full_root: bool },
    /// Anything else: `file.push` to Download.
    Push {
        path: PathBuf,
        note: Option<&'static str>,
    },
}

pub fn classify(path: &Path, full_root: bool) -> DropAction {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "apk" => DropAction::InstallApk(path.to_path_buf()),
        "zip" => DropAction::FlashZip {
            path: path.to_path_buf(),
            full_root,
        },
        _ => DropAction::Push {
            path: path.to_path_buf(),
            note: None,
        },
    }
}

/// Title and body for the drop label while a file hovers (the device design).
pub fn hover_text(action: &DropAction, device: &str) -> (String, String) {
    let name = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    match action {
        DropAction::InstallApk(p) => (
            format!("Release to install {}", name(p)),
            format!(
                "adb install -r on {device}. Replaces the installed version and keeps its data."
            ),
        ),
        DropAction::FlashZip { path, full_root } => (
            format!("Release to flash {}", name(path)),
            if *full_root {
                "A Magisk module installs and reboots the device (about 40 s). Recovery and image zips can't be flashed; other zips go to Download.".into()
            } else {
                "Magisk modules need Full root, so a module is copied to Download. Recovery and image zips can't be flashed.".into()
            },
        ),
        DropAction::Push { path, note } => (
            format!("Release to copy {}", name(path)),
            match note {
                Some(n) => (*n).to_string(),
                None => format!("Goes to /sdcard/Download on {device}."),
            },
        ),
    }
}

/// Signature mismatch from `adb install -r`: the user may reinstall
/// (which deletes the app's data).
pub fn is_signature_mismatch(message: &str) -> bool {
    message.contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE")
        || message.contains("signatures do not match")
        || message.contains("INSTALL_FAILED_SHARED_USER_INCOMPATIBLE")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_route_by_type_and_root() {
        assert_eq!(
            classify(Path::new("/x/shop.APK"), false),
            DropAction::InstallApk("/x/shop.APK".into())
        );
        // Every zip goes to `file.flash`, which decides from its contents.
        assert_eq!(
            classify(Path::new("/x/m.ZIP"), true),
            DropAction::FlashZip {
                path: "/x/m.ZIP".into(),
                full_root: true
            }
        );
        assert_eq!(
            classify(Path::new("/x/m.zip"), false),
            DropAction::FlashZip {
                path: "/x/m.zip".into(),
                full_root: false
            }
        );
        assert!(matches!(
            classify(Path::new("/x/a.png"), true),
            DropAction::Push { note: None, .. }
        ));
    }

    #[test]
    fn hover_copy_matches_design() {
        let (t, b) = hover_text(
            &DropAction::InstallApk("/x/shop-debug.apk".into()),
            "Pixel-ish 6.1",
        );
        assert_eq!(t, "Release to install shop-debug.apk");
        assert_eq!(
            b,
            "adb install -r on Pixel-ish 6.1. Replaces the installed version and keeps its data."
        );
    }

    #[test]
    fn zip_hover_says_what_flashing_does() {
        let zip = |full_root| DropAction::FlashZip {
            path: "/x/lsposed.zip".into(),
            full_root,
        };
        let (t, b) = hover_text(&zip(true), "Phone");
        assert_eq!(t, "Release to flash lsposed.zip");
        assert!(b.contains("reboots"), "{b}");
        let (_, b) = hover_text(&zip(false), "Phone");
        assert!(b.contains("need Full root"), "{b}");
    }

    #[test]
    fn signature_mismatch_is_detected() {
        assert!(is_signature_mismatch(
            "adb: failed to install x.apk: Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: Package com.x signatures do not match previously installed version; ignoring!]"
        ));
        assert!(!is_signature_mismatch(
            "Failure [INSTALL_FAILED_INSUFFICIENT_STORAGE]"
        ));
    }
}
