//! Snapshot manager (Phase 10b): `snapshot.*` for the selected device,
//! thumbnails from the emulator's snapshot folders, D29 refusals.

use std::path::Path;
use std::rc::Rc;

use chrono::{DateTime, Utc};
use jidr_core::RootMode;
use serde_json::{Value, json};
use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};
use uuid::Uuid;

use crate::app::{Ctx, report, toast};
use crate::model::{self, Clock};
use crate::{RootKind, SnapshotVm};

pub fn mode_name(m: RootMode) -> &'static str {
    match m {
        RootMode::None => "Not rooted",
        RootMode::Adb => "Shell root",
        RootMode::Magisk => "Full root",
    }
}

fn root_kind(m: RootMode) -> RootKind {
    match m {
        RootMode::None => RootKind::None,
        RootMode::Adb => RootKind::Shell,
        RootMode::Magisk => RootKind::Full,
    }
}

/// One row of `snapshot.list`.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub quick_boot: bool,
    pub root_mode: Option<RootMode>,
    pub magisk: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub size: u64,
    pub thumbnail: Option<String>,
    pub restorable: bool,
}

pub fn parse(v: &Value) -> Vec<Entry> {
    v.get("snapshots")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|s| Entry {
            name: s["name"].as_str().unwrap_or_default().to_string(),
            quick_boot: s["quick_boot"].as_bool().unwrap_or(false),
            root_mode: serde_json::from_value(s["root_mode"].clone()).ok(),
            magisk: s["magisk_version"].as_str().map(str::to_string),
            created_at: serde_json::from_value(s["created_at"].clone()).ok(),
            size: s["size_bytes"].as_u64().unwrap_or(0),
            thumbnail: s["thumbnail"].as_str().map(str::to_string),
            restorable: s["restorable"].as_bool().unwrap_or(true),
        })
        .collect()
}

/// Quick boot last, newest named snapshots first.
pub fn sorted(mut v: Vec<Entry>) -> Vec<Entry> {
    v.sort_by(|a, b| {
        a.quick_boot
            .cmp(&b.quick_boot)
            .then(a.created_at.cmp(&b.created_at))
    });
    v
}

fn thumbnail(path: &str) -> Option<Image> {
    let bytes = std::fs::read(Path::new(path)).ok()?;
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba {
        return None;
    }
    buf.truncate(info.buffer_size());
    Some(Image::from_rgba8(
        SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&buf, info.width, info.height),
    ))
}

pub fn vm(e: &Entry, clock: &Clock) -> SnapshotVm {
    let thumb = e.thumbnail.as_deref().and_then(thumbnail);
    SnapshotVm {
        name: if e.quick_boot {
            "Quick boot".into()
        } else {
            e.name.clone().into()
        },
        tag: match e.root_mode {
            Some(m) => format!("Taken with: {}", mode_name(m)),
            None => "Not tagged".into(),
        }
        .into(),
        root: e.root_mode.map(root_kind).unwrap_or(RootKind::None),
        auto: e.quick_boot,
        size: if e.size > 0 {
            model::gb(e.size)
        } else {
            String::new()
        }
        .into(),
        when: e
            .created_at
            .map(|t| clock.local(t).format("%b %-d, %H:%M").to_string())
            .unwrap_or_default()
            .into(),
        refused: !e.restorable,
        has_thumb: thumb.is_some(),
        thumb: thumb.unwrap_or_default(),
    }
}

/// The banner above the cards.
pub fn banner(device: &str, mode: RootMode, entries: &[Entry]) -> (String, bool) {
    let named: Vec<&Entry> = entries.iter().filter(|e| !e.quick_boot).collect();
    let refused: Vec<&Entry> = named.iter().copied().filter(|e| !e.restorable).collect();
    let total: u64 = named.iter().map(|e| e.size).sum();
    if let Some(r) = refused.first() {
        let other = r.root_mode.map(mode_name).unwrap_or("another root mode");
        return (
            format!(
                "{device} is {} now. Snapshots taken with {other} can't be restored until {other} is set up again.",
                mode_name(mode)
            ),
            true,
        );
    }
    if named.is_empty() {
        return (String::new(), false);
    }
    let all = if named.len() == 1 {
        "it".to_string()
    } else {
        format!("all {}", named.len())
    };
    (
        format!(
            "Every snapshot remembers the root mode it was taken with. {device} is {} now, so {all} can be restored. {} in total.",
            mode_name(mode),
            model::gb(total)
        ),
        false,
    )
}

