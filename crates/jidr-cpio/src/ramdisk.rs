use crate::compress::{self, Compression};
use crate::{Archive, Entry, Error, Result};

/// One compressed stream of a ramdisk image and the cpio archive inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub compression: Compression,
    pub archive: Archive,
}

impl Segment {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.compression.compress(&self.archive.to_bytes()?)
    }
}

/// A ramdisk image: concatenated compressed cpio streams, as the emulator
/// ships them (generic ramdisk first, then vendor fragments). The kernel
/// unpacks them in order into one rootfs, so later segments override
/// earlier ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ramdisk {
    pub segments: Vec<Segment>,
    /// Zero padding after the last stream, kept verbatim.
    pub trailing: Vec<u8>,
}

impl Ramdisk {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut segments = Vec::new();
        let mut off = 0;
        while let Some(rest) = bytes.get(off..).filter(|r| !r.is_empty()) {
            if !segments.is_empty() && rest.iter().all(|&b| b == 0) {
                return Ok(Ramdisk {
                    segments,
                    trailing: rest.to_vec(),
                });
            }
            let decoded = compress::decompress(rest).map_err(|e| at_offset(e, off))?;
            if decoded.consumed == 0 {
                return Err(Error::UnknownFormat { offset: off });
            }
            segments.push(Segment {
                compression: decoded.compression,
                archive: Archive::parse(&decoded.data)?,
            });
            off += decoded.consumed;
        }
        if segments.is_empty() {
            return Err(Error::Truncated {
                offset: 0,
                what: "a ramdisk stream",
            });
        }
        Ok(Ramdisk {
            segments,
            trailing: Vec::new(),
        })
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        for segment in &self.segments {
            out.extend_from_slice(&segment.to_bytes()?);
        }
        out.extend_from_slice(&self.trailing);
        Ok(out)
    }

    /// The entry the kernel ends up with for `name` (the last segment that
    /// has it wins), with the index of its segment.
    pub fn get(&self, name: &str) -> Option<(usize, &Entry)> {
        self.segments
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, s)| s.archive.get(name).map(|e| (i, e)))
    }
}

/// Makes stream-relative offsets in format errors absolute.
fn at_offset(error: Error, base: usize) -> Error {
    match error {
        Error::UnknownFormat { offset } => Error::UnknownFormat {
            offset: base + offset,
        },
        Error::Truncated { offset, what } => Error::Truncated {
            offset: base + offset,
            what,
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(names: &[&str]) -> Archive {
        let mut a = Archive::new();
        for name in names {
            a.insert(Entry::file(*name, 0o644, name.as_bytes().to_vec()));
        }
        a
    }

    #[test]
    fn concatenated_streams_split_and_rebuild() {
        let ramdisk = Ramdisk {
            segments: vec![
                Segment {
                    compression: Compression::gzip(),
                    archive: archive(&["extra"]),
                },
                Segment {
                    compression: Compression::lz4_legacy(),
                    archive: archive(&["init", "fstab"]),
                },
                Segment {
                    compression: Compression::lz4_legacy(),
                    archive: archive(&["fstab", "lib/modules"]),
                },
            ],
            trailing: vec![0; 12],
        };
        let bytes = ramdisk.to_bytes().unwrap();
        let parsed = Ramdisk::parse(&bytes).unwrap();
        assert_eq!(parsed, ramdisk);
        assert_eq!(parsed.get("fstab").unwrap().0, 2);
        assert_eq!(parsed.get("init").unwrap().0, 1);
        assert!(parsed.get("missing").is_none());
    }

    #[test]
    fn short_zero_padding_after_lz4_is_trailing_not_truncation() {
        // Regression: 1-3 zero bytes after an lz4 legacy stream were reported
        // as a truncated block size.
        for pad in 1..8 {
            let ramdisk = Ramdisk {
                segments: vec![Segment {
                    compression: Compression::lz4_legacy(),
                    archive: archive(&["init"]),
                }],
                trailing: vec![0; pad],
            };
            let bytes = ramdisk.to_bytes().unwrap();
            assert_eq!(Ramdisk::parse(&bytes).unwrap(), ramdisk, "pad {pad}");
        }
        let mut cut = Segment {
            compression: Compression::lz4_legacy(),
            archive: archive(&["init"]),
        }
        .to_bytes()
        .unwrap();
        cut.extend_from_slice(&[1, 0]);
        assert!(matches!(Ramdisk::parse(&cut), Err(Error::Truncated { .. })));
    }

    #[test]
    fn garbage_after_a_stream_is_an_error_with_its_offset() {
        // gzip members have an explicit end; lz4 legacy frames run to the next magic.
        let segment = Segment {
            compression: Compression::gzip(),
            archive: archive(&["init"]),
        };
        let mut bytes = segment.to_bytes().unwrap();
        let len = bytes.len();
        bytes.extend_from_slice(b"junkjunk");
        assert!(matches!(
            Ramdisk::parse(&bytes),
            Err(Error::UnknownFormat { offset }) if offset == len
        ));
        assert!(Ramdisk::parse(&[]).is_err());
        assert!(Ramdisk::parse(&[0; 16]).is_err());
    }
}
