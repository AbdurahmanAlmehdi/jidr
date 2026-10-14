//! Android SDK discovery and install, license consent, AVD lifecycle,
//! hardware profiles and download cache.

pub mod avd;
pub mod cache;
pub mod catalog;
pub mod host;
pub mod install;
pub mod license;
pub mod profiles;
pub mod repo;
pub mod sdk;

pub use avd::{AvdInfo, AvdManager, AvdSpec, ConfigIni};
pub use cache::{CacheEntry, DownloadCache, PruneReport};
pub use catalog::{CatalogFilter, image_catalog};
pub use install::{Fetcher, Phase, Progress, install};
pub use jidr_core::Cancel;
pub use license::{ConsentStore, PendingLicense};
pub use repo::{RemotePackage, Repository};
pub use sdk::{Sdk, SdkKind, SdkSet};
