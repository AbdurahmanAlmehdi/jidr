//! GPS panel: OpenStreetMap tiles around the fix (D14: configurable URL,
//! attribution always shown, tiles cached on disk), map clicks, and route
//! replay through the `gps.route` job.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde_json::{Value, json};
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use crate::app::{Ctx, toast};

pub const ZOOM: u32 = 15;
/// Map image size in pixels (the panel shows it at ~292×150 logical).
pub const MAP_W: u32 = 584;
pub const MAP_H: u32 = 300;
const TILE: f64 = 256.0;
pub const SPEEDS: [f32; 3] = [1.0, 2.0, 4.0];

thread_local! {
    /// The last route file, for Play after it ends.
    pub static ROUTE_FILE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// World pixel of a coordinate at `zoom` (Web Mercator).
pub fn world_px(lat: f64, lon: f64, zoom: u32) -> (f64, f64) {
    let n = TILE * f64::from(1u32 << zoom);
    let lat = lat.clamp(-85.0511, 85.0511).to_radians();
    let x = (lon + 180.0) / 360.0 * n;
    let y = (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * n;
    (x, y)
}

/// Inverse of [`world_px`].
pub fn lat_lon(x: f64, y: f64, zoom: u32) -> (f64, f64) {
    let n = TILE * f64::from(1u32 << zoom);
    let lon = x / n * 360.0 - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * y / n))
        .sinh()
        .atan()
        .to_degrees();
    (lat, lon)
}

/// The tile URL for `{z}/{x}/{y}`.
pub fn tile_url(template: &str, z: u32, x: u32, y: u32) -> String {
    template
        .replace("{z}", &z.to_string())
        .replace("{x}", &x.to_string())
        .replace("{y}", &y.to_string())
}

/// Tiles (x, y) needed for a `MAP_W × MAP_H` view centred on `center`.
pub fn tiles_for(center: (f64, f64)) -> Vec<(u32, u32)> {
    let (x0, y0) = (
        center.0 - f64::from(MAP_W) / 2.0,
        center.1 - f64::from(MAP_H) / 2.0,
    );
    let (x1, y1) = (x0 + f64::from(MAP_W), y0 + f64::from(MAP_H));
    let mut v = Vec::new();
    for ty in (y0 / TILE).floor() as i64..=(y1 / TILE).floor() as i64 {
        for tx in (x0 / TILE).floor() as i64..=(x1 / TILE).floor() as i64 {
            if tx >= 0 && ty >= 0 {
                v.push((tx as u32, ty as u32));
            }
        }
    }
    v
}

/// The route as SVG path commands in 0..1000 map units around `center`.
pub fn route_path(points: &[(f64, f64)], center: (f64, f64)) -> String {
    let mut s = String::new();
    for (i, (lat, lon)) in points.iter().enumerate() {
        let (x, y) = world_px(*lat, *lon, ZOOM);
        let nx = (x - (center.0 - f64::from(MAP_W) / 2.0)) / f64::from(MAP_W) * 1000.0;
        let ny = (y - (center.1 - f64::from(MAP_H) / 2.0)) / f64::from(MAP_H) * 1000.0;
        s.push_str(&format!(
            "{}{nx:.1} {ny:.1} ",
            if i == 0 { "M " } else { "L " }
        ));
    }
    s.trim_end().to_string()
}

fn decode_png(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|[g, a]| [*g, *g, *g, *a])
            .collect(),
        _ => return None,
    };
    Some((info.width, info.height, rgba))
}

async fn fetch_tile(client: &reqwest::Client, cache: &Path, url: &str) -> Option<Vec<u8>> {
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        url.hash(&mut h);
        format!("{:016x}.png", h.finish())
    };
    let path = cache.join(key);
    if let Ok(b) = tokio::fs::read(&path).await {
        return Some(b);
    }
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().await.ok()?.to_vec();
    let _ = tokio::fs::create_dir_all(cache).await;
    let _ = tokio::fs::write(&path, &bytes).await;
    Some(bytes)
}

/// Fetches and composes the map around `(lat, lon)`; `None` offline.
pub async fn render_map(
    template: String,
    cache: PathBuf,
    lat: f64,
    lon: f64,
) -> Option<(u32, u32, Vec<u8>)> {
    let client = reqwest::Client::builder()
        .user_agent(concat!(
            "Jidr/",
            env!("CARGO_PKG_VERSION"),
            " (+https://jidr.ly)"
        ))
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .ok()?;
    let center = world_px(lat, lon, ZOOM);
    let (ox, oy) = (
        center.0 - f64::from(MAP_W) / 2.0,
        center.1 - f64::from(MAP_H) / 2.0,
    );
    let mut out = vec![0u8; (MAP_W * MAP_H * 4) as usize];
    let mut any = false;
    for (tx, ty) in tiles_for(center) {
        let url = tile_url(&template, ZOOM, tx, ty);
        let Some((w, h, px)) = fetch_tile(&client, &cache, &url)
            .await
            .and_then(|b| decode_png(&b))
        else {
            continue;
        };
        any = true;
        let (bx, by) = (
            (f64::from(tx) * TILE - ox).round() as i64,
            (f64::from(ty) * TILE - oy).round() as i64,
        );
        for row in 0..h as i64 {
            let dy = by + row;
            if dy < 0 || dy >= i64::from(MAP_H) {
                continue;
            }
            for col in 0..w as i64 {
                let dx = bx + col;
                if dx < 0 || dx >= i64::from(MAP_W) {
                    continue;
                }
                let s = ((row * i64::from(w) + col) * 4) as usize;
                let d = ((dy * i64::from(MAP_W) + dx) * 4) as usize;
                if let (Some(src), Some(dst)) = (px.get(s..s + 4), out.get_mut(d..d + 4)) {
                    dst.copy_from_slice(src);
                }
            }
        }
    }
    any.then_some((MAP_W, MAP_H, out))
}

