// Test helpers outside #[test] fns aren't covered by clippy.toml's allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use jidr_cpio::compress::{self, XzCheck};
use jidr_cpio::{Archive, Compression, Entry, Format, Ramdisk, Segment};
use proptest::prelude::*;

fn entry() -> impl Strategy<Value = Entry> {
    let name = "[a-z0-9_.]{1,12}(/[a-z0-9_.-]{1,12}){0,3}";
    let data = prop::collection::vec(any::<u8>(), 0..300);
    (name, 0u8..3, 0u32..0o7777, data, any::<u32>(), any::<u32>()).prop_map(
        |(name, kind, perm, data, uid, mtime)| {
            let mut e = match kind {
                0 => Entry::file(name, perm, data),
                1 => Entry::dir(name, perm),
                _ => Entry::symlink(name, String::from_utf8_lossy(&data).into_owned()),
            };
            e.uid = uid;
            e.mtime = mtime;
            e
        },
    )
}

fn archive() -> impl Strategy<Value = Archive> {
    (
        prop::collection::vec(entry(), 0..20),
        any::<bool>(),
        0usize..600,
    )
        .prop_map(|(entries, crc, tail)| {
            let mut a = Archive::new();
            if crc {
                a.format = Format::NewcCrc;
            }
            for e in entries {
                a.insert(e);
            }
            a.set_tail(vec![0; tail]);
            a
        })
}

fn compression() -> impl Strategy<Value = Compression> {
    prop_oneof![
        Just(Compression::None),
        (1u32..=9).prop_map(|level| Compression::Gzip {
            level,
            header: vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3],
        }),
        prop_oneof![Just(1), Just(9), Just(12)].prop_map(|level| Compression::Lz4Legacy { level }),
        // Low presets keep the encoder's memory small; the default is tested elsewhere.
        (
            0u32..=2,
            prop_oneof![Just(XzCheck::Crc32), Just(XzCheck::Crc64)]
        )
            .prop_map(|(preset, check)| Compression::Xz { preset, check }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn archives_round_trip(a in archive()) {
        let bytes = a.to_bytes().unwrap();
        prop_assert_eq!((bytes.len() - a.tail().len()) % 4, 0);
        let parsed = Archive::parse(&bytes).unwrap();
        prop_assert_eq!(&parsed, &a);
        prop_assert_eq!(parsed.to_bytes().unwrap(), bytes);
    }

    #[test]
    fn edits_keep_the_archive_valid(a in archive(), extra in entry(), victim in 0usize..20) {
        let mut a = a;
        let names: Vec<String> = a.iter().map(|e| e.name.clone()).collect();
        if let Some(name) = names.get(victim % names.len().max(1)) {
            a.remove(name);
            prop_assert!(!a.contains(name));
        }
        let name = extra.name.clone();
        a.insert(extra);
        let parsed = Archive::parse(&a.to_bytes().unwrap()).unwrap();
        prop_assert!(parsed.contains(&name));
        prop_assert_eq!(parsed, a);
    }

    #[test]
    fn compression_round_trips(data in prop::collection::vec(any::<u8>(), 0..20_000), c in compression()) {
        // Raw cpio has to look like cpio to be detected.
        let data = if c == Compression::None {
            Archive::new().to_bytes().unwrap()
        } else {
            data
        };
        let packed = c.compress(&data).unwrap();
        let out = compress::decompress(&packed).unwrap();
        prop_assert_eq!(out.data, data);
        prop_assert_eq!(out.consumed, packed.len());
        prop_assert_eq!(out.compression.name(), c.name());
    }

    #[test]
    fn ramdisks_of_lz4_segments_round_trip(archives in prop::collection::vec(archive(), 1..4), pad in 0usize..64) {
        let ramdisk = Ramdisk {
            segments: archives
                .into_iter()
                .map(|archive| Segment { compression: Compression::lz4_legacy(), archive })
                .collect(),
            trailing: vec![0; pad],
        };
        let bytes = ramdisk.to_bytes().unwrap();
        let parsed = Ramdisk::parse(&bytes).unwrap();
        prop_assert_eq!(parsed.to_bytes().unwrap(), bytes);
    }

    #[test]
    fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..2_000)) {
        let _ = Archive::parse(&bytes);
        let _ = Ramdisk::parse(&bytes);
        let _ = compress::decompress(&bytes);
    }

    #[test]
    fn corrupted_archives_never_panic(a in archive(), at in any::<prop::sample::Index>(), byte in any::<u8>()) {
        let mut bytes = a.to_bytes().unwrap();
        if !bytes.is_empty() {
            let i = at.index(bytes.len());
            bytes[i] = byte;
        }
        let _ = Archive::parse(&bytes);
    }
}
