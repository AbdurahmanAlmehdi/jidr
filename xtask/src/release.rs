//! `cargo xtask release`: the macOS release artifacts (Phases 12a and 12b).
//!
//! Builds `jidr` and `jidr-gui` in release mode, assembles `Jidr.app`, packs
//! `Jidr-<version>-arm64.dmg`, mounts it to prove it opens, packs the MCP
//! bundle `jidr-mcp-<version>-darwin-arm64.mcpb` (the signed `jidr` plus
//! `packaging/mcp/manifest.json`) and writes `SHA256SUMS.txt`. With `--scan` it fails if the bundle or the DMG contains
//! anything Jidr must never ship: Google SDK binaries, system images or
//! Magisk (NOTICE, clean-room rules).
//!
//! Only system tools are used (`codesign`, `hdiutil`, `plutil`, `shasum`,
//! `notarytool`), so xtask needs no new dependencies. With
//! `JIDR_SIGNING_IDENTITY` set, the app gets a hardened-runtime Developer ID
//! signature and the DMG is signed, notarized and stapled by
//! `packaging/macos/sign.sh` (which also needs an App Store Connect API key:
//! `NOTARY_KEY_PATH`, `NOTARY_KEY_ID`, `NOTARY_ISSUER_ID`). Without it, or with
//! `--dry-run`, the build is ad-hoc
//! signed and CI gets a warning (D32).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::{cargo, workspace_root};

const USAGE: &str = "\
usage: cargo xtask release [--dry-run] [--scan] [--out <dir>]

  --dry-run   build and package locally; never sign, even if secrets exist
  --scan      fail if the artifacts contain Google SDK binaries, system images or Magisk
  --out DIR   output directory (default: <target>/release-artifacts)";

/// Env var with the Developer ID identity, e.g.
/// `Developer ID Application: Name (TEAMID)` (Phase 12b).
const SIGNING_IDENTITY_ENV: &str = "JIDR_SIGNING_IDENTITY";

#[derive(Debug, Default, PartialEq)]
struct Options {
    dry_run: bool,
    scan: bool,
    out: Option<PathBuf>,
    help: bool,
}

fn parse_args(args: &[String]) -> Result<Options> {
    let mut opts = Options::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--dry-run" => opts.dry_run = true,
            "--scan" => opts.scan = true,
            "--out" => {
                let dir = it.next().context("--out needs a directory")?;
                opts.out = Some(PathBuf::from(dir));
            }
            "-h" | "--help" => opts.help = true,
            other => bail!("unknown argument `{other}`\n{USAGE}"),
        }
    }
    Ok(opts)
}

