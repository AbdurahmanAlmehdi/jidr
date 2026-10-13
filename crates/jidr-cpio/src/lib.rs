//! Ramdisk cpio (newc) reader/writer with gzip, lz4-legacy and xz support.
//!
//! Written clean-room from the public format descriptions: the newc cpio
//! header (`man 5 cpio`, the Linux `initramfs` buffer format), RFC 1952 for
//! gzip, the lz4 "legacy frame" in the lz4 frame-format document and the
//! `.xz` file format specification.
//!
//! An Android emulator `ramdisk.img` is several compressed streams
//! concatenated (a generic ramdisk, then vendor fragments), each holding one
//! cpio archive. [`Ramdisk`] splits and rebuilds that; [`Archive`] edits one
//! archive. An unmodified image writes back byte-identical, because every
//! header field, the trailer entry and the padding after it are kept, and
//! compression uses the reference implementations (liblz4 HC, zlib).
//!
//! ```no_run
//! # fn main() -> Result<(), jidr_cpio::Error> {
//! use jidr_cpio::{Entry, Ramdisk};
//!
//! let pristine = std::fs::read("ramdisk.img")?;
//! let mut ramdisk = Ramdisk::parse(&pristine)?;
//! if let Some(first) = ramdisk.segments.first_mut() {
//!     first.archive.insert(Entry::dir("overlay.d", 0o750));
//! }
//! std::fs::write("ramdisk.patched.img", ramdisk.to_bytes()?)?;
//! # Ok(())
//! # }
//! ```

pub mod compress;
pub mod cpio;
mod error;
mod ramdisk;

pub use compress::Compression;
pub use cpio::{Archive, Entry, Format};
pub use error::{Error, Result};
pub use ramdisk::{Ramdisk, Segment};
