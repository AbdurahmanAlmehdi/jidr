//! Create / edit device form: options from `profile.list` and `image.list`,
//! defaults (D7), the lean preset and auto-hibernate choices (D42),
//! validation, and the `device.create` / `device.edit` params.

use jidr_core::{HardwareProfile, Job, Support, SystemImage};
use serde_json::{Map, Value, json};

use crate::i18n::tr;
use crate::model::{self, DeviceInfo};
use crate::{DeviceForm, ImageRowVm, OptionVm, RootChoice};

/// Images users can pick: supported or best-effort API levels (D23),
/// newest first, Google APIs before Play Store before AOSP.
pub fn sorted_images(images: &[SystemImage]) -> Vec<SystemImage> {
    let mut v: Vec<SystemImage> = images
        .iter()
        .filter(|i| i.support != Support::Unsupported)
        .cloned()
        .collect();
    let tag_rank = |t: &str| match t {
        "google_apis" => 0,
        "google_apis_playstore" => 1,
        "default" => 2,
        _ => 3,
    };
    v.sort_by(|a, b| {
        b.image
            .api
            .cmp(&a.image.api)
            .then(tag_rank(&a.image.tag).cmp(&tag_rank(&b.image.tag)))
    });
    v
}

/// A running `image.install` job for this package, if any.
fn install_job<'a>(img: &SystemImage, jobs: &'a [Job]) -> Option<&'a Job> {
    jobs.iter().find(|j| {
        !j.state.is_terminal()
            && (j.kind == "image.install" || j.kind == "sdk.install")
            && j.message.contains(&img.image.package())
    })
}

pub fn image_label(img: &SystemImage) -> String {
    format!(
        "{} · {}",
        model::android_label(img.image.api),
        model::image_tag(&img.image)
    )
}

pub fn image_rows(images: &[SystemImage], jobs: &[Job]) -> Vec<ImageRowVm> {
    images
        .iter()
        .map(|img| {
            let installed = img.installed_in.is_some();
            let job = install_job(img, jobs);
            let status = match (installed, job) {
                (true, _) => tr!("Installed · {}", size_or_dash(img.size)),
                (false, Some(j)) => match j.progress {
                    Some(p) if img.size > 0 => tr!(
                        "Downloading · {} / {}",
                        format!("{:.1}", f64::from(p) * img.size as f64 / 1e9),
                        model::gb(img.size)
                    ),
                    _ => tr!("Downloading…"),
                },
                (false, None) => tr!("Download · {}", size_or_dash(img.size)),
            };
            ImageRowVm {
                package: img.image.package().into(),
                label: image_label(img).into(),
                status: status.into(),
                installed,
                enabled: job.is_none(),
                play_store: !img.image.allows_adb_root(),
            }
        })
        .collect()
}

fn size_or_dash(bytes: u64) -> String {
    if bytes > 0 {
        model::gb(bytes)
    } else {
        tr!("size unknown")
    }
}

pub fn profile_options(profiles: &[HardwareProfile]) -> Vec<OptionVm> {
    profiles
        .iter()
        .map(|p| OptionVm {
            id: p.id.clone().into(),
            label: format!("{} · {}×{}", p.name, p.width, p.height).into(),
        })
        .collect()
}

/// A free "Pixel-ish 6.1"-style name: the profile name, numbered if taken.
pub fn free_name(base: &str, taken: &[String]) -> String {
    let base = base.replace('″', "");
    if !taken.iter().any(|t| t.eq_ignore_ascii_case(&base)) {
        return base;
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|n| !taken.iter().any(|t| t.eq_ignore_ascii_case(n)))
        .unwrap_or(base)
}

/// Root choice for an image: Shell root where adb root works, else None (D7).
pub fn default_root(img: Option<&SystemImage>, preferred: RootChoice) -> RootChoice {
    let shell_ok = img.is_none_or(|i| i.image.allows_adb_root());
    match preferred {
        RootChoice::Shell if !shell_ok => RootChoice::None,
        other => other,
    }
}