pub fn run(args: &[String]) -> Result<()> {
    let opts = parse_args(args)?;
    if opts.help {
        println!("{USAGE}");
        return Ok(());
    }
    if !cfg!(target_os = "macos") {
        bail!("release builds the macOS app and DMG; run it on macOS (D15)");
    }
    let root = workspace_root();
    let version = workspace_version()?;
    let target = target_dir(&root);
    let out = opts
        .out
        .clone()
        .map(|o| if o.is_absolute() { o } else { root.join(o) })
        .unwrap_or_else(|| target.join("release-artifacts"));
    println!("xtask: release {version} → {}", out.display());

    crate::run(
        &cargo(),
        &[
            "build",
            "--release",
            "--locked",
            "-p",
            "jidr",
            "-p",
            "jidr-gui",
        ],
    )?;
    let bin = target.join("release");

    if out.exists() {
        std::fs::remove_dir_all(&out).with_context(|| format!("clearing {}", out.display()))?;
    }
    let staging = out.join("staging");
    let app = staging.join("Jidr.app");
    assemble_app(&root, &bin, &app, &version)?;
    let identity = std::env::var(SIGNING_IDENTITY_ENV).ok();
    let signing = signing_mode(identity.as_deref(), opts.dry_run);
    let sign_script = root.join("packaging/macos/sign.sh");
    sign(&app, &signing, &sign_script)?;
    // The bundle reuses the signed CLI; the DMG's notarization covers it.
    let mcpb_name = format!("jidr-mcp-{version}-darwin-arm64.mcpb");
    let mcpb_dir = out.join("mcpb");
    pack_mcpb(
        &root,
        &app.join("Contents/Helpers/jidr"),
        &mcpb_dir,
        &out.join(&mcpb_name),
        &version,
    )?;
    std::os::unix::fs::symlink("/Applications", staging.join("Applications"))
        .context("linking /Applications into the DMG")?;

    let dmg_name = format!("Jidr-{version}-arm64.dmg");
    let dmg = out.join(&dmg_name);
    tool(
        "hdiutil",
        &[
            "create",
            "-volname",
            &format!("Jidr {version}"),
            "-srcfolder",
            &path_str(&staging)?,
            "-fs",
            "APFS",
            "-format",
            "UDZO",
            "-ov",
            &path_str(&dmg)?,
        ],
    )?;
    if signing == Signing::DeveloperId {
        // Sign, notarize and staple before the checksum is taken.
        tool("bash", &[&path_str(&sign_script)?, "dmg", &path_str(&dmg)?])?;
    }

    let mut findings = Vec::new();
    if opts.scan {
        findings.extend(scan_tree(&app)?);
        findings.extend(scan_tree(&mcpb_dir)?);
    }
    findings.extend(check_dmg(&dmg, &out.join("mnt"), opts.scan)?);
    std::fs::remove_dir_all(&staging).ok();
    std::fs::remove_dir_all(&mcpb_dir).ok();

    let sums = tool_output("shasum", &["-a", "256", &dmg_name, &mcpb_name], &out)?;
    std::fs::write(out.join("SHA256SUMS.txt"), &sums)?;
    std::fs::write(out.join("VERSION"), format!("{version}\n"))?;
    print!("{sums}");

    if !findings.is_empty() {
        for f in &findings {
            println!("xtask: scan: {}: {}", f.path.display(), f.reason);
        }
        bail!(
            "{} forbidden file(s) in the release artifacts (Google SDK, system images or Magisk)",
            findings.len()
        );
    }
    if opts.scan {
        println!("xtask: scan ok: no Google SDK binaries, system images or Magisk");
    }
    println!("xtask: release ok: {}", dmg.display());
    Ok(())
}

fn path_str(p: &Path) -> Result<String> {
    p.to_str()
        .map(str::to_owned)
        .with_context(|| format!("non-UTF-8 path {}", p.display()))
}

fn target_dir(root: &Path) -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from) {
        Some(dir) if dir.is_absolute() => dir,
        Some(dir) => root.join(dir),
        None => root.join("target"),
    }
}

/// The version of the `jidr` package (the workspace version).
fn workspace_version() -> Result<String> {
    let out = Command::new(cargo())
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(workspace_root())
        .output()
        .context("running cargo metadata")?;
    if !out.status.success() {
        bail!("cargo metadata failed");
    }
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    meta["packages"]
        .as_array()
        .and_then(|pkgs| pkgs.iter().find(|p| p["name"] == "jidr"))
        .and_then(|p| p["version"].as_str())
        .map(str::to_owned)
        .context("package `jidr` not found in cargo metadata")
}

fn tool(program: &str, args: &[&str]) -> Result<()> {
    crate::run(program, args)
}

