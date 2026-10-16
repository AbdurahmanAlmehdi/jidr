//! Settings: GUI preferences (pure view state, kept in `gui-prefs.json`),
//! storage rows from daemon data, and the "Add Jidr to Claude" config merge.

use std::path::{Path, PathBuf};

use jidr_core::SystemImage;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use slint::ComponentHandle;

use crate::model::{self, DeviceInfo};
use crate::{OptionVm, RootChoice, UsageRow};

/// The OpenStreetMap tile server (D14); attribution is always shown.
pub const OSM_TILES: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// 0 System, 1 Light, 2 Dark.
    pub appearance: u8,
    /// `none` | `adb` | `magisk`. A copy of the daemon's `default_root_mode`
    /// (gap B4: the daemon owns it, so the CLI and agents agree), refreshed on
    /// connect; the create sheet and Setup read it.
    pub default_root: String,
    /// Empty = OpenStreetMap.
    pub tile_url: String,
    /// A copy of the daemon's `quick_boot` setting (D29, gap B4).
    pub quick_boot: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            appearance: 0,
            default_root: "adb".into(),
            tile_url: String::new(),
            quick_boot: true,
        }
    }
}

impl Prefs {
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("gui-prefs.json")
    }

    pub fn load(data_dir: &Path) -> Self {
        std::fs::read_to_string(Self::path(data_dir))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(data_dir)?;
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(Self::path(data_dir), text)
    }

    /// Takes the daemon's settings (`settings.get` / `settings.set` result).
    pub fn apply_daemon_settings(&mut self, v: &Value) {
        if let Some(m) = v
            .get("default_root_mode")
            .and_then(Value::as_str)
            .filter(|m| matches!(*m, "none" | "adb" | "magisk"))
        {
            self.default_root = m.to_string();
        }
        if let Some(q) = v.get("quick_boot").and_then(Value::as_bool) {
            self.quick_boot = q;
        }
    }

    pub fn root_choice(&self) -> RootChoice {
        match self.default_root.as_str() {
            "none" => RootChoice::None,
            "magisk" => RootChoice::Full,
            _ => RootChoice::Shell,
        }
    }

    pub fn tiles(&self) -> &str {
        if self.tile_url.is_empty() {
            OSM_TILES
        } else {
            &self.tile_url
        }
    }
}

pub fn tile_options() -> Vec<OptionVm> {
    vec![
        OptionVm {
            id: "osm".into(),
            label: "OpenStreetMap (default)".into(),
        },
        OptionVm {
            id: "custom".into(),
            label: "Custom".into(),
        },
    ]
}

/// Per-device disk use, largest first, plus a total row (`fraction` −1).
pub fn device_usage(
    devices: &[DeviceInfo],
    snapshot_counts: &[(uuid::Uuid, usize)],
) -> Vec<UsageRow> {
    let mut v: Vec<(&DeviceInfo, u64)> = devices
        .iter()
        .map(|d| (d, d.disk_bytes.unwrap_or(0)))
        .collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.1));
    let max = v.first().map(|x| x.1).unwrap_or(0).max(1);
    let total: u64 = v.iter().map(|x| x.1).sum();
    let mut rows: Vec<UsageRow> = v
        .iter()
        .map(|(d, bytes)| {
            let snaps = snapshot_counts
                .iter()
                .find(|(id, _)| *id == d.device.id)
                .map(|(_, n)| *n)
                .unwrap_or(0);
            UsageRow {
                name: d.device.name.clone().into(),
                sub: match snaps {
                    0 => String::new(),
                    1 => "1 snapshot".into(),
                    n => format!("{n} snapshots"),
                }
                .into(),
                fraction: *bytes as f32 / max as f32,
                size: model::gb(*bytes).into(),
            }
        })
        .collect();
    if !rows.is_empty() {
        rows.push(UsageRow {
            name: "Total".into(),
            sub: "".into(),
            fraction: -1.0,
            size: model::gb(total).into(),
        });
    }
    rows
}

