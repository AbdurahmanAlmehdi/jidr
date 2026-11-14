//! Command palette (⌘K): every launcher action plus one entry per device,
//! fuzzy-filtered (letters in order, case-insensitive).

use crate::i18n::tr;
use crate::{DevKind, DevState, DeviceVm, PaletteItem, PaletteKind};

fn item(kind: PaletteKind, title: &str, subtitle: &str, kbd: &str, action: String) -> PaletteItem {
    PaletteItem {
        kind,
        title: title.into(),
        subtitle: subtitle.into(),
        kbd: kbd.into(),
        action: action.into(),
        state: DevState::Stopped,
    }
}

/// All entries for the current devices.
pub fn items(devices: &[DeviceVm], sdk_ready: bool) -> Vec<PaletteItem> {
    let mut v = Vec::new();
    for d in devices {
        let mut open = item(
            PaletteKind::Device,
            &tr!("Open {}", d.name),
            &d.status,
            "",
            format!("open:{}", d.id),
        );
        open.state = d.state;
        v.push(open);
    }
    if !sdk_ready {
        v.push(item(
            PaletteKind::Action,
            &tr!("Set up Android SDK"),
            &tr!("Find or install the emulator and an image"),
            "⌘↵",
            "setup".into(),
        ));
    }
    v.push(item(
        PaletteKind::Action,
        &tr!("New device"),
        &tr!("Create a device from a profile"),
        "⌘N",
        "new".into(),
    ));
    for d in devices {
        if d.kind == DevKind::Base {
            // Bases never start; agents lease clones of them.
            continue;
        }
        match d.state {
            DevState::Hibernated => v.push(item(
                PaletteKind::Action,
                &tr!("Resume {}", d.name),
                &tr!("Back where it was, in seconds"),
                "",
                format!("start:{}", d.id),
            )),
            DevState::Stopped | DevState::Crashed => v.push(item(
                PaletteKind::Action,
                &tr!("Start {}", d.name),
                &d.android,
                "",
                format!("start:{}", d.id),
            )),
            DevState::Running | DevState::Booting | DevState::Starting => v.push(item(
                PaletteKind::Action,
                &tr!("Stop {}", d.name),
                &tr!("Saves a quick-boot snapshot"),
                "",
                format!("stop:{}", d.id),
            )),
            _ => {}
        }
        if d.state == DevState::Running && d.kind == DevKind::Device {
            v.push(item(
                PaletteKind::Action,
                &tr!("Hibernate {}", d.name),
                &tr!("Saves its whole state and frees its RAM"),
                "",
                format!("hibernate:{}", d.id),
            ));
        }
        if d.can_base {
            v.push(item(
                PaletteKind::Action,
                &tr!("Save {} as a base…", d.name),
                &tr!("For agents' throwaway clones (pool.get)"),
                "",
                format!("base:{}", d.id),
            ));
        }
        if d.state == DevState::Stopped && d.kind == DevKind::Device {
            v.push(item(
                PaletteKind::Action,
                &tr!("Edit {}", d.name),
                &tr!("Name, RAM, CPU, screen"),
                "",
                format!("edit:{}", d.id),
            ));
        }
    }
    v.push(item(
        PaletteKind::Setting,
        &tr!("Settings"),
        &tr!("SDK, storage, agents, about"),
        "⌘,",
        "settings".into(),
    ));
    v.push(item(
        PaletteKind::Setting,
        &tr!("Add Jidr to Claude"),
        &tr!("MCP setup"),
        "",
        "settings:agents".into(),
    ));
    v.push(item(
        PaletteKind::Setting,
        &tr!("Toggle light and dark"),
        &tr!("Appearance"),
        "",
        "appearance".into(),
    ));
    v
}

/// Letters of `query` appear in `text` in order (case-insensitive).
pub fn matches(text: &str, query: &str) -> bool {
    let mut t = text.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .all(|q| t.any(|c| c == q))
}

/// Entries matching `query`; contiguous matches first.
pub fn filter(all: &[PaletteItem], query: &str) -> Vec<PaletteItem> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return all.to_vec();
    }
    let mut v: Vec<(bool, PaletteItem)> = all
        .iter()
        .filter(|i| matches(&i.title, &q) || matches(&i.subtitle, &q))
        .map(|i| (!i.title.to_lowercase().contains(&q), i.clone()))
        .collect();
    v.sort_by_key(|(rank, _)| *rank);
    v.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_matching_is_ordered_and_case_insensitive() {
        assert!(matches("Open Pixel-ish 6.1", "pix61"));
        assert!(matches("New device", "nd"));
        assert!(!matches("New device", "dn"));
    }

    #[test]
    fn devices_get_open_and_lifecycle_entries() {
        let devices = crate::fixtures::devices();
        let all = items(&devices, true);
        assert!(
            all.iter()
                .any(|i| i.action == format!("open:{}", devices[0].id).as_str())
        );
        assert!(all.iter().any(|i| i.title == "Stop Pixel-ish 6.1"));
        assert!(all.iter().any(|i| i.title == "Start Tablet 11"));
        let f = filter(&all, "tablet");
        assert!(f.iter().all(|i| i.title.to_lowercase().contains("tablet")
            || i.subtitle.to_lowercase().contains("tablet")));
        assert_eq!(filter(&all, "").len(), all.len());
    }

    #[test]
    fn hibernate_resume_and_bases() {
        let devices = crate::fixtures::vms(&crate::fixtures::hibernated_infos());
        let all = items(&devices, true);
        let titles: Vec<_> = all.iter().map(|i| i.title.to_string()).collect();
        assert!(titles.contains(&"Resume Tablet 11".to_string()));
        assert!(!titles.contains(&"Start Tablet 11".to_string()));
        assert!(titles.contains(&"Hibernate Pixel-ish 6.1".to_string()));
        // Full root can't become a base (base.create takes none or adb).
        assert!(!titles.iter().any(|t| t.starts_with("Save Pixel-ish")));

        let pool = crate::fixtures::vms(&crate::fixtures::pool_infos());
        let titles: Vec<_> = items(&pool, true)
            .iter()
            .map(|i| i.title.to_string())
            .collect();
        assert!(!titles.contains(&"Start Shop base".to_string()));
        assert!(!titles.contains(&"Hibernate Shop base 1".to_string()));
    }
}