fn tool_output(program: &str, args: &[&str], dir: &Path) -> Result<String> {
    let out = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .with_context(|| format!("running {program}"))?;
    if !out.status.success() {
        bail!(
            "`{program} {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::copy(from, to)
        .with_context(|| format!("copying {} to {}", from.display(), to.display()))?;
    Ok(())
}

/// `Jidr.app` layout. The CLI lives in `Helpers/`: `MacOS/jidr` would collide
/// with `MacOS/Jidr` on case-insensitive APFS.
fn assemble_app(root: &Path, bin: &Path, app: &Path, version: &str) -> Result<()> {
    let contents = app.join("Contents");
    copy(&bin.join("jidr-gui"), &contents.join("MacOS/Jidr"))?;
    copy(&bin.join("jidr"), &contents.join("Helpers/jidr"))?;
    copy(
        &root.join("apps/jidr-gui/assets/macos/Jidr.icns"),
        &contents.join("Resources/Jidr.icns"),
    )?;
    for (from, name) in [
        ("LICENSE-APACHE", "LICENSE-APACHE"),
        ("apps/jidr-gui/LICENSE-GPL", "LICENSE-GPL"),
        ("NOTICE", "NOTICE"),
    ] {
        copy(
            &root.join(from),
            &contents.join("Resources/licenses").join(name),
        )?;
    }
    let template = std::fs::read_to_string(root.join("packaging/macos/Info.plist.in"))
        .context("reading packaging/macos/Info.plist.in")?;
    let plist = contents.join("Info.plist");
    std::fs::write(&plist, render_version(&template, version, "Info.plist.in")?)?;
    std::fs::write(contents.join("PkgInfo"), "APPL????")?;
    tool("plutil", &["-lint", &path_str(&plist)?])
}

/// An MCPB bundle (a zip: `manifest.json`, `server/jidr`, icon, licences)
/// for Claude Desktop and the MCP registries.
fn pack_mcpb(root: &Path, jidr: &Path, dir: &Path, mcpb: &Path, version: &str) -> Result<()> {
    copy(jidr, &dir.join("server/jidr"))?;
    for (from, name) in [
        ("assets/brand/app-icon-512.png", "icon.png"),
        ("LICENSE-APACHE", "LICENSE-APACHE"),
        ("NOTICE", "NOTICE"),
    ] {
        copy(&root.join(from), &dir.join(name))?;
    }
    let template = std::fs::read_to_string(root.join("packaging/mcp/manifest.json"))
        .context("reading packaging/mcp/manifest.json")?;
    let manifest = render_version(&template, version, "manifest.json")?;
    serde_json::from_str::<serde_json::Value>(&manifest).context("manifest.json is not JSON")?;
    std::fs::write(dir.join("manifest.json"), manifest)?;
    tool_output("zip", &["-X", "-r", "-q", &path_str(mcpb)?, "."], dir)?;
    Ok(())
}

/// Fills `@VERSION@` in a packaging template.
fn render_version(template: &str, version: &str, file: &str) -> Result<String> {
    if !template.contains("@VERSION@") {
        bail!("{file} has no @VERSION@ placeholder");
    }
    if version.is_empty()
        || !version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        bail!("invalid version `{version}`");
    }
    Ok(template.replace("@VERSION@", version))
}

/// How the release is signed.
#[derive(Debug, PartialEq)]
enum Signing {
    /// Hardened runtime with the Developer ID identity, then notarization.
    DeveloperId,
    /// Ad-hoc only, with the reason shown as a warning.
    AdHoc(&'static str),
}

fn signing_mode(identity: Option<&str>, dry_run: bool) -> Signing {
    match (identity.filter(|s| !s.is_empty()), dry_run) {
        (_, true) => Signing::AdHoc("dry run: the app is ad-hoc signed only"),
        (Some(_), false) => Signing::DeveloperId,
        (None, false) => Signing::AdHoc(
            "Apple Developer ID secrets are missing; this release is unsigned (ad-hoc). Users open it from System Settings › Privacy & Security › Open Anyway (D32)",
        ),
    }
}

/// Signs the app inside-out: with the Developer ID through `sign.sh`, or
/// ad-hoc.
fn sign(app: &Path, signing: &Signing, sign_script: &Path) -> Result<()> {
    let message = match signing {
        Signing::DeveloperId => {
            return tool("bash", &[&path_str(sign_script)?, "app", &path_str(app)?]);
        }
        Signing::AdHoc(message) => message,
    };
    if std::env::var_os("GITHUB_ACTIONS").is_some() && !message.starts_with("dry run") {
        println!("::warning::{message}");
    } else {
        println!("xtask: warning: {message}");
    }
    for path in [app.join("Contents/Helpers/jidr"), app.to_path_buf()] {
        tool(
            "codesign",
            &[
                "--force",
                "--sign",
                "-",
                "--timestamp=none",
                &path_str(&path)?,
            ],
        )?;
    }
    tool("codesign", &["--verify", "--strict", &path_str(app)?])
}

/// Mounts the DMG read-only, checks the app is there, scans it if asked, and
/// always detaches.
fn check_dmg(dmg: &Path, mountpoint: &Path, scan: bool) -> Result<Vec<Finding>> {
    std::fs::create_dir_all(mountpoint)?;
    tool(
        "hdiutil",
        &[
            "attach",
            "-readonly",
            "-nobrowse",
            "-noautoopen",
            "-mountpoint",
            &path_str(mountpoint)?,
            &path_str(dmg)?,
        ],
    )?;
    let result = (|| {
        if !mountpoint.join("Jidr.app/Contents/MacOS/Jidr").is_file() {
            bail!("the mounted DMG has no Jidr.app");
        }
        if scan {
            scan_tree(mountpoint)
        } else {
            Ok(Vec::new())
        }
    })();
    let detached = tool("hdiutil", &["detach", &path_str(mountpoint)?]);
    let findings = result?;
    detached?;
    std::fs::remove_dir(mountpoint).ok();
    Ok(findings)
}

#[derive(Debug, PartialEq)]
struct Finding {
    path: PathBuf,
    reason: &'static str,
}

/// Walks `root` without following symlinks and classifies every file.
fn scan_tree(root: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in
            std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path)?;
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if meta.is_dir() {
                if let Some(reason) = classify_name(&name) {
                    findings.push(Finding {
                        path: path.clone(),
                        reason,
                    });
                }
                stack.push(path);
            } else if meta.is_file() {
                let reason = classify_name(&name).or_else(|| {
                    read_head(&path)
                        .ok()
                        .and_then(|head| classify_content(&head))
                });
                if let Some(reason) = reason {
                    findings.push(Finding { path, reason });
                }
            }
        }
    }
    findings.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(findings)
}

