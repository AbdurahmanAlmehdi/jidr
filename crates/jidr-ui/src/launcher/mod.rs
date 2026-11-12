//! Launcher wiring: selection, tabs, lifecycle actions (hibernate, Save as
//! base, the RAM-budget refusal), the create/edit sheet, the command
//! palette and the confirmation dialog.

use std::rc::Rc;
use std::sync::Arc;

use jidr_core::{ErrorCode, HardwareProfile, JidrResult, SystemImage};
use serde_json::{Value, json};
use slint::{ComponentHandle, ModelRc, VecModel, Weak};
use uuid::Uuid;

use crate::app::{Ctx, report, toast};
use crate::bridge::Bridge;
use crate::device_form;
use crate::model::Exhausted;
use crate::{AppWindow, DeviceForm, PaletteItem, RootChoice};

fn models<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(v))
}

pub fn wire(ctx: &Rc<Ctx>) {
    let app = &ctx.app;
    let weak = app.as_weak();

    let c = ctx.clone();
    app.on_select_device(move |i| {
        let id = c.device_id(i);
        c.state.borrow_mut().selected = id;
        c.rerender();
        crate::snapshots::load(&c);
    });

    let c = ctx.clone();
    app.on_open_device(move |i| {
        if let Some(id) = c.device_id(i) {
            open_tab(&c, id);
        }
    });

    let c = ctx.clone();
    app.on_select_tab(move |i| {
        c.state.borrow_mut().active = usize::try_from(i).ok();
        c.rerender();
    });

    let c = ctx.clone();
    app.on_close_tab(move |i| close_tab(&c, usize::try_from(i).ok()));

    let c = ctx.clone();
    app.on_shortcut(move |s| match s.as_str() {
        p if crate::player::shortcut(&c, p) => {}
        "library" => {
            c.state.borrow_mut().active = None;
            c.rerender();
        }
        "close-tab" => {
            let active = c.state.borrow().active;
            close_tab(&c, active);
        }
        t => {
            if let Some(n) = t.strip_prefix("tab-").and_then(|n| n.parse::<usize>().ok()) {
                let len = c.state.borrow().tabs.len();
                if n >= 1 && n <= len {
                    c.state.borrow_mut().active = Some(n - 1);
                    c.rerender();
                }
            }
        }
    });

    let c = ctx.clone();
    app.on_start_device(move |i| {
        if let Some(id) = c.device_id(i) {
            start(&c, id);
        }
    });

    let c = ctx.clone();
    app.on_stop_device(move |i| {
        if let Some(id) = c.device_id(i) {
            stop(&c, id);
        }
    });

    let c = ctx.clone();
    app.on_hibernate_device(move |i| {
        if let Some(id) = c.device_id(i) {
            hibernate(&c, id);
        }
    });

    let c = ctx.clone();
    app.on_save_base(move |i| {
        if let Some(id) = c.device_id(i) {
            save_as_base(&c, id);
        }
    });

    let c = ctx.clone();
    app.on_exhausted_hibernate(move || {
        let Some((id, x)) = c.pending_exhausted.borrow_mut().take() else {
            return;
        };
        let picks = x.picks();
        let names: Vec<String> = x
            .hibernate_candidates
            .iter()
            .filter(|d| picks.contains(&d.id))
            .map(|d| d.name.clone())
            .collect();
        let w = c.app.as_weak();
        toast(&w, format!("Hibernating {} to make room", names.join(", ")));
        let bridge = c.bridge.clone();
        let b2 = bridge.clone();
        hibernate_all(&bridge, picks, move |r| match r {
            Ok(()) => start_device(&b2, &w, id, false),
            Err(e) => toast(&w, e.message),
        });
    });

    let c = ctx.clone();
    app.on_exhausted_force(move || {
        if let Some((id, _)) = c.pending_exhausted.borrow_mut().take() {
            start_device(&c.bridge, &c.app.as_weak(), id, true);
        }
    });

    let c = ctx.clone();
    app.on_exhausted_cancel(move || {
        c.pending_exhausted.borrow_mut().take();
    });

    let c = ctx.clone();
    let w = weak.clone();
    app.on_cancel_job(move |i| {
        let job = {
            let st = c.state.borrow();
            usize::try_from(i)
                .ok()
                .and_then(|i| st.data.jobs.get(i))
                .map(|j| j.id)
        };
        if let Some(job) = job {
            c.bridge.call(
                "jobs.cancel",
                json!({ "job": job }),
                report(&w, "Cancelled"),
            );
        }
    });

    let c = ctx.clone();
    let w = weak.clone();
    app.on_verify_root(move |i| {
        if let Some(id) = c.device_id(i) {
            c.bridge.run(
                "root.verify",
                json!({ "device": id }),
                |_| {},
                report(&w, "Root verified"),
            );
        }
    });

    let c = ctx.clone();
    app.on_revert_root(move |i| {
        let Some(id) = c.device_id(i) else { return };
        let name = c.info(id).map(|d| d.device.name).unwrap_or_default();
        let c2 = c.clone();
        confirm(
            &c,
            &format!("Revert {name} to unrooted?"),
            "Removes root and cold boots the device (about 40 s). Apps and data stay. Snapshots taken with root can't be restored until root is set up again.",
            "Revert",
            move || {
                let w = c2.app.as_weak();
                c2.bridge.run(
                    "root.set",
                    json!({ "device": id, "mode": "none" }),
                    |_| {},
                    report(&w, &format!("Reverted {name} to unrooted")),
                );
            },
        );
    });

    let c = ctx.clone();
    app.on_more_device(move |i| {
        // The palette, narrowed to this device, lists everything it can do.
        let name = c
            .device_id(i)
            .and_then(|id| c.info(id))
            .map(|d| d.device.name)
            .unwrap_or_default();
        open_palette(&c, &name);
    });

    let c = ctx.clone();
    app.on_new_device(move || new_device(&c));

    let c = ctx.clone();
    app.on_cancel_form(move || {
        c.app.set_sheet_open(false);
        c.catalog.borrow_mut().editing = None;
    });

    let c = ctx.clone();
    app.on_form_image_changed(move |i| {
        let mut form = c.app.get_form();
        let preferred = c.prefs.borrow().root_choice();
        let cat = c.catalog.borrow();
        if let Some(img) = usize::try_from(i).ok().and_then(|i| cat.images.get(i)) {
            if !img.image.allows_adb_root() && form.root == RootChoice::Shell {
                form.root = RootChoice::None;
            } else if img.image.allows_adb_root() && form.root == RootChoice::None {
                form.root = device_form::default_root(Some(img), preferred);
            }
        }
        form.image = i;
        c.app.set_form(form);
    });

    let c = ctx.clone();
    app.on_form_profile_changed(move |mut form, i| {
        if let Ok(i) = usize::try_from(i) {
            device_form::apply_profile(&mut form, &c.catalog.borrow().profiles, i);
        }
        form
    });

    let c = ctx.clone();
    app.on_form_lean(move |mut form| {
        if let Err(e) = device_form::apply_lean(&mut form, &c.catalog.borrow().profiles) {
            form.error = e.into();
        }
        form
    });

    let c = ctx.clone();
    app.on_submit_form(move |form| submit(&c, form));

    let c = ctx.clone();
    app.on_open_palette(move || open_palette(&c, ""));

    let c = ctx.clone();
    app.on_palette_query_changed(move |q| {
        let all = palette_items(&c);
        c.app
            .set_palette_items(models(crate::palette::filter(&all, &q)));
    });

    let c = ctx.clone();
    app.on_palette_run(move |item| run_palette(&c, &item));

    let c = ctx.clone();
    app.on_confirm_ok(move || {
        if let Some(f) = c.pending_confirm.borrow_mut().take() {
            f();
        }
    });

    let c = ctx.clone();
    app.on_open_settings(move || crate::settings::open(&c, 0));

    let c = ctx.clone();
    app.on_setup_sdk(move || crate::wizard::open(&c));
    let c = ctx.clone();
    app.on_use_existing_sdk(move || crate::wizard::open(&c));

    let w = weak.clone();
    app.on_drag_window(move || {
        if let Some(app) = w.upgrade() {
            use slint::winit_030::WinitWindowAccessor;
            app.window().with_winit_window(|win| {
                let _ = win.drag_window();
            });
        }
    });
}

