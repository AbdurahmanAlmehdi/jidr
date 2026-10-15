//! Compression formats found in ramdisks: gzip (RFC 1952), lz4 legacy frames
//! and xz. Each stream is decoded on its own, so concatenated streams can be
//! split and rebuilt with the same settings.

use std::io::{Read, Write};

use crate::{Error, Result};

/// Upper bound on one decompressed stream; real ramdisks are well under 100 MiB.
pub const MAX_DECOMPRESSED: usize = 1 << 30;

/// The lz4 legacy-frame magic (little-endian `0x184C2102`).
pub const LZ4_LEGACY_MAGIC: [u8; 4] = [0x02, 0x21, 0x4c, 0x18];
/// Uncompressed size of every lz4 legacy block except the last.
pub const LZ4_LEGACY_BLOCK: usize = 8 << 20;
/// AOSP builds ramdisks with `lz4 -l -12`.
pub const LZ4_ANDROID_LEVEL: i32 = 12;

const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];
const XZ_MAGIC: [u8; 6] = [0xfd, b'7', b'z', b'X', b'Z', 0x00];
/// Enough for any preset (`-9` needs about 65 MiB to decode).
const XZ_MEMLIMIT: u64 = 512 << 20;
/// What `gzip -n` writes on Unix: deflate, no flags, mtime 0, XFL 0, OS 3.
const GZIP_DEFAULT_HEADER: [u8; 10] = [0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];

/// Integrity check stored in an xz stream header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XzCheck {
    None,
    Crc32,
    Crc64,
    Sha256,
}

impl XzCheck {
    fn from_id(id: u8) -> Option<Self> {
        match id {
            0x00 => Some(XzCheck::None),
            0x01 => Some(XzCheck::Crc32),
            0x04 => Some(XzCheck::Crc64),
            0x0a => Some(XzCheck::Sha256),
            _ => None,
        }
    }

    fn to_lzma(self) -> liblzma::stream::Check {
        match self {
            XzCheck::None => liblzma::stream::Check::None,
            XzCheck::Crc32 => liblzma::stream::Check::Crc32,
            XzCheck::Crc64 => liblzma::stream::Check::Crc64,
            XzCheck::Sha256 => liblzma::stream::Check::Sha256,
        }
    }
}

/// How one stream is compressed, with the settings needed to reproduce it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compression {
    /// A raw cpio archive.
    None,
    /// gzip with zlib deflate at `level`; `header` is the original member
    /// header (magic through the optional fields), written back verbatim.
    Gzip { level: u32, header: Vec<u8> },
    /// lz4 legacy frame: 8 MiB blocks, each compressed with liblz4
    /// (HC when `level >= 3`).
    Lz4Legacy { level: i32 },
    /// xz with liblzma's preset `preset` and the given check.
    Xz { preset: u32, check: XzCheck },
}

/// One decoded stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decompressed {
    pub compression: Compression,
    pub data: Vec<u8>,
    /// Input bytes the stream occupied; the next stream starts here.
    pub consumed: usize,
}

impl Compression {
    /// `gzip -n -6`.
    pub fn gzip() -> Self {
        Compression::Gzip {
            level: 6,
            header: GZIP_DEFAULT_HEADER.to_vec(),
        }
    }

    /// `lz4 -l -12`, as AOSP builds emulator ramdisks.
    pub fn lz4_legacy() -> Self {
        Compression::Lz4Legacy {
            level: LZ4_ANDROID_LEVEL,
        }
    }

    /// `xz -9 --check=crc32`: the check the kernel's and other embedded xz
    /// decoders always support.
    pub fn xz() -> Self {
        Compression::Xz {
            preset: 9,
            check: XzCheck::Crc32,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Compression::None => "cpio",
            Compression::Gzip { .. } => "gzip",
            Compression::Lz4Legacy { .. } => "lz4-legacy",
            Compression::Xz { .. } => "xz",
        }
    }

