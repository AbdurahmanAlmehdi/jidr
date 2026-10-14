//! Golden snapshots of every screen and state, in dark, light and RTL.
//!
//! ```text
//! cargo run -p jidr-ui --example snapshot -- --check     # compare with tests/golden
//! cargo run -p jidr-ui --example snapshot -- --update    # rewrite the goldens
//! cargo run -p jidr-ui --example snapshot -- --only launcher
//! ```
//! Renders with Slint's software renderer, so it runs headless and the same
//! everywhere. Failures write `<target>/snapshots/<name>.png` for review.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use jidr_ui::offscreen;
use jidr_ui::snapshots::{Variant, cases};

/// A channel may drift this much (font hinting, rounding)…
const TOLERANCE: u8 = 24;
/// …and this share of pixels may differ before a case fails.
const BUDGET: f64 = 0.002;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let update = args.iter().any(|a| a == "--update");
    let only = args
        .iter()
        .position(|a| a == "--only")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let out = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"))
        .join("snapshots");

    if let Err(e) = offscreen::init() {
        eprintln!("snapshot: {e}");
        return ExitCode::FAILURE;
    }
    let mut failed = Vec::new();
    let mut count = 0;
    for case in cases() {
        if only.as_deref().is_some_and(|o| !case.name.contains(o)) {
            continue;
        }
        for variant in Variant::ALL {
            let name = format!("{}.{}", case.name, variant.suffix());
            let buf = match (case.render)(variant) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("✗ {name}: render failed: {e}");
                    failed.push(name);
                    continue;
                }
            };
            count += 1;
            let path = golden.join(format!("{name}.png"));
            if update {
                if let Err(e) = offscreen::write_png(&path, &buf) {
                    eprintln!("✗ {name}: {e}");
                    failed.push(name);
                }
                continue;
            }
            let verdict = offscreen::read_png(&path).map(|(w, h, px)| {
                if (w, h) != (buf.width(), buf.height()) {
                    return Err(format!(
                        "size {}×{} vs golden {w}×{h}",
                        buf.width(),
                        buf.height()
                    ));
                }
                let d = offscreen::diff(buf.as_bytes(), &px, TOLERANCE);
                if d.ratio() > BUDGET {
                    Err(format!(
                        "{} of {} pixels differ ({:.2}%)",
                        d.differing,
                        d.total,
                        d.ratio() * 100.0
                    ))
                } else {
                    Ok(())
                }
            });
            match verdict {
                Ok(Ok(())) => println!("✓ {name}"),
                Ok(Err(why)) | Err(why) => {
                    let actual = out.join(format!("{name}.png"));
                    let _ = offscreen::write_png(&actual, &buf);
                    eprintln!("✗ {name}: {why}; actual: {}", actual.display());
                    failed.push(name);
                }
            }
        }
    }
    if update {
        println!("updated {count} goldens in {}", golden.display());
    }
    if failed.is_empty() {
        println!("snapshot: {count} renders ok");
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "snapshot: {} of {count} failed (rerun with --update after reviewing)",
            failed.len()
        );
        ExitCode::FAILURE
    }
}