/// Installed images with how many devices use them.
pub fn image_usage(images: &[SystemImage], devices: &[DeviceInfo]) -> Vec<UsageRow> {
    images
        .iter()
        .filter(|i| i.installed_in.is_some())
        .map(|i| {
            let n = devices.iter().filter(|d| d.device.image == i.image).count();
            UsageRow {
                name: crate::device_form::image_label(i).into(),
                sub: match n {
                    0 => "No devices".to_string(),
                    1 => "1 device".to_string(),
                    n => format!("{n} devices"),
                }
                .into(),
                fraction: 0.0,
                size: if i.size > 0 {
                    model::gb(i.size)
                } else {
                    "—".into()
                }
                .into(),
            }
        })
        .collect()
}

/// Claude Desktop's config file.
pub fn claude_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("Claude/claude_desktop_config.json"))
}

/// Adds (or replaces) the `jidr` MCP server, keeping everything else.
pub fn merge_claude_config(existing: Option<&str>, jidr_bin: &str) -> Result<String, String> {
    let mut root: Value = match existing.map(str::trim) {
        Some(t) if !t.is_empty() => serde_json::from_str(t)
            .map_err(|e| format!("claude_desktop_config.json isn't valid JSON: {e}"))?,
        _ => json!({}),
    };
    let obj = root
        .as_object_mut()
        .ok_or("claude_desktop_config.json isn't a JSON object")?;
    let servers = obj
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("mcpServers isn't a JSON object")?;
    servers.insert(
        "jidr".into(),
        json!({ "command": jidr_bin, "args": ["mcp"] }),
    );
    serde_json::to_string_pretty(&root).map_err(|e| e.to_string())
}

/// Writes the merged config.
pub fn add_to_claude(path: &Path, jidr_bin: &str) -> Result<(), String> {
    let existing = std::fs::read_to_string(path).ok();
    let merged = merge_claude_config(existing.as_deref(), jidr_bin)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, merged).map_err(|e| e.to_string())
}

/// Copies text to the macOS pasteboard.
pub fn copy_text(text: &str) {
    use std::io::Write as _;
    if let Ok(mut child) = std::process::Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

/// Opens a URL or reveals a path with macOS `open`.
pub fn open_external(args: &[&str]) {
    let _ = std::process::Command::new("open").args(args).spawn();
}

fn models<T: Clone + 'static>(v: Vec<T>) -> slint::ModelRc<T> {
    slint::ModelRc::new(slint::VecModel::from(v))
}