    /// Identifies the stream at the start of `bytes` from its magic and
    /// header. Settings that a stream doesn't record (lz4 level, xz preset)
    /// get the Android defaults.
    pub fn detect(bytes: &[u8]) -> Result<Self> {
        if bytes.starts_with(&LZ4_LEGACY_MAGIC) {
            Ok(Compression::lz4_legacy())
        } else if bytes.starts_with(&GZIP_MAGIC) {
            let header_len = gzip_header_len(bytes)?;
            let level = match bytes.get(8) {
                Some(2) => 9,
                Some(4) => 1,
                _ => 6,
            };
            Ok(Compression::Gzip {
                level,
                header: bytes.get(..header_len).unwrap_or_default().to_vec(),
            })
        } else if bytes.starts_with(&XZ_MAGIC) {
            let check = bytes
                .get(7)
                .and_then(|id| XzCheck::from_id(id & 0x0f))
                .ok_or_else(|| corrupt("xz", "unknown integrity check in the stream header"))?;
            Ok(Compression::Xz { preset: 9, check })
        } else if bytes.starts_with(b"070701") || bytes.starts_with(b"070702") {
            Ok(Compression::None)
        } else {
            Err(Error::UnknownFormat { offset: 0 })
        }
    }

    pub fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
        match self {
            Compression::None => Ok(data.to_vec()),
            Compression::Gzip { level, header } => gzip_compress(data, *level, header),
            Compression::Lz4Legacy { level } => lz4_legacy_compress(data, *level),
            Compression::Xz { preset, check } => xz_compress(data, *preset, *check),
        }
    }
}

/// Decodes the single stream at the start of `bytes`.
pub fn decompress(bytes: &[u8]) -> Result<Decompressed> {
    let compression = Compression::detect(bytes)?;
    let (data, consumed) = match &compression {
        Compression::None => (bytes.to_vec(), bytes.len()),
        Compression::Gzip { header, .. } => gzip_decompress(bytes, header.len())?,
        Compression::Lz4Legacy { .. } => lz4_legacy_decompress(bytes)?,
        Compression::Xz { .. } => xz_decompress(bytes)?,
    };
    Ok(Decompressed {
        compression,
        data,
        consumed,
    })
}

fn corrupt(format: &'static str, message: impl ToString) -> Error {
    Error::Corrupt {
        format,
        message: message.to_string(),
    }
}

/// Reads `reader` to the end, failing past [`MAX_DECOMPRESSED`].
fn read_limited<R: Read>(reader: &mut R, format: &'static str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    reader
        .take(MAX_DECOMPRESSED as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| corrupt(format, e))?;
    if out.len() > MAX_DECOMPRESSED {
        return Err(Error::Limit {
            limit: MAX_DECOMPRESSED,
        });
    }
    Ok(out)
}

// ---- gzip (RFC 1952) -------------------------------------------------------

const FHCRC: u8 = 0x02;
const FEXTRA: u8 = 0x04;
const FNAME: u8 = 0x08;
const FCOMMENT: u8 = 0x10;

fn gzip_header_len(bytes: &[u8]) -> Result<usize> {
    let truncated = |offset| Error::Truncated {
        offset,
        what: "a gzip header",
    };
    let fixed = bytes.get(..10).ok_or(truncated(0))?;
    if fixed.get(2) != Some(&8) {
        return Err(corrupt("gzip", "compression method is not deflate"));
    }
    let flags = fixed.get(3).copied().unwrap_or(0);
    let mut off = 10;
    if flags & FEXTRA != 0 {
        let xlen = bytes.get(off..off + 2).ok_or(truncated(off))?;
        off += 2 + usize::from(u16::from_le_bytes([xlen[0], xlen[1]]));
    }
    for flag in [FNAME, FCOMMENT] {
        if flags & flag != 0 {
            let rest = bytes.get(off..).ok_or(truncated(off))?;
            let nul = rest.iter().position(|&b| b == 0).ok_or(truncated(off))?;
            off += nul + 1;
        }
    }
    if flags & FHCRC != 0 {
        off += 2;
    }
    if off > bytes.len() {
        return Err(truncated(off));
    }
    Ok(off)
}