pub fn open_tab(c: &Rc<Ctx>, id: Uuid) {
    {
        let mut st = c.state.borrow_mut();
        st.selected = Some(id);
        let pos = match st.tabs.iter().position(|t| *t == id) {
            Some(p) => p,
            None => {
                st.tabs.push(id);
                st.tabs.len() - 1
            }
        };
        st.active = Some(pos);
    }
    c.rerender();
}

fn close_tab(c: &Rc<Ctx>, i: Option<usize>) {
    {
        let mut st = c.state.borrow_mut();
        if let Some(i) = i
            && i < st.tabs.len()
        {
            st.tabs.remove(i);
        }
        st.active = None;
    }
    c.rerender();
}

/// Starts (or resumes) a device. A start past the RAM budget opens the
/// `resource_exhausted` dialog instead of a toast (D42).
pub fn start(c: &Rc<Ctx>, id: Uuid) {
    start_device(&c.bridge, &c.app.as_weak(), id, false);
}

/// `device.start` from any thread; `force` starts past the budget.
pub fn start_device(bridge: &Bridge, w: &Weak<AppWindow>, id: Uuid, force: bool) {
    let mut params = json!({ "device": id });
    if force {
        params["force"] = json!(true);
    }
    let w = w.clone();
    bridge.run(
        "device.start",
        params,
        |_| {},
        move |r| {
            if let Some(x) = exhausted(&r) {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(c) = crate::app::ctx() {
                        show_exhausted(&c, id, x);
                    }
                });
            } else {
                report(&w, "")(r);
            }
        },
    );
}