/// Enough of a file to see its magic numbers and an APK's first entries.
fn read_head(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut head = Vec::new();
    std::fs::File::open(path)?
        .take(64 * 1024)
        .read_to_end(&mut head)?;
    Ok(head)
}

/// Forbidden by file name (lowercased).
fn classify_name(name: &str) -> Option<&'static str> {
    const SDK_TOOLS: &[&str] = &[
        "emulator",
        "emulator-check",
        "adb",
        "fastboot",
        "sdkmanager",
        "avdmanager",
        "aapt",
        "aapt2",
        "mksdcard",
    ];
    const IMAGE_WORDS: &[&str] = &[
        "system",
        "vendor",
        "ramdisk",
        "userdata",
        "boot",
        "cache",
        "encryptionkey",
        "product",
    ];
    if name.contains("magisk") {
        return Some("Magisk (GPL-3.0, downloaded at runtime, never bundled)");
    }
    if SDK_TOOLS.contains(&name) || name.starts_with("qemu-system") || name.starts_with("libqemu") {
        return Some(
            "Android SDK / emulator binary (downloaded on the user's machine, never bundled)",
        );
    }
    if name.starts_with("kernel-ranchu") || name == "kernel-qemu" {
        return Some("emulator kernel (system image component)");
    }
    if name.ends_with(".img") && IMAGE_WORDS.iter().any(|w| name.contains(w)) {
        return Some("Android system image");
    }
    if name == "source.properties" || name == "package.xml" {
        return Some("Android SDK package metadata");
    }
    None
}