fn parse(v: &str) -> Option<f64> {
    v.trim().parse().ok()
}

/// Reloads the map around the fields' coordinates.
pub fn refresh_map(ctx: &Rc<Ctx>) {
    let vm = ctx.app.get_widgets();
    let (Some(lat), Some(lon)) = (parse(&vm.gps_lat), parse(&vm.gps_lon)) else {
        return;
    };
    let template = ctx.prefs.borrow().tiles().to_string();
    let cache = ctx.paths.cache.join("tiles");
    let weak = ctx.app.as_weak();
    ctx.bridge.handle().spawn(async move {
        let map = render_map(template, cache, lat, lon).await;
        let _ = slint::invoke_from_event_loop(move || {
            let Some(app) = weak.upgrade() else { return };
            match map {
                Some((w, h, px)) => {
                    let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&px, w, h);
                    app.set_gps_map(Image::from_rgba8(buf));
                    app.set_gps_has_map(true);
                }
                None => app.set_gps_has_map(false),
            }
            app.set_gps_marker_x(0.5);
            app.set_gps_marker_y(0.5);
        });
    });
}

/// `gps.set` from the fields.
pub fn set_location(ctx: &Rc<Ctx>, device: uuid::Uuid) {
    let vm = ctx.app.get_widgets();
    let (Some(lat), Some(lon)) = (parse(&vm.gps_lat), parse(&vm.gps_lon)) else {
        toast(
            &ctx.app.as_weak(),
            "Latitude and longitude must be numbers".into(),
        );
        return;
    };
    let mut p = json!({ "device": device, "latitude": lat, "longitude": lon });
    if let Some(a) = parse(&vm.gps_alt) {
        p["altitude"] = json!(a);
    }
    if let Some(b) = parse(&vm.gps_bearing) {
        p["bearing"] = json!(b);
    }
    let w = ctx.app.as_weak();
    ctx.bridge.call("gps.set", p, move |r| {
        if let Err(e) = r {
            toast(&w, e.message);
        }
    });
    refresh_map(ctx);
}

/// A click on the map moves the fix there.
pub fn map_click(ctx: &Rc<Ctx>, device: uuid::Uuid, nx: f32, ny: f32) {
    let mut vm = ctx.app.get_widgets();
    let (Some(lat), Some(lon)) = (parse(&vm.gps_lat), parse(&vm.gps_lon)) else {
        return;
    };
    let c = world_px(lat, lon, ZOOM);
    let x = c.0 + (f64::from(nx) - 0.5) * f64::from(MAP_W);
    let y = c.1 + (f64::from(ny) - 0.5) * f64::from(MAP_H);
    let (la, lo) = lat_lon(x, y, ZOOM);
    vm.gps_lat = format!("{la:.4}").into();
    vm.gps_lon = format!("{lo:.4}").into();
    ctx.app.set_widgets(vm);
    set_location(ctx, device);
}

/// Asks for a GPX/KML file with the macOS file chooser.
pub fn choose_file() -> Option<PathBuf> {
    let out = std::process::Command::new("osascript")
        .args([
            "-e",
            "POSIX path of (choose file with prompt \"Choose a GPX or KML route\" of type {\"gpx\", \"kml\", \"public.xml\", \"com.topografix.gpx\"})",
        ])
        .output()
        .ok()?;
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!p.is_empty()).then(|| PathBuf::from(p))
}

