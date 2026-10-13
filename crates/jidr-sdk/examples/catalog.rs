//! Prints the image catalog and the emulator package for this host.
//! `cargo run -p jidr-sdk --example catalog` (set `JIDR_SDK_REPO` for a mirror).

use jidr_core::JidrPaths;
use jidr_sdk::{CatalogFilter, Fetcher, SdkSet, image_catalog, repo};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo = Fetcher::new()?.repository(&repo::repo_base()).await?;
    println!(
        "{} packages, {} licenses",
        repo.packages.len(),
        repo.licenses.len()
    );
    if let Some(emu) = repo.latest("emulator", 0) {
        let arc = emu.archive_for_host();
        println!(
            "emulator {} ({}), license {:?}, {:.0} MB",
            emu.revision,
            arc.map(|a| a.url.as_str())
                .unwrap_or("no archive for this host"),
            emu.license,
            arc.map(|a| a.size as f64 / 1e6).unwrap_or(0.0)
        );
    }
    let sdks = SdkSet::discover(&JidrPaths::discover()?);
    for img in image_catalog(&repo, &sdks, &CatalogFilter::default()) {
        println!(
            "API {:>2} {:<28} r{:<8} {:>6.0} MB  {:?}  {}",
            img.image.api,
            img.image.tag,
            img.revision,
            img.size as f64 / 1e6,
            img.support,
            img.installed_in
                .map(|p| format!("installed in {}", p.display()))
                .unwrap_or_default()
        );
    }
    Ok(())
}
