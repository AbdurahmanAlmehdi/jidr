//! newc cpio archives (`070701`, and `070702` with checksums).
//!
//! Layout, per entry: a 110-byte ASCII header (6-byte magic + 13 fields of 8
//! hex digits), the NUL-terminated name, padding to a 4-byte boundary, the
//! data, padding to a 4-byte boundary. The archive ends with an entry named
//! `TRAILER!!!`; anything after it (usually zero padding) is kept as-is.

use std::io::Write;

use crate::{Error, Result};

pub const S_IFMT: u32 = 0o170_000;
pub const S_IFDIR: u32 = 0o040_000;
pub const S_IFREG: u32 = 0o100_000;
pub const S_IFLNK: u32 = 0o120_000;

const TRAILER: &str = "TRAILER!!!";
const HEADER_LEN: usize = 110;
const FIELDS: [&str; 13] = [
    "ino",
    "mode",
    "uid",
    "gid",
    "nlink",
    "mtime",
    "filesize",
    "devmajor",
    "devminor",
    "rdevmajor",
    "rdevminor",
    "namesize",
    "check",
];

/// Which newc variant the archive uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// `070701`: the `check` field is unused (Android's mkbootfs writes this).
    #[default]
    Newc,
    /// `070702`: `check` is the 32-bit sum of the data bytes.
    NewcCrc,
}

impl Format {
    fn magic(self) -> &'static [u8; 6] {
        match self {
            Format::Newc => b"070701",
            Format::NewcCrc => b"070702",
        }
    }
}

/// One cpio entry. Every header field is kept so an unmodified archive
/// writes back byte-identical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub ino: u32,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub nlink: u32,
    pub mtime: u32,
    pub dev_major: u32,
    pub dev_minor: u32,
    pub rdev_major: u32,
    pub rdev_minor: u32,
    /// The raw `check` field of a `070701` entry, kept for byte-identical
    /// output. In `070702` archives the checksum is verified on parse and
    /// recomputed on write, and this stays 0.
    pub check: u32,
    pub data: Vec<u8>,
}

impl Entry {
    fn new(name: impl Into<String>, mode: u32, data: Vec<u8>) -> Self {
        Entry {
            name: name.into(),
            ino: 0,
            mode,
            uid: 0,
            gid: 0,
            nlink: 1,
            mtime: 0,
            dev_major: 0,
            dev_minor: 0,
            rdev_major: 0,
            rdev_minor: 0,
            check: 0,
            data,
        }
    }

    /// A regular file owned by root, with permission bits `perm` (e.g. `0o750`).
    pub fn file(name: impl Into<String>, perm: u32, data: impl Into<Vec<u8>>) -> Self {
        Entry::new(name, S_IFREG | (perm & 0o7777), data.into())
    }

    /// A directory owned by root.
    pub fn dir(name: impl Into<String>, perm: u32) -> Self {
        Entry::new(name, S_IFDIR | (perm & 0o7777), Vec::new())
    }

    /// A symlink; its data is the link target.
    pub fn symlink(name: impl Into<String>, target: impl Into<String>) -> Self {
        Entry::new(name, S_IFLNK | 0o777, target.into().into_bytes())
    }

    pub fn file_type(&self) -> u32 {
        self.mode & S_IFMT
    }

    pub fn is_file(&self) -> bool {
        self.file_type() == S_IFREG
    }

    pub fn is_dir(&self) -> bool {
        self.file_type() == S_IFDIR
    }

    pub fn is_symlink(&self) -> bool {
        self.file_type() == S_IFLNK
    }

    /// Permission bits, including setuid/setgid/sticky.
    pub fn permissions(&self) -> u32 {
        self.mode & 0o7777
    }

    fn fields(&self, namesize: u32, check: u32) -> Result<[u32; 13]> {
        let filesize = u32::try_from(self.data.len()).map_err(|_| Error::TooLarge {
            name: self.name.clone(),
        })?;
        Ok([
            self.ino,
            self.mode,
            self.uid,
            self.gid,
            self.nlink,
            self.mtime,
            filesize,
            self.dev_major,
            self.dev_minor,
            self.rdev_major,
            self.rdev_minor,
            namesize,
            check,
        ])
    }
}

/// A parsed newc archive: entries in archive order, the trailer entry and the
/// bytes that follow it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    pub format: Format,
    entries: Vec<Entry>,
    trailer: Entry,
    tail: Vec<u8>,
}