/// Defaults for a new device: first profile (`phone-6.1-fhd` if present),
/// newest installed Google APIs image, Shell root where allowed.
pub fn new_form(
    profiles: &[HardwareProfile],
    images: &[SystemImage],
    taken: &[String],
    preferred_root: RootChoice,
) -> DeviceForm {
    let profile = profiles
        .iter()
        .position(|p| p.id == "phone-6.1-fhd")
        .unwrap_or(0);
    let image = images
        .iter()
        .position(|i| i.installed_in.is_some() && i.image.allows_adb_root())
        .or_else(|| images.iter().position(|i| i.installed_in.is_some()))
        .unwrap_or(0);
    let mut form = DeviceForm {
        start_after: true,
        image: image as i32,
        root: default_root(images.get(image), preferred_root),
        ..Default::default()
    };
    apply_profile(&mut form, profiles, profile);
    form.name = free_name(
        profiles
            .get(profile)
            .map(|p| p.name.as_str())
            .unwrap_or("Phone"),
        taken,
    )
    .into();
    form
}

/// Fills RAM, CPU and screen from a profile.
pub fn apply_profile(form: &mut DeviceForm, profiles: &[HardwareProfile], index: usize) {
    form.profile = index as i32;
    if let Some(p) = profiles.get(index) {
        fill_hardware(form, p);
        form.lean = p.id == LEAN_PROFILE;
    }
}

fn fill_hardware(form: &mut DeviceForm, p: &HardwareProfile) {
    form.ram_gb = format_gb(p.ram_mb).into();
    form.cpus = p.cpus.to_string().into();
    form.screen = format!("{}×{}", p.width, p.height).into();
    form.density = p.density.to_string().into();
}

/// The built-in lean profile (D42): 720×1280, 2 GB, 2 cores, no audio or
/// cameras; about 4 GB of host RAM instead of about 6.
pub const LEAN_PROFILE: &str = "agent-lean";

/// The Lean preset. A new device takes the lean profile; an existing one
/// can't change profile (`device.edit`), so it takes the lean profile's
/// RAM, cores and screen. `Err` when the daemon has no lean profile.
pub fn apply_lean(form: &mut DeviceForm, profiles: &[HardwareProfile]) -> Result<(), String> {
    let i = profiles
        .iter()
        .position(|p| p.id == LEAN_PROFILE)
        .ok_or_else(|| tr!("This daemon has no lean profile; update Jidr."))?;
    if form.edit {
        fill_hardware(form, &profiles[i]);
        form.lean = true;
    } else {
        apply_profile(form, profiles, i);
    }
    Ok(())
}

/// Auto-hibernate choices (D42) with the value each one stands for: in the
/// edit sheet `None` follows the global setting, `Some(0)` is never.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HibernateChoices {
    pub options: Vec<OptionVm>,
    pub values: Vec<Option<u32>>,
}

const HIBERNATE_MINUTES: [u32; 4] = [15, 30, 60, 120];

/// "After 30 min idle", "After 2 h idle".
pub fn idle_after(minutes: u32) -> String {
    if minutes >= 60 && minutes.is_multiple_of(60) {
        tr!("After {}\u{a0}h idle", minutes / 60)
    } else {
        tr!("After {}\u{a0}min idle", minutes)
    }
}

impl HibernateChoices {
    fn push(&mut self, label: String, value: Option<u32>) {
        self.options.push(OptionVm {
            id: value.map(|m| m.to_string()).unwrap_or_default().into(),
            label: label.into(),
        });
        self.values.push(value);
    }