thread_local! {
    static ENTRIES: std::cell::RefCell<Vec<Entry>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn selected(ctx: &Ctx) -> Option<(Uuid, String, RootMode, bool, Option<String>)> {
    let st = ctx.state.borrow();
    let id = st
        .selected
        .or_else(|| st.data.devices.first().map(|d| d.device.id))?;
    let d = st.data.devices.iter().find(|d| d.device.id == id)?;
    Some((
        id,
        d.device.name.clone(),
        d.device.root.mode,
        matches!(d.device.status, jidr_core::DeviceStatus::Running),
        d.device.root.magisk_version.clone(),
    ))
}

/// Loads the selected device's snapshots into the window.
pub fn load(ctx: &Rc<Ctx>) {
    let Some((id, name, mode, running, _)) = selected(ctx) else {
        return;
    };
    ctx.app.set_snap_can_save(running);
    let token = crate::app::defer(move |r: jidr_core::JidrResult<Value>| {
        let Some(c) = crate::app::ctx() else { return };
        // Still the same device?
        if selected(&c).map(|s| s.0) != Some(id) {
            return;
        }
        let entries = sorted(r.map(|v| parse(&v)).unwrap_or_default());
        let clock = Clock::system();
        let vms: Vec<SnapshotVm> = entries.iter().map(|e| vm(e, &clock)).collect();
        let (text, warn) = banner(&name, mode, &entries);
        c.app.set_snapshots(ModelRc::new(VecModel::from(vms)));
        c.app.set_snap_banner(text.into());
        c.app.set_snap_banner_warning(warn);
        ENTRIES.with(|e| *e.borrow_mut() = entries);
    });
    ctx.bridge
        .call("snapshot.list", json!({ "device": id }), move |r| {
            crate::app::fire(token, r)
        });
}

fn entry(i: i32) -> Option<Entry> {
    ENTRIES.with(|e| {
        usize::try_from(i)
            .ok()
            .and_then(|i| e.borrow().get(i).cloned())
    })
}

pub fn wire(ctx: &Rc<Ctx>) {
    let c = ctx.clone();
    ctx.app.on_manage_snapshots(move |_| {
        c.app.set_snapshots_open(true);
        load(&c);
    });

    let c = ctx.clone();
    ctx.app.on_restore_snapshot(move |_, i| {
        let Some(e) = entry(i) else { return };
        let Some((id, name, mode, running, _)) = selected(&c) else { return };
        if !e.restorable {
            let other = e.root_mode.unwrap_or(RootMode::None);
            c.app.set_refuse_name(e.name.clone().into());
            c.app.set_refuse_snapshot_line(
                format!(
                    "Taken with: {}{}",
                    mode_name(other),
                    e.magisk.as_ref().map(|v| format!(" · Magisk {v}")).unwrap_or_default()
                )
                .into(),
            );
            c.app.set_refuse_device_line(mode_name(mode).into());
            c.app.set_refuse_body(
                format!(
                    "It was taken with {}. {name} is {} now, so restoring it would bring back a root setup Jidr hasn't verified on this device.",
                    mode_name(other),
                    mode_name(mode)
                )
                .into(),
            );
            c.app.set_refuse_hint(
                format!(
                    "Set up {} first, then restore. Snapshots taken while {} restore as usual.",
                    mode_name(other),
                    mode_name(mode)
                )
                .into(),
            );
            c.app.set_refuse_action(format!("Set up {}…", mode_name(other)).into());
            c.app.set_refuse_open(true);
            return;
        }
        if !running {
            toast(&c.app.as_weak(), format!("Start {name} to restore a snapshot"));
            return;
        }
        let w = c.app.as_weak();
        let label = if e.quick_boot { "Quick boot".to_string() } else { e.name.clone() };
        c.bridge.run(
            "snapshot.load",
            json!({ "device": id, "name": e.name }),
            |_| {},
            report(&w, &format!("Restored “{label}”")),
        );
    });

    let c = ctx.clone();
    ctx.app.on_refuse_act(move || {
        let Some((id, ..)) = selected(&c) else { return };
        let target = ENTRIES.with(|e| {
            e.borrow()
                .iter()
                .find(|x| x.name == c.app.get_refuse_name().as_str())
                .and_then(|x| x.root_mode)
        });
        let mode = match target {
            Some(RootMode::Magisk) => "magisk",
            Some(RootMode::Adb) => "adb",
            _ => "none",
        };
        let w = c.app.as_weak();
        // root.set is Stream C's command (9c/9d).
        c.bridge.run(
            "root.set",
            json!({ "device": id, "mode": mode }),
            |_| {},
            report(&w, "Root set up; the snapshot can be restored now"),
        );
    });

    let c = ctx.clone();
    ctx.app.on_snapshot_save_request(move || {
        let Some((_, name, mode, running, magisk)) = selected(&c) else {
            return;
        };
        if !running {
            toast(&c.app.as_weak(), format!("Start {name} to save a snapshot"));
            return;
        }
        let sizes: Vec<String> = ENTRIES.with(|e| {
            e.borrow()
                .iter()
                .filter(|x| !x.quick_boot && x.size > 0)
                .map(|x| format!("{:.1}", x.size as f64 / 1e9))
                .collect()
        });
        c.app.set_save_name(
            chrono::Local::now()
                .format("Snapshot %b %-d, %H.%M")
                .to_string()
                .into(),
        );
        c.app.set_save_root_kind(root_kind(mode));
        c.app.set_save_root_tag(
            format!(
                "Taken with: {}{}",
                mode_name(mode),
                magisk
                    .filter(|_| mode == RootMode::Magisk)
                    .map(|v| format!(" · Magisk {v}"))
                    .unwrap_or_default()
            )
            .into(),
        );
        c.app.set_save_hint(match sizes.len() {
            0 => "Snapshots take about as much disk as the device's data.".into(),
            1 => format!("Its other snapshot is {} GB.", sizes[0]).into(),
            _ => format!(
                "Its other snapshots are {} and {} GB.",
                sizes[..sizes.len() - 1].join(", "),
                sizes[sizes.len() - 1]
            )
            .into(),
        });
        c.app.set_save_open(true);
    });

    let c = ctx.clone();
    ctx.app.on_snapshot_save(move |name| {
        let Some((id, ..)) = selected(&c) else { return };
        let w = c.app.as_weak();
        let token = crate::app::defer(move |r: jidr_core::JidrResult<Value>| {
            if let Some(c) = crate::app::ctx() {
                load(&c);
                if let Err(e) = r {
                    toast(&c.app.as_weak(), e.message);
                }
            }
        });
        toast(&w, format!("Saving “{name}”…"));
        let w2 = w.clone();
        let n = name.to_string();
        c.bridge.run(
            "snapshot.save",
            json!({ "device": id, "name": name.as_str() }),
            |_| {},
            move |r| {
                if r.is_ok() {
                    toast(&w2, format!("Saved snapshot “{n}”"));
                }
                crate::app::fire(token, r);
            },
        );
    });

    let c = ctx.clone();
    ctx.app.on_snapshot_delete(move |i| {
        let Some(e) = entry(i) else { return };
        let Some((id, ..)) = selected(&c) else { return };
        let c2 = c.clone();
        crate::launcher::confirm(
            &c,
            &format!("Delete “{}”?", e.name),
            &format!(
                "Frees {}. The device itself doesn't change. This can't be undone.",
                if e.size > 0 {
                    model::gb(e.size)
                } else {
                    "its disk space".into()
                }
            ),
            "Delete",
            move || {
                let token = crate::app::defer(move |r: jidr_core::JidrResult<Value>| {
                    if let Some(c) = crate::app::ctx() {
                        load(&c);
                        match r {
                            Ok(_) => toast(&c.app.as_weak(), "Deleted the snapshot".into()),
                            Err(e) => toast(&c.app.as_weak(), e.message),
                        }
                    }
                });
                c2.bridge.call(
                    "snapshot.delete",
                    json!({ "device": id, "name": e.name }),
                    move |r| crate::app::fire(token, r),
                );
            },
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> Value {
        json!({ "device_root_mode": "none", "snapshots": [
            { "name": "default_boot", "quick_boot": true, "root_mode": null, "size_bytes": 0, "restorable": true },
            { "name": "Clean install", "root_mode": "magisk", "magisk_version": "30.7",
              "created_at": "2026-10-09T18:40:00Z", "size_bytes": 2100000000u64, "restorable": false },
            { "name": "Logged in", "root_mode": "none", "created_at": "2026-10-13T11:02:00Z",
              "size_bytes": 2400000000u64, "restorable": true }
        ]})
    }

    #[test]
    fn list_parses_and_sorts_quick_boot_last() {
        let e = sorted(parse(&list()));
        assert_eq!(
            e.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(),
            ["Clean install", "Logged in", "default_boot"]
        );
        let clock = crate::fixtures::clock();
        let v = vm(&e[0], &clock);
        assert_eq!(v.tag, "Taken with: Full root");
        assert!(v.refused);
        assert_eq!(v.size, "2.1 GB");
        assert_eq!(vm(&e[2], &clock).name, "Quick boot");
    }

    #[test]
    fn banner_explains_refusals() {
        let e = parse(&list());
        let (text, warn) = banner("Pixel-ish 6.1", RootMode::None, &e);
        assert!(warn);
        assert!(
            text.contains("Not rooted now") && text.contains("Full root"),
            "{text}"
        );
        let ok: Vec<Entry> = e
            .into_iter()
            .map(|mut x| {
                x.restorable = true;
                x
            })
            .collect();
        let (text, warn) = banner("Pixel-ish 6.1", RootMode::Magisk, &ok);
        assert!(!warn);
        assert!(
            text.contains("all 2 can be restored") && text.contains("4.5 GB"),
            "{text}"
        );
    }
}