/// Opens the Settings window on `tab` (0 General, 1 Storage, 2 Agents, 3 About).
pub fn open(ctx: &std::rc::Rc<crate::app::Ctx>, tab: i32) {
    let Ok(win) = crate::SettingsWindow::new() else {
        return;
    };
    let theme = ctx.app.global::<crate::Theme>();
    let wt = win.global::<crate::Theme>();
    wt.set_dark(theme.get_dark());
    wt.set_rtl(theme.get_rtl());
    wt.set_reduce_motion(theme.get_reduce_motion());
    win.set_draw_lights(false);
    win.set_tab(tab);
    win.set_version(env!("CARGO_PKG_VERSION").into());
    let prefs = ctx.prefs.borrow().clone();
    win.set_default_root(prefs.root_choice());
    win.set_tile_options(models(tile_options()));
    win.set_tile_choice(if prefs.tile_url.is_empty() { 0 } else { 1 });
    win.set_tile_url(prefs.tile_url.clone().into());
    win.set_appearance(i32::from(prefs.appearance));
    win.set_quick_boot(prefs.quick_boot);
    let devices = ctx.state.borrow().data.devices.clone();
    win.set_device_usage(models(device_usage(&devices, &[])));
    let cache = dir_size(&ctx.paths.cache);
    win.set_cache_size(model::gb(cache).into());

    // SDK state and images come from the daemon.
    let weak = win.as_weak();
    let b2 = ctx.bridge.clone();
    let token = crate::app::defer(move |(status, images): (Value, Vec<SystemImage>)| {
        let Some(win) = weak.upgrade() else { return };
        let sdk = status
            .get("tools_sdk")
            .or_else(|| status.get("private_sdk"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let ready = crate::bridge::sdk_has_tools(&status);
        win.set_sdk_path(sdk.into());
        win.set_sdk_ready(ready);
        win.set_sdk_state(if ready { "Ready" } else { "Not installed" }.into());
        win.set_sdk_note(
            status
                .get("emulator_version")
                .and_then(Value::as_str)
                .map(|v| format!("Emulator {v}"))
                .unwrap_or_default()
                .into(),
        );
        win.set_image_usage(models(image_usage(&images, &devices)));
    });
    ctx.bridge.call("sdk.status", json!({}), move |s| {
        let status = s.unwrap_or(Value::Null);
        b2.call("image.list", json!({ "available": false }), move |i| {
            let images: Vec<SystemImage> = i
                .ok()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            crate::app::fire(token, (status, images));
        });
    });

    // The daemon's settings may have changed (CLI, an agent): show them.
    let weak = win.as_weak();
    let token = crate::app::defer(move |v: Option<Value>| {
        let (Some(v), Some(c), Some(win)) = (v, crate::app::ctx(), weak.upgrade()) else {
            return;
        };
        c.prefs.borrow_mut().apply_daemon_settings(&v);
        let prefs = c.prefs.borrow();
        win.set_default_root(prefs.root_choice());
        win.set_quick_boot(prefs.quick_boot);
    });
    ctx.bridge.call("settings.get", json!({}), move |r| {
        crate::app::fire(token, r.ok());
    });

    let c = ctx.clone();
    win.on_default_root_changed(move |choice| {
        let mode = crate::device_form::root_mode(choice);
        c.prefs.borrow_mut().default_root = mode.into();
        let _ = c.prefs.borrow().save(&c.paths.data);
        set_daemon_setting(&c, json!({ "default_root_mode": mode }));
    });
    let c = ctx.clone();
    win.on_tiles_changed(move |choice, url| {
        c.prefs.borrow_mut().tile_url = if choice == 0 {
            String::new()
        } else {
            url.to_string()
        };
        let _ = c.prefs.borrow().save(&c.paths.data);
    });
    let c = ctx.clone();
    let w = win.as_weak();
    win.on_appearance_changed(move |i| {
        c.prefs.borrow_mut().appearance = u8::try_from(i).unwrap_or(0);
        let _ = c.prefs.borrow().save(&c.paths.data);
        c.apply_appearance();
        if let Some(win) = w.upgrade() {
            win.global::<crate::Theme>()
                .set_dark(c.app.global::<crate::Theme>().get_dark());
        }
    });
    let c = ctx.clone();
    win.on_quick_boot_changed(move |v| {
        c.prefs.borrow_mut().quick_boot = v;
        let _ = c.prefs.borrow().save(&c.paths.data);
        set_daemon_setting(&c, json!({ "quick_boot": v }));
    });
    let data = ctx.paths.data.clone();
    win.on_show_sdk(move || {
        let sdk = data.join("sdk");
        open_external(&["-R", &sdk.to_string_lossy()]);
    });
    win.on_change_sdk(|| {});
    let c = ctx.clone();
    let cache_dir = ctx.paths.cache.clone();
    let w = win.as_weak();
    win.on_clear_cache(move || {
        let _ = std::fs::remove_dir_all(&cache_dir);
        if let Some(win) = w.upgrade() {
            win.set_cache_size(model::gb(0).into());
        }
        crate::app::toast(&c.app.as_weak(), "Cleared the download cache".into());
    });
    let c = ctx.clone();
    win.on_add_to_claude(move || {
        // The CLI that serves MCP, never the GUI itself (see bundled_jidr).
        let bin = jidr_client::SpawnOptions::default()
            .program()
            .to_string_lossy()
            .into_owned();
        let msg = match claude_config_path() {
            Some(path) => match add_to_claude(&path, &bin) {
                Ok(()) => "Added Jidr to Claude Desktop. Restart Claude to use it.".to_string(),
                Err(e) => e,
            },
            None => "Can't find Claude Desktop's settings folder".to_string(),
        };
        crate::app::toast(&c.app.as_weak(), msg);
    });
    let c = ctx.clone();
    win.on_copy(move |t| {
        copy_text(&t);
        crate::app::toast(&c.app.as_weak(), "Copied".into());
    });
    let repo = env!("CARGO_PKG_REPOSITORY");
    win.on_open_source(move || open_external(&[repo]));
    win.on_sponsor(move || open_external(&[repo]));
    win.on_check_updates(move || open_external(&[&format!("{repo}/releases")]));
    win.on_show_licences(move || open_external(&[&format!("{repo}/blob/main/NOTICE")]));

    let _ = win.show();
    ctx.windows.borrow_mut().push(Box::new(win));
}

/// Bytes under `dir` (cache size).
pub fn dir_size(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    rd.filter_map(Result::ok)
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

/// Gap B4: refreshes the cached default root mode and quick boot from the
/// daemon (on connect, so the create sheet follows `jidr settings`).
pub fn sync_from_daemon(ctx: &crate::app::Ctx) {
    let token = crate::app::defer(move |v: Option<Value>| {
        if let (Some(v), Some(c)) = (v, crate::app::ctx()) {
            c.prefs.borrow_mut().apply_daemon_settings(&v);
        }
    });
    ctx.bridge.call("settings.get", json!({}), move |r| {
        crate::app::fire(token, r.ok());
    });
}

/// `settings.set`; a failure is shown as a toast.
fn set_daemon_setting(ctx: &crate::app::Ctx, params: Value) {
    let weak = ctx.app.as_weak();
    ctx.bridge.call("settings.set", params, move |r| {
        if let Err(e) = r {
            crate::app::toast(&weak, format!("Couldn't save the setting: {}", e.message));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_round_trip_and_default() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(Prefs::load(tmp.path()), Prefs::default());
        let p = Prefs {
            appearance: 2,
            default_root: "magisk".into(),
            tile_url: "https://t.example/{z}/{x}/{y}.png".into(),
            quick_boot: false,
        };
        p.save(tmp.path()).unwrap();
        let back = Prefs::load(tmp.path());
        assert_eq!(back, p);
        assert_eq!(back.root_choice(), RootChoice::Full);
        assert_eq!(Prefs::default().tiles(), OSM_TILES);
    }

    #[test]
    fn claude_config_keeps_other_servers() {
        let merged = merge_claude_config(
            Some(r#"{"mcpServers":{"other":{"command":"x"}},"theme":"dark"}"#),
            "/Applications/Jidr.app/Contents/MacOS/jidr",
        )
        .unwrap();
        let v: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(v["mcpServers"]["jidr"]["args"][0], "mcp");
        assert_eq!(v["theme"], "dark");
        assert!(merge_claude_config(Some("[1]"), "jidr").is_err());
        assert!(merge_claude_config(None, "jidr").is_ok());
    }

    #[test]
    fn usage_rows_sort_and_total() {
        let rows = device_usage(
            &crate::fixtures::device_infos(),
            &[(uuid::Uuid::from_u128(1), 3)],
        );
        assert_eq!(rows[0].name, "Pixel-ish 6.1");
        assert_eq!(rows[0].sub, "3 snapshots");
        assert_eq!(rows.last().unwrap().size, "19.2 GB");
    }

    /// Gap B4: the daemon's settings replace the cached copy; unknown or
    /// missing values leave it alone.
    #[test]
    fn daemon_settings_update_the_cached_prefs() {
        let mut p = Prefs::default();
        p.apply_daemon_settings(&json!({ "default_root_mode": "magisk", "quick_boot": false }));
        assert_eq!(p.root_choice(), RootChoice::Full);
        assert!(!p.quick_boot);
        p.apply_daemon_settings(&json!({ "default_root_mode": "bogus" }));
        assert_eq!(p.default_root, "magisk");
        assert!(!p.quick_boot);
    }
}