/// The details of a `resource_exhausted` refusal, if that's what `r` is.
pub fn exhausted(r: &JidrResult<Value>) -> Option<Exhausted> {
    match r {
        Err(e) if e.code == ErrorCode::ResourceExhausted => {
            e.data.clone().and_then(|d| serde_json::from_value(d).ok())
        }
        _ => None,
    }
}

/// Opens the "not enough RAM" dialog for a refused start of `id`.
pub fn show_exhausted(c: &Rc<Ctx>, id: Uuid, x: Exhausted) {
    let name = c.info(id).map(|d| d.device.name).unwrap_or_default();
    set_exhausted(&c.app, &name, &x);
    *c.pending_exhausted.borrow_mut() = Some((id, x));
    c.app.set_exhausted_open(true);
}

/// Fills the dialog's view model (also used by the goldens).
pub fn set_exhausted(app: &AppWindow, name: &str, x: &Exhausted) {
    let picks = x.picks();
    let first = x
        .hibernate_candidates
        .iter()
        .find(|d| picks.contains(&d.id))
        .map(|d| d.name.clone())
        .unwrap_or_default();
    app.set_exhausted_name(name.into());
    app.set_exhausted_need(crate::model::ram_gb(x.needed_mb).into());
    app.set_exhausted_free(crate::model::ram_gb(x.free_mb()).into());
    app.set_exhausted_budget(crate::model::ram_gb(x.budget_mb).into());
    app.set_exhausted_rows(models(x.rows()));
    app.set_exhausted_picks(picks.len() as i32);
    app.set_exhausted_first_pick(first.into());
}

/// Hibernates `ids` one after another, then calls `done`.
fn hibernate_all(
    bridge: &Arc<Bridge>,
    mut ids: Vec<Uuid>,
    done: impl FnOnce(JidrResult<()>) + Send + 'static,
) {
    if ids.is_empty() {
        return done(Ok(()));
    }
    let next = ids.remove(0);
    let b = bridge.clone();
    bridge.run(
        "device.hibernate",
        json!({ "device": next }),
        |_| {},
        move |r| match r {
            Ok(_) => hibernate_all(&b, ids, done),
            Err(e) => done(Err(e)),
        },
    );
}

/// `device.hibernate`: saves the whole state and frees its RAM.
pub fn hibernate(c: &Rc<Ctx>, id: Uuid) {
    let name = c.info(id).map(|d| d.device.name).unwrap_or_default();
    let w = c.app.as_weak();
    c.bridge.run(
        "device.hibernate",
        json!({ "device": id }),
        |_| {},
        report(&w, format!("Hibernated {name}")),
    );
}

/// `base.create` from a running device, after a confirmation.
pub fn save_as_base(c: &Rc<Ctx>, id: Uuid) {
    let Some(info) = c.info(id) else { return };
    let name = info.device.name.clone();
    let taken: Vec<String> = c
        .state
        .borrow()
        .data
        .devices
        .iter()
        .map(|d| d.device.name.clone())
        .collect();
    let base = device_form::free_name(&format!("{name} base"), &taken);
    let c2 = c.clone();
    confirm_with(
        c,
        &format!("Save {name} as a base?"),
        &format!(
            "Agents lease throwaway clones of a base with pool.get. {name} stops, and \"{base}\" becomes a read-only copy of it, animations off."
        ),
        "Save as base",
        false,
        move || {
            let w = c2.app.as_weak();
            c2.bridge.run(
                "base.create",
                json!({ "from_device": id, "name": base }),
                |_| {},
                report(&w, format!("Saved {base} as a base")),
            );
        },
    );
}

