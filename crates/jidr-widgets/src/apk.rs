//! Reads the package name from an APK's binary `AndroidManifest.xml`
//! (Android's documented binary XML chunk format: a string pool, then
//! start-element chunks with typed attributes). No tools from the SDK.

use std::io::Read;
use std::path::Path;

use jidr_core::{JidrError, JidrResult};

const RES_STRING_POOL: u16 = 0x0001;
const RES_XML: u16 = 0x0003;
const RES_XML_START_ELEMENT: u16 = 0x0102;
const UTF8_FLAG: u32 = 0x100;
/// `Res_value::TYPE_STRING`.
const TYPE_STRING: u8 = 0x03;

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    b.get(i..i + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn bad(what: &str) -> JidrError {
    JidrError::usage(format!("not a valid APK manifest: {what}"))
}

/// The string pool's strings.
fn strings(b: &[u8], at: usize) -> JidrResult<Vec<String>> {
    let header = u16_at(b, at + 2).ok_or_else(|| bad("string pool header"))? as usize;
    let count = u32_at(b, at + 8).ok_or_else(|| bad("string count"))? as usize;
    let flags = u32_at(b, at + 16).ok_or_else(|| bad("string flags"))?;
    let start = u32_at(b, at + 20).ok_or_else(|| bad("strings start"))? as usize;
    let utf8 = flags & UTF8_FLAG != 0;
    let mut out = Vec::with_capacity(count.min(4096));
    for i in 0..count {
        let off = u32_at(b, at + header + i * 4).ok_or_else(|| bad("string offset"))? as usize;
        let p = at + start + off;
        let s = if utf8 {
            // u8/u16 char count, then u8/u16 byte count, then bytes.
            let mut p = p;
            let skip_len = |p: &mut usize| -> Option<usize> {
                let first = *b.get(*p)? as usize;
                if first & 0x80 != 0 {
                    let second = *b.get(*p + 1)? as usize;
                    *p += 2;
                    Some(((first & 0x7f) << 8) | second)
                } else {
                    *p += 1;
                    Some(first)
                }
            };
            skip_len(&mut p).ok_or_else(|| bad("utf8 length"))?;
            let n = skip_len(&mut p).ok_or_else(|| bad("utf8 length"))?;
            String::from_utf8_lossy(b.get(p..p + n).ok_or_else(|| bad("utf8 string"))?).into_owned()
        } else {
            let mut n = u16_at(b, p).ok_or_else(|| bad("utf16 length"))? as usize;
            let mut p = p + 2;
            if n & 0x8000 != 0 {
                let low = u16_at(b, p).ok_or_else(|| bad("utf16 length"))? as usize;
                n = ((n & 0x7fff) << 16) | low;
                p += 2;
            }
            let units: Vec<u16> = (0..n)
                .map(|k| u16_at(b, p + k * 2).ok_or_else(|| bad("utf16 string")))
                .collect::<JidrResult<_>>()?;
            String::from_utf16_lossy(&units)
        };
        out.push(s);
    }
    Ok(out)
}

/// The `package` attribute of `<manifest>` in a binary manifest.
pub fn manifest_package(b: &[u8]) -> JidrResult<String> {
    if u16_at(b, 0) != Some(RES_XML) {
        return Err(bad("missing XML header"));
    }
    let mut at = u16_at(b, 2).ok_or_else(|| bad("header size"))? as usize;
    let mut pool: Vec<String> = Vec::new();
    while at + 8 <= b.len() {
        let kind = u16_at(b, at).ok_or_else(|| bad("chunk type"))?;
        let size = u32_at(b, at + 4).ok_or_else(|| bad("chunk size"))? as usize;
        if size < 8 {
            return Err(bad("chunk size"));
        }
        match kind {
            RES_STRING_POOL => pool = strings(b, at)?,
            RES_XML_START_ELEMENT => {
                let header = u16_at(b, at + 2).ok_or_else(|| bad("element header"))? as usize;
                let name = u32_at(b, at + 20).ok_or_else(|| bad("element name"))? as usize;
                if pool.get(name).map(String::as_str) == Some("manifest") {
                    let attr_start = u16_at(b, at + 24).ok_or_else(|| bad("attributes"))? as usize;
                    let attr_size = u16_at(b, at + 26).ok_or_else(|| bad("attributes"))? as usize;
                    let count = u16_at(b, at + 28).ok_or_else(|| bad("attributes"))? as usize;
                    let base = at + header.min(16) + attr_start;
                    for k in 0..count {
                        let a = base + k * attr_size.max(20);
                        let aname = u32_at(b, a + 4).ok_or_else(|| bad("attribute"))? as usize;
                        if pool.get(aname).map(String::as_str) != Some("package") {
                            continue;
                        }
                        let raw = u32_at(b, a + 8).ok_or_else(|| bad("attribute"))?;
                        let dtype = *b.get(a + 15).ok_or_else(|| bad("attribute"))?;
                        let data = u32_at(b, a + 16).ok_or_else(|| bad("attribute"))?;
                        let idx = if raw != u32::MAX {
                            raw
                        } else if dtype == TYPE_STRING {
                            data
                        } else {
                            return Err(bad("package isn't a string"));
                        };
                        return pool
                            .get(idx as usize)
                            .cloned()
                            .ok_or_else(|| bad("package string"));
                    }
                    return Err(bad("<manifest> has no package"));
                }
            }
            _ => {}
        }
        at += size;
    }
    Err(bad("no <manifest> element"))
}

/// Package name of an APK file.
pub fn package_name(apk: &Path) -> JidrResult<String> {
    let file = std::fs::File::open(apk)
        .map_err(|e| JidrError::usage(format!("cannot open {}: {e}", apk.display())))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| JidrError::usage(format!("{} isn't an APK: {e}", apk.display())))?;
    let mut entry = zip
        .by_name("AndroidManifest.xml")
        .map_err(|_| JidrError::usage(format!("{} has no AndroidManifest.xml", apk.display())))?;
    let mut bytes = Vec::new();
    entry
        .read_to_end(&mut bytes)
        .map_err(|e| JidrError::usage(format!("reading the manifest: {e}")))?;
    manifest_package(&bytes)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a minimal binary manifest: `<manifest package="…">`.
    pub(crate) fn tiny_manifest(package: &str, utf8: bool) -> Vec<u8> {
        let strs = ["manifest", "package", package];
        // String pool.
        let mut data = Vec::new();
        let mut offsets = Vec::new();
        for s in strs {
            offsets.push(data.len() as u32);
            if utf8 {
                data.push(s.chars().count() as u8);
                data.push(s.len() as u8);
                data.extend_from_slice(s.as_bytes());
                data.push(0);
            } else {
                let units: Vec<u16> = s.encode_utf16().collect();
                data.extend_from_slice(&(units.len() as u16).to_le_bytes());
                for u in units {
                    data.extend_from_slice(&u.to_le_bytes());
                }
                data.extend_from_slice(&0u16.to_le_bytes());
            }
        }
        while data.len() % 4 != 0 {
            data.push(0);
        }
        let header = 28u32;
        let start = header + 4 * strs.len() as u32;
        let mut pool = Vec::new();
        pool.extend_from_slice(&RES_STRING_POOL.to_le_bytes());
        pool.extend_from_slice(&(header as u16).to_le_bytes());
        pool.extend_from_slice(&(start + data.len() as u32).to_le_bytes());
        pool.extend_from_slice(&(strs.len() as u32).to_le_bytes());
        pool.extend_from_slice(&0u32.to_le_bytes());
        pool.extend_from_slice(&(if utf8 { UTF8_FLAG } else { 0 }).to_le_bytes());
        pool.extend_from_slice(&start.to_le_bytes());
        pool.extend_from_slice(&0u32.to_le_bytes());
        for o in offsets {
            pool.extend_from_slice(&o.to_le_bytes());
        }
        pool.extend_from_slice(&data);
        // <manifest> start element with one attribute (raw value = string 2).
        let mut el = Vec::new();
        el.extend_from_slice(&RES_XML_START_ELEMENT.to_le_bytes());
        el.extend_from_slice(&16u16.to_le_bytes());
        el.extend_from_slice(&(36u32 + 20).to_le_bytes());
        el.extend_from_slice(&1u32.to_le_bytes()); // line
        el.extend_from_slice(&u32::MAX.to_le_bytes()); // comment
        el.extend_from_slice(&u32::MAX.to_le_bytes()); // ns
        el.extend_from_slice(&0u32.to_le_bytes()); // name = "manifest"
        el.extend_from_slice(&20u16.to_le_bytes()); // attributeStart
        el.extend_from_slice(&20u16.to_le_bytes()); // attributeSize
        el.extend_from_slice(&1u16.to_le_bytes()); // count
        el.extend_from_slice(&[0; 6]); // id/class/style
        el.extend_from_slice(&u32::MAX.to_le_bytes()); // attr ns
        el.extend_from_slice(&1u32.to_le_bytes()); // attr name = "package"
        el.extend_from_slice(&2u32.to_le_bytes()); // raw value
        el.extend_from_slice(&8u16.to_le_bytes());
        el.push(0);
        el.push(TYPE_STRING);
        el.extend_from_slice(&2u32.to_le_bytes());
        let mut out = Vec::new();
        out.extend_from_slice(&RES_XML.to_le_bytes());
        out.extend_from_slice(&8u16.to_le_bytes());
        out.extend_from_slice(&((8 + pool.len() + el.len()) as u32).to_le_bytes());
        out.extend_from_slice(&pool);
        out.extend_from_slice(&el);
        out
    }

    #[test]
    fn reads_package_from_utf16_and_utf8_pools() {
        assert_eq!(
            manifest_package(&tiny_manifest("com.example.shop", false)).unwrap(),
            "com.example.shop"
        );
        assert_eq!(
            manifest_package(&tiny_manifest("ly.jidr.hello", true)).unwrap(),
            "ly.jidr.hello"
        );
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        assert!(manifest_package(b"").is_err());
        assert!(manifest_package(&[3, 0, 8, 0, 255, 255, 255, 255]).is_err());
        let mut m = tiny_manifest("a.b", false);
        m.truncate(m.len() - 10);
        assert!(manifest_package(&m).is_err());
    }
}
