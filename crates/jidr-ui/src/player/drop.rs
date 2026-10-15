//! What a file dropped on the device does (plan 7c, D35).

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropAction {
    /// `file.install` (replace, keep data), then launch it.
    InstallApk(PathBuf),
    /// Full-root device: `root.module.install` + reboot (D35).
    MagiskModule(PathBuf),
    /// Anything else, or a zip without Full root: `file.push` to Download.
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
        "zip" if full_root => DropAction::MagiskModule(path.to_path_buf()),
        "zip" => DropAction::Push {
            path: path.to_path_buf(),
            note: Some("Magisk modules need Full root, so it was copied to Download."),
        },
        _ => DropAction::Push {
            path: path.to_path_buf(),
            note: None,
        },
    }
}

/// Title and body for the drop label while a file hovers (Device.dc.html).
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
        DropAction::MagiskModule(p) => (
            format!("Release to install {} as a Magisk module", name(p)),
            "Installs the module and reboots the device (about 40 s).".into(),
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
        assert_eq!(
            classify(Path::new("/x/m.zip"), true),
            DropAction::MagiskModule("/x/m.zip".into())
        );
        assert!(matches!(
            classify(Path::new("/x/m.zip"), false),
            DropAction::Push { note: Some(_), .. }
        ));
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
    fn signature_mismatch_is_detected() {
        assert!(is_signature_mismatch(
            "adb: failed to install x.apk: Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: Package com.x signatures do not match previously installed version; ignoring!]"
        ));
        assert!(!is_signature_mismatch(
            "Failure [INSTALL_FAILED_INSUFFICIENT_STORAGE]"
        ));
    }
}