impl Default for Archive {
    fn default() -> Self {
        Archive::new()
    }
}

impl Archive {
    /// An empty `070701` archive with a standard trailer and no padding.
    pub fn new() -> Self {
        Archive {
            format: Format::Newc,
            entries: Vec::new(),
            trailer: Entry::new(TRAILER, 0, Vec::new()),
            tail: Vec::new(),
        }
    }

    /// Parses a whole archive. Bytes after the trailer are kept as the tail.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut format = None;
        let mut entries = Vec::new();
        let mut off = 0;
        loop {
            if off >= bytes.len() {
                return Err(Error::MissingTrailer);
            }
            let (entry, entry_format, next) = parse_entry(bytes, off)?;
            if *format.get_or_insert(entry_format) != entry_format {
                return Err(Error::BadMagic { offset: off });
            }
            off = next;
            if entry.name == TRAILER {
                let tail = bytes.get(off..).unwrap_or_default().to_vec();
                return Ok(Archive {
                    format: entry_format,
                    entries,
                    trailer: entry,
                    tail,
                });
            }
            entries.push(entry);
        }
    }

    /// Serializes the archive, recomputing `check` for [`Format::NewcCrc`].
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let size = self
            .entries
            .iter()
            .map(|e| HEADER_LEN + e.name.len() + e.data.len() + 8)
            .sum::<usize>()
            + HEADER_LEN
            + 16
            + self.tail.len();
        let mut out = Vec::with_capacity(size);
        self.write_to(&mut out)?;
        Ok(out)
    }

    pub fn write_to<W: Write>(&self, out: &mut W) -> Result<()> {
        let mut written = 0;
        for entry in self.entries.iter().chain(std::iter::once(&self.trailer)) {
            written += write_entry(out, self.format, entry, written)?;
        }
        out.write_all(&self.tail)?;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries in archive order, without the trailer.
    pub fn iter(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter()
    }

    /// The entry the kernel ends up with for `name`. Names can repeat
    /// (mkbootfs writes `dev` twice) and the last one wins on unpack, so this
    /// returns the last match.
    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().rev().find(|e| e.name == name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Entry> {
        self.entries.iter_mut().rev().find(|e| e.name == name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Adds `entry`, or replaces the effective entry of the same name in
    /// place. A new entry with `ino == 0` gets the next free inode; a
    /// replacement keeps the inode of the entry it replaces. Returns the
    /// replaced entry.
    pub fn insert(&mut self, mut entry: Entry) -> Option<Entry> {
        if let Some(existing) = self.get_mut(&entry.name) {
            if entry.ino == 0 {
                entry.ino = existing.ino;
            }
            return Some(std::mem::replace(existing, entry));
        }
        if entry.ino == 0 {
            entry.ino = self.next_ino();
        }
        self.entries.push(entry);
        None
    }

    /// Removes every entry named `name`; returns the effective (last) one.
    pub fn remove(&mut self, name: &str) -> Option<Entry> {
        let last = self.entries.iter().rposition(|e| e.name == name)?;
        let removed = self.entries.remove(last);
        self.entries.retain(|e| e.name != name);
        Some(removed)
    }

    /// Renames the entry `from` to `to`, keeping its position and header.
    pub fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        if from != to && self.contains(to) {
            return Err(Error::Exists(to.to_owned()));
        }
        let entry = self
            .get_mut(from)
            .ok_or_else(|| Error::NotFound(from.to_owned()))?;
        entry.name = to.to_owned();
        Ok(())
    }

    /// One more than the largest inode in use (trailer included).
    pub fn next_ino(&self) -> u32 {
        self.entries
            .iter()
            .chain(std::iter::once(&self.trailer))
            .map(|e| e.ino)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    /// The bytes after the trailer entry (usually zero padding).
    pub fn tail(&self) -> &[u8] {
        &self.tail
    }

    pub fn set_tail(&mut self, tail: Vec<u8>) {
        self.tail = tail;
    }
}

fn pad4(n: usize) -> usize {
    (4 - n % 4) % 4
}

fn slice<'a>(bytes: &'a [u8], start: usize, len: usize, what: &'static str) -> Result<&'a [u8]> {
    start
        .checked_add(len)
        .and_then(|end| bytes.get(start..end))
        .ok_or(Error::Truncated {
            offset: start,
            what,
        })
}

fn parse_hex(field: &[u8], offset: usize, name: &'static str) -> Result<u32> {
    let bad = || Error::BadHeader {
        offset,
        field: name,
    };
    let text = std::str::from_utf8(field).map_err(|_| bad())?;
    u32::from_str_radix(text, 16).map_err(|_| bad())
}

/// Parses the entry at `off`; returns it, its format and the offset of the next one.
fn parse_entry(bytes: &[u8], off: usize) -> Result<(Entry, Format, usize)> {
    let header = slice(bytes, off, HEADER_LEN, "a 110-byte cpio header")?;
    let format = match header.get(..6) {
        Some(b"070701") => Format::Newc,
        Some(b"070702") => Format::NewcCrc,
        _ => return Err(Error::BadMagic { offset: off }),
    };
    let mut f = [0u32; 13];
    for (i, (value, name)) in f.iter_mut().zip(FIELDS).enumerate() {
        let start = 6 + 8 * i;
        let digits = header.get(start..start + 8).ok_or(Error::Truncated {
            offset: off + start,
            what: "a cpio header field",
        })?;
        *value = parse_hex(digits, off + start, name)?;
    }
    let [
        ino,
        mode,
        uid,
        gid,
        nlink,
        mtime,
        filesize,
        dev_major,
        dev_minor,
        rdev_major,
        rdev_minor,
        namesize,
        check,
    ] = f;

    let name_off = off + HEADER_LEN;
    let raw_name = slice(bytes, name_off, namesize as usize, "a cpio entry name")?;
    let name = match raw_name.split_last() {
        Some((0, name)) => std::str::from_utf8(name)
            .map_err(|_| Error::BadName { offset: name_off })?
            .to_owned(),
        _ => return Err(Error::BadName { offset: name_off }),
    };
    let data_off = name_off + namesize as usize;
    let data_off = data_off + pad4(data_off);
    let data = slice(bytes, data_off, filesize as usize, "cpio entry data")?.to_vec();
    let next = data_off + data.len();
    let next = next + pad4(next);

    let check = match format {
        Format::Newc => check,
        Format::NewcCrc if checksum(&data) == check => 0,
        Format::NewcCrc => return Err(Error::Checksum { name }),
    };
    let entry = Entry {
        name,
        ino,
        mode,
        uid,
        gid,
        nlink,
        mtime,
        dev_major,
        dev_minor,
        rdev_major,
        rdev_minor,
        check,
        data,
    };
    // A short final padding run is tolerated: some writers stop at the data.
    Ok((entry, format, next.min(bytes.len())))
}

fn checksum(data: &[u8]) -> u32 {
    data.iter()
        .fold(0u32, |sum, &b| sum.wrapping_add(u32::from(b)))
}

/// Writes one entry starting at archive offset `at`; returns the bytes written.
fn write_entry<W: Write>(out: &mut W, format: Format, entry: &Entry, at: usize) -> Result<usize> {
    let namesize = u32::try_from(entry.name.len() + 1).map_err(|_| Error::TooLarge {
        name: entry.name.clone(),
    })?;
    let check = match format {
        Format::Newc => entry.check,
        Format::NewcCrc => checksum(&entry.data),
    };
    let mut header = Vec::with_capacity(HEADER_LEN + entry.name.len() + 4);
    header.extend_from_slice(format.magic());
    for value in entry.fields(namesize, check)? {
        header.extend_from_slice(format!("{value:08x}").as_bytes());
    }
    header.extend_from_slice(entry.name.as_bytes());
    header.push(0);
    header.resize(header.len() + pad4(at + header.len()), 0);
    out.write_all(&header)?;
    out.write_all(&entry.data)?;
    let end = at + header.len() + entry.data.len();
    let padding = pad4(end);
    out.write_all(&[0; 3][..padding])?;
    Ok(header.len() + entry.data.len() + padding)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Archive {
        let mut a = Archive::new();
        a.insert(Entry::dir("dev", 0o755));
        a.insert(Entry::file("init", 0o750, b"#!init".to_vec()));
        a.insert(Entry::symlink("bin", "system/bin"));
        a
    }

    #[test]
    fn round_trips_and_pads_to_four_bytes() {
        let a = sample();
        let bytes = a.to_bytes().unwrap();
        assert_eq!(bytes.len() % 4, 0);
        assert_eq!(Archive::parse(&bytes).unwrap(), a);
    }

    #[test]
    fn header_matches_the_newc_layout() {
        let mut a = Archive::new();
        a.insert(Entry::file("a", 0o644, b"xyz".to_vec()));
        let bytes = a.to_bytes().unwrap();
        let expected_header = b"070701\
            00000001000081a4000000000000000000000001000000000000000300000000\
            0000000000000000000000000000000200000000a\0";
        assert_eq!(&bytes[..112], &expected_header[..]);
        // 110 + 2 = 112 is aligned, so the data follows directly, then one byte of padding.
        assert_eq!(&bytes[112..116], b"xyz\0");
        assert_eq!(&bytes[116..122], b"070701");
    }

    #[test]
    fn keeps_trailer_fields_and_tail() {
        let mut a = sample();
        a.trailer.mode = 0o755;
        a.trailer.ino = 300_012;
        a.set_tail(vec![0; 208]);
        let bytes = a.to_bytes().unwrap();
        let parsed = Archive::parse(&bytes).unwrap();
        assert_eq!(parsed.trailer.ino, 300_012);
        assert_eq!(parsed.tail().len(), 208);
        assert_eq!(parsed.to_bytes().unwrap(), bytes);
    }

    #[test]
    fn duplicate_names_resolve_to_the_last_entry() {
        let mut a = Archive::new();
        a.insert(Entry::dir("dev", 0o755));
        a.entries.push(Entry::dir("dev", 0o700));
        assert_eq!(a.get("dev").unwrap().permissions(), 0o700);
        assert_eq!(a.remove("dev").unwrap().permissions(), 0o700);
        assert!(a.is_empty());
    }

    #[test]
    fn insert_replaces_in_place_and_keeps_the_inode() {
        let mut a = sample();
        let ino = a.get("init").unwrap().ino;
        let old = a.insert(Entry::file("init", 0o750, b"magiskinit".to_vec()));
        assert_eq!(old.unwrap().data, b"#!init");
        let names: Vec<_> = a.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["dev", "init", "bin"]);
        assert_eq!(a.get("init").unwrap().ino, ino);
    }

    #[test]
    fn new_entries_get_fresh_inodes() {
        let mut a = sample();
        let inos: Vec<_> = a.iter().map(|e| e.ino).collect();
        assert_eq!(inos, [1, 2, 3]);
        a.trailer.ino = 50;
        a.insert(Entry::dir("overlay.d", 0o750));
        assert_eq!(a.get("overlay.d").unwrap().ino, 51);
    }

    #[test]
    fn rename_moves_an_entry_and_refuses_to_clobber() {
        let mut a = sample();
        a.rename("init", ".backup/init").unwrap();
        assert!(!a.contains("init"));
        assert!(a.contains(".backup/init"));
        assert!(matches!(a.rename("dev", "bin"), Err(Error::Exists(_))));
        assert!(matches!(a.rename("nope", "x"), Err(Error::NotFound(_))));
    }

    #[test]
    fn crc_format_checks_data() {
        let mut a = sample();
        a.format = Format::NewcCrc;
        let mut bytes = a.to_bytes().unwrap();
        assert_eq!(Archive::parse(&bytes).unwrap().format, Format::NewcCrc);
        let pos = bytes.windows(6).position(|w| w == b"#!init").unwrap();
        bytes[pos] = b'?';
        assert!(matches!(
            Archive::parse(&bytes),
            Err(Error::Checksum { .. })
        ));
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        assert!(matches!(Archive::parse(b""), Err(Error::MissingTrailer)));
        assert!(matches!(
            Archive::parse(&[b'x'; 200]),
            Err(Error::BadMagic { offset: 0 })
        ));
        let mut bytes = sample().to_bytes().unwrap();
        bytes[8] = b'z';
        assert!(matches!(
            Archive::parse(&bytes),
            Err(Error::BadHeader { field: "ino", .. })
        ));
        let bytes = sample().to_bytes().unwrap();
        let cut = bytes.len() - 120;
        assert!(Archive::parse(&bytes[..cut]).is_err());
    }

    #[test]
    fn missing_nul_in_name_is_an_error() {
        let mut bytes = sample().to_bytes().unwrap();
        // "dev\0" -> "devX"
        bytes[113] = b'X';
        assert!(matches!(
            Archive::parse(&bytes),
            Err(Error::BadName { offset: 110 })
        ));
    }
}