    /// The per-device choices, given the global setting and the device's
    /// current value (kept as a choice when it isn't a preset).
    pub fn device(global: u32, current: Option<u32>) -> Self {
        let mut c = Self::default();
        let setting = if global == 0 {
            tr!("Off")
        } else {
            idle_after(global).to_lowercase()
        };
        c.push(tr!("Same as Settings ({})", setting), None);
        c.push(tr!("Never"), Some(0));
        for m in HIBERNATE_MINUTES {
            c.push(idle_after(m), Some(m));
        }
        if let Some(m) = current.filter(|m| *m > 0 && !HIBERNATE_MINUTES.contains(m)) {
            c.push(idle_after(m), Some(m));
        }
        c
    }

    /// The global choices (Settings › General); `0` is off.
    pub fn global(current: u32) -> Self {
        let mut c = Self::default();
        c.push(tr!("Off"), Some(0));
        for m in HIBERNATE_MINUTES {
            c.push(idle_after(m), Some(m));
        }
        if current > 0 && !HIBERNATE_MINUTES.contains(&current) {
            c.push(idle_after(current), Some(current));
        }
        c
    }

    pub fn index_of(&self, value: Option<u32>) -> i32 {
        self.values
            .iter()
            .position(|v| *v == value)
            .map(|i| i as i32)
            .unwrap_or(0)
    }

    pub fn value(&self, index: i32) -> Option<Option<u32>> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.values.get(i))
            .copied()
    }
}

fn format_gb(mb: u32) -> String {
    if mb.is_multiple_of(1024) {
        (mb / 1024).to_string()
    } else {
        format!("{:.1}", f64::from(mb) / 1024.0)
    }
}

/// The edit form for an existing device.
pub fn edit_form(
    info: &DeviceInfo,
    profiles: &[HardwareProfile],
    hibernate: &HibernateChoices,
) -> DeviceForm {
    DeviceForm {
        edit: true,
        name: info.device.name.clone().into(),
        profile: profiles
            .iter()
            .position(|p| p.id == info.device.profile)
            .unwrap_or(0) as i32,
        image: -1,
        root: match info.device.root.mode {
            jidr_core::RootMode::None => RootChoice::None,
            jidr_core::RootMode::Adb => RootChoice::Shell,
            jidr_core::RootMode::Magisk => RootChoice::Full,
        },
        ram_gb: format_gb(info.ram_mb).into(),
        cpus: info.cpus.to_string().into(),
        screen: format!("{}×{}", info.width, info.height).into(),
        density: info.density.to_string().into(),
        start_after: false,
        hibernate: hibernate.index_of(info.device.auto_hibernate_minutes),
        lean: info.device.profile == LEAN_PROFILE,
        error: Default::default(),
        play_store: !info.device.image.allows_adb_root(),
    }
}

struct Hardware {
    ram_mb: u32,
    cpus: u32,
    width: u32,
    height: u32,
    density: Option<u32>,
}

fn parse_hardware(form: &DeviceForm) -> Result<Hardware, String> {
    let ram: f64 = form
        .ram_gb
        .trim()
        .parse()
        .map_err(|_| tr!("RAM must be a number of GB, like 4."))?;
    if !(0.5..=64.0).contains(&ram) {
        return Err(tr!("RAM must be between 0.5 and 64 GB."));
    }
    let cpus: u32 = form
        .cpus
        .trim()
        .parse()
        .map_err(|_| tr!("CPU cores must be a whole number."))?;
    if !(1..=16).contains(&cpus) {
        return Err(tr!("CPU cores must be between 1 and 16."));
    }
    let (w, h) = form
        .screen
        .split(['×', 'x', 'X'])
        .map(|s| s.trim().parse::<u32>())
        .collect::<Result<Vec<_>, _>>()
        .ok()
        .and_then(|v| match v.as_slice() {
            [w, h] => Some((*w, *h)),
            _ => None,
        })
        .ok_or_else(|| tr!("Screen must look like 1080×2400."))?;
    if !(240..=4096).contains(&w) || !(240..=4096).contains(&h) {
        return Err(tr!("Screen sides must be between 240 and 4096 pixels."));
    }
    Ok(Hardware {
        ram_mb: (ram * 1024.0).round() as u32,
        cpus,
        width: w,
        height: h,
        density: form.density.trim().parse().ok(),
    })
}