/// Starts replaying `path`; progress lands in the view model.
pub fn start_route(ctx: &Rc<Ctx>, device: uuid::Uuid, path: PathBuf) {
    let mut vm = ctx.app.get_widgets();
    vm.route_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .into();
    vm.route_active = true;
    vm.route_paused = false;
    vm.route_progress = 0.0;
    vm.route_text = "Starting…".into();
    let speed = SPEEDS
        .get(usize::try_from(vm.route_speed).unwrap_or(0))
        .copied()
        .unwrap_or(1.0);
    let repeat = vm.route_loop;
    ctx.app.set_widgets(vm);
    ROUTE_FILE.with(|f| *f.borrow_mut() = Some(path.clone()));
    // Draw the route on the map.
    if let Ok(text) = std::fs::read_to_string(&path) {
        let pts = route_points(&text);
        if let Some(first) = pts.first() {
            let mut vm = ctx.app.get_widgets();
            if parse(&vm.gps_lat).is_none() {
                vm.gps_lat = format!("{:.4}", first.0).into();
                vm.gps_lon = format!("{:.4}", first.1).into();
                ctx.app.set_widgets(vm);
            }
            let vm = ctx.app.get_widgets();
            let c = world_px(
                parse(&vm.gps_lat).unwrap_or(first.0),
                parse(&vm.gps_lon).unwrap_or(first.1),
                ZOOM,
            );
            ctx.app.set_gps_route_path(route_path(&pts, c).into());
            refresh_map(ctx);
        }
    }
    let weak = ctx.app.as_weak();
    let w2 = weak.clone();
    ctx.bridge.run(
        "gps.route",
        json!({ "device": device, "path": path, "speed": speed, "loop": repeat }),
        move |job| {
            let (p, msg) = (job.progress.unwrap_or(0.0), job.message.clone());
            let weak = weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = weak.upgrade() {
                    let mut vm = app.get_widgets();
                    vm.route_progress = p;
                    vm.route_text = msg.into();
                    app.set_widgets(vm);
                }
            });
        },
        move |r: jidr_core::JidrResult<Value>| {
            let err = r.err().filter(|e| !jidr_core::cancel::is_cancelled(e));
            let failed = err.is_some();
            let weak = w2.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = weak.upgrade() {
                    let mut vm = app.get_widgets();
                    vm.route_active = false;
                    vm.route_paused = false;
                    if !failed {
                        vm.route_progress = 1.0;
                    }
                    app.set_widgets(vm);
                }
            });
            if let Some(e) = err {
                toast(&w2, e.message);
            }
        },
    );
}

/// (lat, lon) of a GPX/KML text, for drawing (the daemon parses it again).
pub fn route_points(text: &str) -> Vec<(f64, f64)> {
    let mut v = Vec::new();
    let mut rest = text;
    // GPX: lat="…" lon="…" on trkpt/rtept/wpt.
    while let Some(i) = rest.find("lat=") {
        let after = &rest[i..];
        let num = |s: &str, key: &str| -> Option<f64> {
            let j = s.find(key)? + key.len() + 1;
            let q = s[j - 1..].chars().next()?;
            let end = s[j..].find(q)?;
            s[j..j + end].parse().ok()
        };
        let head = &after[..after.find('>').unwrap_or(after.len())];
        if let (Some(la), Some(lo)) = (num(head, "lat="), num(head, "lon=")) {
            v.push((la, lo));
        }
        rest = &after[4..];
    }
    if v.is_empty() {
        // KML: lon,lat[,alt] tuples inside <coordinates>.
        if let Some(a) = text.find("<coordinates>") {
            let body = &text[a + 13..];
            let body = &body[..body.find("</coordinates>").unwrap_or(body.len())];
            for t in body.split_whitespace() {
                let p: Vec<f64> = t.split(',').filter_map(|x| x.parse().ok()).collect();
                if p.len() >= 2 {
                    v.push((p[1], p[0]));
                }
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mercator_round_trips() {
        let (x, y) = world_px(32.8954, 13.1802, ZOOM);
        let (la, lo) = lat_lon(x, y, ZOOM);
        assert!((la - 32.8954).abs() < 1e-9 && (lo - 13.1802).abs() < 1e-9);
        // Tile of Martyrs' Square at z15.
        assert_eq!(((x / 256.0) as u32, (y / 256.0) as u32), (17583, 13210));
    }

    #[test]
    fn view_needs_the_tiles_around_the_point() {
        let c = world_px(32.8954, 13.1802, ZOOM);
        let t = tiles_for(c);
        assert!(t.len() >= 6 && t.len() <= 12, "{t:?}");
        assert!(t.contains(&((c.0 / 256.0) as u32, (c.1 / 256.0) as u32)));
        assert_eq!(
            tile_url("https://t/{z}/{x}/{y}.png", 15, 1, 2),
            "https://t/15/1/2.png"
        );
    }

    #[test]
    fn route_path_starts_at_the_centre() {
        let c = world_px(32.8954, 13.1802, ZOOM);
        let p = route_path(&[(32.8954, 13.1802), (32.8900, 13.1850)], c);
        assert!(p.starts_with("M 500.0 500.0 L "), "{p}");
        let pts = route_points(
            "<gpx><trkpt lat=\"32.8954\" lon=\"13.1802\"/><trkpt lat='32.89' lon='13.18'/></gpx>",
        );
        assert_eq!(pts, vec![(32.8954, 13.1802), (32.89, 13.18)]);
        assert_eq!(
            route_points("<kml><coordinates>13.18,32.89,0 13.2,32.8</coordinates></kml>").len(),
            2
        );
    }
}