fn gzip_decompress(bytes: &[u8], header_len: usize) -> Result<(Vec<u8>, usize)> {
    let body = bytes.get(header_len..).unwrap_or_default();
    let mut decoder = flate2::bufread::DeflateDecoder::new(body);
    let data = read_limited(&mut decoder, "gzip")?;
    let rest = decoder.into_inner();
    let trailer = rest.get(..8).ok_or(Error::Truncated {
        offset: bytes.len(),
        what: "the gzip CRC32 and size trailer",
    })?;
    let mut crc = flate2::Crc::new();
    crc.update(&data);
    let stored_crc = u32::from_le_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
    let stored_len = u32::from_le_bytes([trailer[4], trailer[5], trailer[6], trailer[7]]);
    if crc.sum() != stored_crc || (data.len() as u32) != stored_len {
        return Err(corrupt("gzip", "CRC32 or size trailer mismatch"));
    }
    let consumed = bytes.len() - rest.len() + 8;
    Ok((data, consumed))
}

fn gzip_compress(data: &[u8], level: u32, header: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() / 2 + header.len() + 8);
    out.extend_from_slice(header);
    let mut encoder = flate2::write::DeflateEncoder::new(out, flate2::Compression::new(level));
    encoder.write_all(data)?;
    let mut out = encoder.finish()?;
    let mut crc = flate2::Crc::new();
    crc.update(data);
    out.extend_from_slice(&crc.sum().to_le_bytes());
    // ISIZE is the input size modulo 2^32 (RFC 1952 §2.3.1).
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    Ok(out)
}

// ---- lz4 legacy frame ------------------------------------------------------
//
// A legacy frame has no end marker: it runs until the input ends, a block
// size of zero, or the next frame's magic. So an lz4 stream can only be
// followed by another lz4 stream, which is how AOSP concatenates ramdisks.

fn lz4_legacy_decompress(bytes: &[u8]) -> Result<(Vec<u8>, usize)> {
    let max_block = lz4::block::compress_bound(LZ4_LEGACY_BLOCK).map_err(|e| corrupt("lz4", e))?;
    let mut data = Vec::new();
    let mut off = LZ4_LEGACY_MAGIC.len();
    while let Some(size) = bytes.get(off..off + 4) {
        let size = u32::from_le_bytes([size[0], size[1], size[2], size[3]]) as usize;
        // The next stream's magic, or zero padding, ends this one.
        if size == 0 || size == u32::from_le_bytes(LZ4_LEGACY_MAGIC) as usize {
            break;
        }
        if size > max_block {
            return Err(corrupt(
                "lz4",
                format!("block at byte {off} is {size} bytes"),
            ));
        }
        let block = bytes.get(off + 4..off + 4 + size).ok_or(Error::Truncated {
            offset: off + 4,
            what: "an lz4 block",
        })?;
        let decoded = lz4::block::decompress(block, Some(LZ4_LEGACY_BLOCK as i32))
            .map_err(|e| corrupt("lz4", format!("block at byte {off}: {e}")))?;
        if data.len() + decoded.len() > MAX_DECOMPRESSED {
            return Err(Error::Limit {
                limit: MAX_DECOMPRESSED,
            });
        }
        data.extend_from_slice(&decoded);
        off += 4 + size;
    }
    // Fewer than 4 bytes left: zero padding ends the stream, anything else is cut off.
    let rest = bytes.get(off..).unwrap_or_default();
    if !rest.is_empty() && rest.len() < 4 && rest.iter().any(|&b| b != 0) {
        return Err(Error::Truncated {
            offset: off,
            what: "an lz4 block size",
        });
    }
    Ok((data, off))
}

fn lz4_legacy_compress(data: &[u8], level: i32) -> Result<Vec<u8>> {
    let mode = if level >= 3 {
        lz4::block::CompressionMode::HIGHCOMPRESSION(level)
    } else {
        lz4::block::CompressionMode::DEFAULT
    };
    let mut out = Vec::with_capacity(data.len() / 2 + 16);
    out.extend_from_slice(&LZ4_LEGACY_MAGIC);
    for chunk in data.chunks(LZ4_LEGACY_BLOCK) {
        let block = lz4::block::compress(chunk, Some(mode), false)?;
        out.extend_from_slice(&(block.len() as u32).to_le_bytes());
        out.extend_from_slice(&block);
    }
    Ok(out)
}

