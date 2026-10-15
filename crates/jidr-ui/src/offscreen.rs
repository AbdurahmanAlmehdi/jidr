//! Offscreen rendering for golden snapshots: Slint's software renderer on a
//! `MinimalSoftwareWindow`, so no display, GPU or event loop is needed.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{PlatformError, Rgba8Pixel, SharedPixelBuffer};

thread_local! {
    static LAST: RefCell<Option<Rc<MinimalSoftwareWindow>>> = const { RefCell::new(None) };
}

struct OffscreenPlatform;

impl Platform for OffscreenPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let w = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        LAST.with(|c| *c.borrow_mut() = Some(w.clone()));
        Ok(w)
    }
}

/// Installs the offscreen platform. Call once, before creating components.
pub fn init() -> Result<(), String> {
    slint::platform::set_platform(Box::new(OffscreenPlatform)).map_err(|e| e.to_string())
}

/// Renders `window` at `width`×`height` logical px (scale 1).
pub fn render(
    window: &slint::Window,
    width: u32,
    height: u32,
) -> Result<SharedPixelBuffer<Rgba8Pixel>, PlatformError> {
    window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
    window.set_size(slint::PhysicalSize::new(width, height));
    slint::platform::update_timers_and_animations();
    window.take_snapshot()
}

pub fn write_png(path: &Path, buf: &SharedPixelBuffer<Rgba8Pixel>) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), buf.width(), buf.height());
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::High);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(buf.as_bytes())
        .map_err(|e| e.to_string())
}

/// Reads an RGBA8 PNG as (width, height, pixels).
pub fn read_png(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut reader = dec.read_info().map_err(|e| e.to_string())?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| "PNG too large".to_string())?;
    let mut data = vec![0; size];
    let info = reader.next_frame(&mut data).map_err(|e| e.to_string())?;
    data.truncate(info.buffer_size());
    if info.color_type != png::ColorType::Rgba {
        return Err(format!("{}: not RGBA", path.display()));
    }
    Ok((info.width, info.height, data))
}

/// Result of comparing a render with its golden.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Diff {
    pub differing: usize,
    pub total: usize,
}

impl Diff {
    pub fn ratio(&self) -> f64 {
        self.differing as f64 / self.total.max(1) as f64
    }
}

/// Pixels whose channels differ by more than `tolerance`. Size mismatches
/// count every pixel.
pub fn diff(actual: &[u8], expected: &[u8], tolerance: u8) -> Diff {
    let total = actual.len().max(expected.len()) / 4;
    if actual.len() != expected.len() {
        return Diff {
            differing: total,
            total,
        };
    }
    let differing = actual
        .as_chunks::<4>()
        .0
        .iter()
        .zip(expected.as_chunks::<4>().0)
        .filter(|(a, e)| {
            a.iter()
                .zip(e.iter())
                .any(|(x, y)| x.abs_diff(*y) > tolerance)
        })
        .count();
    Diff { differing, total }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_counts_pixels_over_tolerance() {
        let a = [10, 10, 10, 255, 0, 0, 0, 255];
        let b = [12, 10, 10, 255, 0, 90, 0, 255];
        assert_eq!(
            diff(&a, &b, 4),
            Diff {
                differing: 1,
                total: 2
            }
        );
        assert_eq!(diff(&a, &b[..4], 4).differing, 2);
    }
}