pub fn stop(c: &Rc<Ctx>, id: Uuid) {
    let w = c.app.as_weak();
    c.bridge
        .call("device.stop", json!({ "device": id }), report(&w, ""));
}

/// Shows the confirmation dialog; `ok` runs on confirm.
pub fn confirm(c: &Rc<Ctx>, title: &str, body: &str, action: &str, ok: impl Fn() + 'static) {
    confirm_with(c, title, body, action, true, ok);
}

/// [`confirm`] with a non-destructive (primary) action button.
pub fn confirm_with(
    c: &Rc<Ctx>,
    title: &str,
    body: &str,
    action: &str,
    destructive: bool,
    ok: impl Fn() + 'static,
) {
    *c.pending_confirm.borrow_mut() = Some(Box::new(ok));
    c.app.set_confirm_title(title.into());
    c.app.set_confirm_body(body.into());
    c.app.set_confirm_action(action.into());
    c.app.set_confirm_destructive(destructive);
    c.app.set_confirm_open(true);
}

/// Loads profiles and images, then calls `then` on the UI thread.
fn load_catalog(c: &Rc<Ctx>, then: impl FnOnce(&Rc<Ctx>) + 'static) {
    let bridge = c.bridge.clone();
    let b2 = bridge.clone();
    let c2 = c.clone();
    let token = crate::app::defer(
        move |(profiles, images): (Vec<HardwareProfile>, Vec<SystemImage>)| {
            {
                let mut cat = c2.catalog.borrow_mut();
                cat.profiles = profiles;
                cat.images = device_form::sorted_images(&images);
            }
            then(&c2);
        },
    );
    bridge.call("profile.list", json!({}), move |p| {
        let profiles: Vec<HardwareProfile> = p
            .ok()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        b2.call("image.list", json!({}), move |i| {
            let images: Vec<SystemImage> = i
                .ok()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            crate::app::fire(token, (profiles, images));
        });
    });
}

fn new_device(c: &Rc<Ctx>) {
    load_catalog(c, |c| {
        let taken: Vec<String> = c
            .state
            .borrow()
            .data
            .devices
            .iter()
            .map(|d| d.device.name.clone())
            .collect();
        let preferred = c.prefs.borrow().root_choice();
        let jobs = c.state.borrow().data.jobs.clone();
        let cat = c.catalog.borrow();
        let form = device_form::new_form(&cat.profiles, &cat.images, &taken, preferred);
        c.app
            .set_profiles(models(device_form::profile_options(&cat.profiles)));
        c.app
            .set_images(models(device_form::image_rows(&cat.images, &jobs)));
        c.app.set_form(form);
        c.app.set_sheet_open(true);
        drop(cat);
        c.catalog.borrow_mut().editing = None;
    });
}

pub fn edit_device(c: &Rc<Ctx>, id: Uuid) {
    load_catalog(c, move |c| {
        let Some(info) = c.info(id) else { return };
        let global = c.prefs.borrow().auto_hibernate_minutes;
        let hib = device_form::HibernateChoices::device(global, info.device.auto_hibernate_minutes);
        let cat = c.catalog.borrow();
        c.app
            .set_profiles(models(device_form::profile_options(&cat.profiles)));
        c.app
            .set_form(device_form::edit_form(&info, &cat.profiles, &hib));
        c.app.set_hibernate_options(models(hib.options.clone()));
        c.app.set_sheet_open(true);
        drop(cat);
        let mut cat = c.catalog.borrow_mut();
        cat.editing = Some(id);
        cat.hibernate = hib;
    });
}

fn set_form_error(c: &Rc<Ctx>, msg: String) {
    let mut f = c.app.get_form();
    f.error = msg.into();
    c.app.set_form(f);
}