fn check_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(tr!("Give the device a name."));
    }
    Ok(name.to_string())
}

pub fn root_mode(c: RootChoice) -> &'static str {
    match c {
        RootChoice::None => "none",
        RootChoice::Shell => "adb",
        RootChoice::Full => "magisk",
    }
}

/// `device.create` params; only values that differ from the profile are sent.
pub fn create_params(
    form: &DeviceForm,
    profiles: &[HardwareProfile],
    images: &[SystemImage],
) -> Result<Value, String> {
    let name = check_name(&form.name)?;
    let img = usize::try_from(form.image)
        .ok()
        .and_then(|i| images.get(i))
        .ok_or_else(|| tr!("Pick an Android version."))?;
    if form.root == RootChoice::Shell && !img.image.allows_adb_root() {
        return Err(tr!("Play Store images don't allow adb root."));
    }
    let hw = parse_hardware(form)?;
    let profile = usize::try_from(form.profile)
        .ok()
        .and_then(|i| profiles.get(i));
    let mut p = Map::new();
    p.insert("name".into(), json!(name));
    p.insert("image".into(), json!(img.image.package()));
    p.insert("root_mode".into(), json!(root_mode(form.root)));
    if let Some(prof) = profile {
        p.insert("profile".into(), json!(prof.id));
        if prof.ram_mb != hw.ram_mb {
            p.insert("ram_mb".into(), json!(hw.ram_mb));
        }
        if prof.cpus != hw.cpus {
            p.insert("cpus".into(), json!(hw.cpus));
        }
        if (prof.width, prof.height) != (hw.width, hw.height) {
            p.insert("width".into(), json!(hw.width));
            p.insert("height".into(), json!(hw.height));
        }
    }
    Ok(Value::Object(p))
}