// ---- xz ----------------------------------------------------------------------

fn xz_decompress(bytes: &[u8]) -> Result<(Vec<u8>, usize)> {
    let stream = liblzma::stream::Stream::new_stream_decoder(XZ_MEMLIMIT, 0)
        .map_err(|e| corrupt("xz", e))?;
    let mut decoder = liblzma::bufread::XzDecoder::new_stream(bytes, stream);
    let data = read_limited(&mut decoder, "xz")?;
    let consumed = bytes.len() - decoder.into_inner().len();
    Ok((data, consumed))
}

fn xz_compress(data: &[u8], preset: u32, check: XzCheck) -> Result<Vec<u8>> {
    let stream = liblzma::stream::Stream::new_easy_encoder(preset, check.to_lzma())
        .map_err(|e| corrupt("xz", e))?;
    let mut encoder = liblzma::write::XzEncoder::new_stream(Vec::new(), stream);
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        (0..100_000u32)
            .flat_map(|i| (i % 251).to_le_bytes())
            .collect()
    }

    #[test]
    fn every_format_round_trips_and_reports_its_length() {
        let data = sample();
        for c in [
            Compression::gzip(),
            Compression::lz4_legacy(),
            Compression::Lz4Legacy { level: 1 },
            Compression::xz(),
        ] {
            let packed = c.compress(&data).unwrap();
            let mut followed = packed.clone();
            followed.extend_from_slice(&LZ4_LEGACY_MAGIC);
            let out = decompress(&followed).unwrap();
            assert_eq!(out.data, data, "{}", c.name());
            assert_eq!(out.consumed, packed.len(), "{}", c.name());
            assert_eq!(Compression::detect(&packed).unwrap().name(), c.name());
        }
    }

    #[test]
    fn lz4_legacy_splits_into_8_mib_blocks() {
        let data = vec![7u8; LZ4_LEGACY_BLOCK + 10];
        let packed = Compression::lz4_legacy().compress(&data).unwrap();
        let first = u32::from_le_bytes(packed[4..8].try_into().unwrap()) as usize;
        let second_at = 8 + first;
        assert!(packed.len() > second_at + 4);
        assert_eq!(decompress(&packed).unwrap().data.len(), data.len());
    }

    #[test]
    fn gzip_header_fields_are_kept() {
        // FNAME set: "ramdisk\0" follows the fixed header.
        let mut header = vec![0x1f, 0x8b, 8, FNAME, 1, 2, 3, 4, 2, 3];
        header.extend_from_slice(b"ramdisk\0");
        let c = Compression::Gzip { level: 9, header };
        let packed = c.compress(b"hello hello hello").unwrap();
        assert_eq!(Compression::detect(&packed).unwrap(), c);
        let out = decompress(&packed).unwrap();
        assert_eq!(out.data, b"hello hello hello");
        assert_eq!(out.consumed, packed.len());
    }

    #[test]
    fn xz_check_comes_from_the_stream_header() {
        let c = Compression::Xz {
            preset: 9,
            check: XzCheck::Crc64,
        };
        let packed = c.compress(b"data").unwrap();
        assert_eq!(Compression::detect(&packed).unwrap(), c);
    }

    #[test]
    fn corrupt_input_is_an_error_not_a_panic() {
        let data = sample();
        for c in [
            Compression::gzip(),
            Compression::lz4_legacy(),
            Compression::xz(),
        ] {
            let packed = c.compress(&data).unwrap();
            let truncated = &packed[..packed.len() / 2];
            assert!(decompress(truncated).is_err(), "{}", c.name());
            let mut flipped = packed.clone();
            let mid = flipped.len() / 2;
            flipped[mid] ^= 0xff;
            flipped[mid + 1] ^= 0xff;
            let _ = decompress(&flipped);
        }
        assert!(matches!(
            decompress(b"PK\x03\x04"),
            Err(Error::UnknownFormat { offset: 0 })
        ));
        assert!(decompress(&[0x1f, 0x8b, 8]).is_err());
        assert!(decompress(&[0x02, 0x21, 0x4c, 0x18, 0xff, 0xff, 0xff, 0x7f]).is_err());
    }
}
