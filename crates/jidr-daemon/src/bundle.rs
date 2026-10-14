//! `device.logs`: one zip with everything needed to debug a device.

use std::io::Write;
use std::path::{Path, PathBuf};

use jidr_core::{Device, ErrorCode, JidrResult, ResultExt};
use zip::write::SimpleFileOptions;

/// Writes `out` with the device record, its config, emulator and crash logs
/// and (when running) a logcat dump. Returns the number of files added.
pub fn write_bundle(
    out: &Path,
    device: &Device,
    avd_dir: &Path,
    log_dir: &Path,
    logcat: Option<&str>,
) -> JidrResult<usize> {
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)
            .ctx(ErrorCode::Usage, format!("creating {}", dir.display()))?;
    }
    let file =
        std::fs::File::create(out).ctx(ErrorCode::Usage, format!("creating {}", out.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut n = 0;
    let mut add =
        |zip: &mut zip::ZipWriter<std::fs::File>, name: &str, bytes: &[u8]| -> JidrResult<()> {
            zip.start_file(name, opts)
                .ctx(ErrorCode::Unexpected, "writing zip")?;
            zip.write_all(bytes)
                .ctx(ErrorCode::Unexpected, "writing zip")?;
            n += 1;
            Ok(())
        };
    let record = serde_json::to_vec_pretty(device).ctx(ErrorCode::Unexpected, "encoding device")?;
    add(&mut zip, "device.json", &record)?;
    for name in ["config.ini", "hardware-qemu.ini", "emu-launch-params.txt"] {
        if let Ok(bytes) = std::fs::read(avd_dir.join(name)) {
            add(&mut zip, &format!("avd/{name}"), &bytes)?;
        }
    }
    let mut logs: Vec<PathBuf> = std::fs::read_dir(log_dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect()
        })
        .unwrap_or_default();
    logs.sort();
    for p in logs {
        if let (Some(name), Ok(bytes)) = (p.file_name().and_then(|n| n.to_str()), std::fs::read(&p))
        {
            add(&mut zip, &format!("logs/{name}"), &bytes)?;
        }
    }
    if let Some(text) = logcat {
        add(&mut zip, "logcat.txt", text.as_bytes())?;
    }
    let about = format!(
        "jidr {}\nos {} {}\ncreated {}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        chrono::Utc::now()
    );
    add(&mut zip, "about.txt", about.as_bytes())?;
    zip.finish().ctx(ErrorCode::Unexpected, "finishing zip")?;
    Ok(n)
}