/// `device.edit` params for the fields that changed.
pub fn edit_params(
    form: &DeviceForm,
    info: &DeviceInfo,
    hibernate: &HibernateChoices,
) -> Result<Value, String> {
    let name = check_name(&form.name)?;
    let hw = parse_hardware(form)?;
    let mut p = Map::new();
    p.insert("device".into(), json!(info.device.id));
    if name != info.device.name {
        p.insert("name".into(), json!(name));
    }
    if hw.ram_mb != info.ram_mb {
        p.insert("ram_mb".into(), json!(hw.ram_mb));
    }
    if hw.cpus != info.cpus {
        p.insert("cpus".into(), json!(hw.cpus));
    }
    if (hw.width, hw.height) != (info.width, info.height) {
        p.insert("width".into(), json!(hw.width));
        p.insert("height".into(), json!(hw.height));
    }
    if let Some(d) = hw.density.filter(|d| *d != info.density && *d > 0) {
        p.insert("density".into(), json!(d));
    }
    // `null` goes back to following the global setting.
    if let Some(v) = hibernate.value(form.hibernate)
        && v != info.device.auto_hibernate_minutes
    {
        p.insert("auto_hibernate_minutes".into(), json!(v));
    }
    Ok(Value::Object(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jidr_core::ImageRef;

    fn img(api: u32, tag: &str, installed: bool) -> SystemImage {
        SystemImage {
            image: ImageRef {
                api,
                tag: tag.into(),
                abi: "arm64-v8a".into(),
            },
            display: String::new(),
            revision: "1".into(),
            support: Support::for_api(api),
            size: 2_000_000_000,
            license: None,
            installed_in: installed.then(|| "/sdk".into()),
            pristine_ramdisk_sha256: None,
        }
    }

    fn profiles() -> Vec<HardwareProfile> {
        jidr_core::provider::parse_params(serde_json::json!([
            {"id": "small-phone-5", "name": "Small phone 5″", "width": 720, "height": 1280, "density": 320, "ram_mb": 2048, "cpus": 2},
            {"id": "phone-6.1-fhd", "name": "Phone 6.1″ FHD+", "width": 1080, "height": 2400, "density": 420, "ram_mb": 4096, "cpus": 4}
        ]))
        .unwrap()
    }

    #[test]
    fn images_sort_newest_first_and_drop_unsupported() {
        let v = sorted_images(&[
            img(33, "google_apis_playstore", true),
            img(35, "google_apis", true),
            img(26, "google_apis", true),
            img(35, "google_apis_playstore", false),
        ]);
        let labels: Vec<_> = v.iter().map(image_label).collect();
        assert_eq!(
            labels,
            [
                "Android 15 · API 35 · Google APIs",
                "Android 15 · API 35 · Play Store",
                "Android 13 · API 33 · Play Store"
            ]
        );
    }

    #[test]
    fn defaults_follow_d7() {
        let images = sorted_images(&[
            img(35, "google_apis", true),
            img(33, "google_apis_playstore", true),
        ]);
        let f = new_form(
            &profiles(),
            &images,
            &["Phone 6.1 FHD+".into()],
            RootChoice::Shell,
        );
        assert_eq!(f.profile, 1);
        assert_eq!(f.image, 0);
        assert_eq!(f.root, RootChoice::Shell);
        assert_eq!(f.name, "Phone 6.1 FHD+ 2");
        assert_eq!(
            (f.ram_gb.as_str(), f.cpus.as_str(), f.screen.as_str()),
            ("4", "4", "1080×2400")
        );
        assert_eq!(
            default_root(images.get(1), RootChoice::Shell),
            RootChoice::None
        );
    }

    #[test]
    fn create_params_send_only_overrides_and_validate() {
        let images = sorted_images(&[
            img(35, "google_apis", true),
            img(33, "google_apis_playstore", true),
        ]);
        let mut f = new_form(&profiles(), &images, &[], RootChoice::Shell);
        let p = create_params(&f, &profiles(), &images).unwrap();
        assert_eq!(p["profile"], "phone-6.1-fhd");
        assert_eq!(p["root_mode"], "adb");
        assert!(p.get("ram_mb").is_none());
        f.ram_gb = "6".into();
        f.screen = "1440x3120".into();
        let p = create_params(&f, &profiles(), &images).unwrap();
        assert_eq!(p["ram_mb"], 6144);
        assert_eq!(
            (p["width"].as_u64(), p["height"].as_u64()),
            (Some(1440), Some(3120))
        );
        f.image = 1;
        assert!(
            create_params(&f, &profiles(), &images)
                .unwrap_err()
                .contains("Play Store")
        );
        f.root = RootChoice::Full;
        f.cpus = "x".into();
        assert!(
            create_params(&f, &profiles(), &images)
                .unwrap_err()
                .contains("CPU")
        );
        f.cpus = "4".into();
        f.name = "  ".into();
        assert!(
            create_params(&f, &profiles(), &images)
                .unwrap_err()
                .contains("name")
        );
    }

    #[test]
    fn image_rows_show_install_state() {
        let images = [img(35, "google_apis", true), img(36, "google_apis", false)];
        let rows = image_rows(&images, &[]);
        assert_eq!(rows[0].status, "Installed · 2.0 GB");
        assert_eq!(rows[1].status, "Download · 2.0 GB");
        assert!(rows[1].enabled);
    }

    /// The edit sheet shows root-mode cards (9e); a Play Store device must
    /// keep Shell root disabled there, so the form carries the image kind.
    #[test]
    fn edit_form_knows_play_store_devices() {
        let profiles = profiles();
        let mut info = crate::fixtures::device_infos()[1].clone();
        let hib = HibernateChoices::device(0, None);
        assert!(!edit_form(&info, &profiles, &hib).play_store);
        info.device.image.tag = "google_apis_playstore".into();
        info.device.root.mode = jidr_core::RootMode::Magisk;
        let form = edit_form(&info, &profiles, &hib);
        assert!(form.play_store && form.edit);
        assert_eq!(form.root, RootChoice::Full);
    }

    fn with_lean(mut v: Vec<HardwareProfile>) -> Vec<HardwareProfile> {
        let mut lean = v[0].clone();
        lean.id = LEAN_PROFILE.into();
        lean.name = "Agent lean".into();
        v.push(lean);
        v
    }

    #[test]
    fn lean_preset_picks_the_lean_profile_for_a_new_device() {
        let plain = profiles();
        let profiles = with_lean(plain.clone());
        let images = sorted_images(&[img(35, "google_apis", true)]);
        let mut f = new_form(&profiles, &images, &[], RootChoice::Shell);
        assert!(!f.lean);
        apply_lean(&mut f, &profiles).unwrap();
        assert!(f.lean);
        assert_eq!(f.profile, 2);
        assert_eq!((f.ram_gb.as_str(), f.screen.as_str()), ("2", "720×1280"));
        let p = create_params(&f, &profiles, &images).unwrap();
        assert_eq!(p["profile"], LEAN_PROFILE);
        assert!(p.get("ram_mb").is_none());
        // Another profile clears it.
        apply_profile(&mut f, &profiles, 1);
        assert!(!f.lean);
        assert!(apply_lean(&mut f, &plain).is_err());
    }

    #[test]
    fn lean_preset_on_an_existing_device_edits_its_hardware() {
        let profiles = with_lean(profiles());
        let info = crate::fixtures::device_infos()[1].clone();
        let hib = HibernateChoices::device(0, None);
        let mut f = edit_form(&info, &profiles, &hib);
        apply_lean(&mut f, &profiles).unwrap();
        assert!(f.lean && f.edit);
        let p = edit_params(&f, &info, &hib).unwrap();
        assert_eq!(p["ram_mb"], 2048);
        assert_eq!(p["cpus"], 2);
        assert_eq!(
            (p["width"].as_u64(), p["height"].as_u64()),
            (Some(720), Some(1280))
        );
        assert!(p.get("profile").is_none());
        assert!(p.get("auto_hibernate_minutes").is_none());
    }

    #[test]
    fn auto_hibernate_choices_round_trip() {
        let hib = HibernateChoices::device(30, Some(45));
        let labels: Vec<_> = hib.options.iter().map(|o| o.label.to_string()).collect();
        assert_eq!(labels[0], "Same as Settings (after 30\u{a0}min idle)");
        assert_eq!(labels[1], "Never");
        assert_eq!(labels.last().unwrap(), "After 45\u{a0}min idle");
        assert_eq!(hib.index_of(Some(45)), 6);
        assert_eq!(idle_after(120), "After 2\u{a0}h idle");
        assert_eq!(HibernateChoices::global(0).options[0].label, "Off");

        let profiles = profiles();
        let mut info = crate::fixtures::device_infos()[1].clone();
        info.device.auto_hibernate_minutes = Some(15);
        let hib = HibernateChoices::device(0, Some(15));
        let mut f = edit_form(&info, &profiles, &hib);
        assert_eq!(hib.value(f.hibernate), Some(Some(15)));
        assert!(
            edit_params(&f, &info, &hib)
                .unwrap()
                .get("auto_hibernate_minutes")
                .is_none()
        );
        f.hibernate = 0;
        let p = edit_params(&f, &info, &hib).unwrap();
        assert!(p["auto_hibernate_minutes"].is_null() && p.get("auto_hibernate_minutes").is_some());
        f.hibernate = 1;
        assert_eq!(
            edit_params(&f, &info, &hib).unwrap()["auto_hibernate_minutes"],
            0
        );
    }
}