/// Forbidden by content: formats that only Android payloads use. Jidr's own
/// binaries are Mach-O, so any ELF in a macOS bundle is foreign.
fn classify_content(head: &[u8]) -> Option<&'static str> {
    let starts = |magic: &[u8]| head.starts_with(magic);
    if starts(b"\x7fELF") {
        return Some("ELF binary (Android/Linux code has no place in the macOS app)");
    }
    if starts(b"ANDROID!") || starts(b"VNDRBOOT") {
        return Some("Android boot image");
    }
    if starts(&[0x3a, 0xff, 0x26, 0xed]) {
        return Some("Android sparse image");
    }
    if head.get(1080..1082) == Some(&[0x53, 0xef]) {
        return Some("ext4 filesystem image");
    }
    if starts(&[0x02, 0x21, 0x4c, 0x18]) || starts(b"070701") || starts(b"070702") {
        return Some("ramdisk (lz4-legacy or cpio)");
    }
    if starts(b"dex\n") {
        return Some("Android dex bytecode");
    }
    if starts(b"PK\x03\x04") && contains(head, b"AndroidManifest.xml") {
        return Some("Android APK");
    }
    None
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    #[test]
    fn signing_follows_the_identity_and_dry_run() {
        use super::{Signing, signing_mode};
        assert_eq!(
            signing_mode(Some("Developer ID Application: A (T)"), false),
            Signing::DeveloperId
        );
        assert!(
            matches!(signing_mode(Some("Developer ID Application: A (T)"), true), Signing::AdHoc(m) if m.starts_with("dry run"))
        );
        assert!(matches!(signing_mode(None, false), Signing::AdHoc(m) if m.contains("missing")));
        assert!(matches!(signing_mode(Some(""), false), Signing::AdHoc(_)));
    }

    use super::*;

    #[test]
    fn parses_flags() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let o = parse_args(&args(&["--dry-run", "--scan", "--out", "dist"])).unwrap();
        assert_eq!(
            o,
            Options {
                dry_run: true,
                scan: true,
                out: Some(PathBuf::from("dist")),
                help: false
            }
        );
        assert!(parse_args(&args(&["--out"])).is_err());
        assert!(parse_args(&args(&["--sign"])).is_err());
        assert!(parse_args(&args(&["-h"])).unwrap().help);
    }

    #[test]
    fn forbidden_names() {
        for name in [
            "magisk.apk",
            "magiskinit",
            "libmagisk64.so",
            "emulator",
            "adb",
            "qemu-system-aarch64-headless",
            "kernel-ranchu",
            "system.img",
            "ramdisk.img",
            "userdata-qemu.img",
            "source.properties",
        ] {
            assert!(classify_name(name).is_some(), "{name}");
        }
        for name in [
            "jidr",
            "jidr.icns",
            "info.plist",
            "license-gpl",
            "notice",
            "disk.img",
        ] {
            assert_eq!(classify_name(name), None, "{name}");
        }
    }

    #[test]
    fn forbidden_content() {
        assert!(classify_content(b"\x7fELF\x02\x01\x01").is_some());
        assert!(classify_content(b"ANDROID!....").is_some());
        assert!(classify_content(&[0x3a, 0xff, 0x26, 0xed, 0, 0]).is_some());
        assert!(classify_content(&[0x02, 0x21, 0x4c, 0x18, 1, 2, 3, 4]).is_some());
        assert!(classify_content(b"070701000493e0").is_some());
        assert!(classify_content(b"dex\n035\0").is_some());
        let mut ext4 = vec![0u8; 2048];
        ext4[1080] = 0x53;
        ext4[1081] = 0xef;
        assert!(classify_content(&ext4).is_some());
        let mut apk = b"PK\x03\x04".to_vec();
        apk.extend_from_slice(b"....AndroidManifest.xml....");
        assert!(classify_content(&apk).is_some());
        // Ours: Mach-O (arm64), a plain zip, plists, text.
        assert_eq!(
            classify_content(&[0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1]),
            None
        );
        assert_eq!(classify_content(b"PK\x03\x04plain.txt"), None);
        assert_eq!(classify_content(b"<?xml version"), None);
        assert_eq!(classify_content(b""), None);
    }

    #[test]
    fn scan_finds_planted_payloads_and_ignores_symlinks() {
        let tmp = std::env::temp_dir().join(format!("jidr-scan-{}", std::process::id()));
        let app = tmp.join("Jidr.app/Contents");
        std::fs::create_dir_all(app.join("MacOS")).unwrap();
        std::fs::write(app.join("MacOS/Jidr"), [0xcf, 0xfa, 0xed, 0xfe]).unwrap();
        std::fs::write(app.join("Info.plist"), "<?xml").unwrap();
        assert!(scan_tree(&tmp).unwrap().is_empty());

        std::fs::create_dir_all(app.join("Resources/sdk/platform-tools")).unwrap();
        std::fs::write(app.join("Resources/sdk/platform-tools/adb"), b"x").unwrap();
        std::fs::write(app.join("Resources/payload.bin"), b"\x7fELF\x02").unwrap();
        std::os::unix::fs::symlink("/Applications", tmp.join("Applications")).unwrap();
        let found = scan_tree(&tmp).unwrap();
        std::fs::remove_dir_all(&tmp).unwrap();
        let names: Vec<_> = found
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        // Sorted by path; the /Applications symlink is not followed or flagged.
        assert_eq!(names, ["payload.bin", "adb"]);
    }

    #[test]
    fn templates_get_the_version() {
        let p = render_version("<string>@VERSION@</string>", "0.1.0", "t").unwrap();
        assert_eq!(p, "<string>0.1.0</string>");
        assert!(render_version("<string/>", "0.1.0", "t").is_err());
        assert!(render_version("@VERSION@", "0.1.0<x>", "t").is_err());
        let manifest = include_str!("../../packaging/mcp/manifest.json");
        assert!(render_version(manifest, "0.1.0", "manifest.json").is_ok());
    }
}