fn submit(c: &Rc<Ctx>, form: DeviceForm) {
    let editing = c.catalog.borrow().editing;
    let w = c.app.as_weak();
    if let Some(id) = editing {
        let Some(info) = c.info(id) else { return };
        let params = device_form::edit_params(&form, &info, &c.catalog.borrow().hibernate);
        match params {
            Err(e) => set_form_error(c, e),
            Ok(params) => {
                c.app.set_sheet_open(false);
                // 9e: a changed root mode runs `root.set` (Full root opens
                // the Root sheet).
                let mode = crate::root::mode_of(form.root);
                let changed = mode != info.device.root.mode;
                let saved = report(&w, &format!("Saved {}", form.name));
                c.bridge.call("device.edit", params, move |r| {
                    let ok = r.is_ok();
                    saved(r);
                    if ok && changed {
                        crate::root::start_later(id, mode);
                    }
                });
            }
        }
        return;
    }
    let cat = c.catalog.borrow();
    let params = match device_form::create_params(&form, &cat.profiles, &cat.images) {
        Ok(p) => p,
        Err(e) => {
            drop(cat);
            return set_form_error(c, e);
        }
    };
    let image = usize::try_from(form.image)
        .ok()
        .and_then(|i| cat.images.get(i))
        .cloned();
    drop(cat);
    c.app.set_sheet_open(false);
    let bridge = c.bridge.clone();
    let start = form.start_after;
    let name = form.name.to_string();
    // 9e: Full root runs `root.set magisk` after create (it boots the
    // device and opens the Root sheet) instead of a plain start.
    let full_root = crate::root::mode_of(form.root) == jidr_core::RootMode::Magisk;
    // A missing image is downloaded first (a job, shown in the sidebar).
    let create = move |bridge: std::sync::Arc<crate::bridge::Bridge>,
                       w: slint::Weak<crate::AppWindow>| {
        let b2 = bridge.clone();
        bridge.call("device.create", params, move |r| match r {
            Ok(dev) => {
                toast(&w, format!("Created {name}"));
                if full_root
                    && let Some(id) = dev
                        .get("id")
                        .and_then(|v| v.as_str())
                        .and_then(|s| uuid::Uuid::parse_str(s).ok())
                {
                    crate::root::start_later(id, jidr_core::RootMode::Magisk);
                } else if start
                    && let Some(id) = dev
                        .get("id")
                        .and_then(|v| v.as_str())
                        .and_then(|s| uuid::Uuid::parse_str(s).ok())
                {
                    start_device(&b2, &w, id, false);
                }
            }
            Err(e) => toast(&w, e.message),
        });
    };
    match image {
        Some(img) if img.installed_in.is_none() => {
            let b = bridge.clone();
            let w2 = w.clone();
            bridge.run(
                "image.install",
                json!({ "image": img.image.package() }),
                |_| {},
                move |r| match r {
                    Ok(_) => create(b, w2),
                    Err(e) => toast(&w2, e.message),
                },
            );
        }
        _ => create(bridge, w),
    }
}

fn palette_items(c: &Rc<Ctx>) -> Vec<PaletteItem> {
    use slint::Model;
    let devices: Vec<_> = c.app.get_devices().iter().collect();
    crate::palette::items(&devices, c.app.get_sdk_ready())
}

fn open_palette(c: &Rc<Ctx>, query: &str) {
    let all = palette_items(c);
    c.app.set_palette_query(query.into());
    c.app
        .set_palette_items(models(crate::palette::filter(&all, query)));
    c.app.set_palette_open(true);
}

fn run_palette(c: &Rc<Ctx>, item: &PaletteItem) {
    let action = item.action.to_string();
    let (verb, arg) = action.split_once(':').unwrap_or((action.as_str(), ""));
    let id = Uuid::parse_str(arg).ok();
    match (verb, id) {
        ("open", Some(id)) => open_tab(c, id),
        ("start", Some(id)) => start(c, id),
        ("stop", Some(id)) => stop(c, id),
        ("hibernate", Some(id)) => hibernate(c, id),
        ("base", Some(id)) => save_as_base(c, id),
        ("edit", Some(id)) => edit_device(c, id),
        ("new", _) => new_device(c),
        ("setup", _) => crate::wizard::open(c),
        ("settings", _) => crate::settings::open(c, if arg == "agents" { 2 } else { 0 }),
        ("appearance", _) => {
            let dark = c.app.global::<crate::Theme>().get_dark();
            c.prefs.borrow_mut().appearance = if dark { 1 } else { 2 };
            let _ = c.prefs.borrow().save(&c.paths.data);
            c.apply_appearance();
        }
        _ => {}
    }
}

/// Exposed for tests: the params the sheet would send.
pub fn create_params_for(
    form: &DeviceForm,
    profiles: &[HardwareProfile],
    images: &[SystemImage],
) -> Result<Value, String> {
    device_form::create_params(form, profiles, images)
}
