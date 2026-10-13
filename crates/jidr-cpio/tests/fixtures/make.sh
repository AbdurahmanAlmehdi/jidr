#!/bin/sh
# Regenerates the synthetic ramdisk fixtures with the reference tools, so the
# round-trip tests check jidr-cpio against them byte for byte. The content is
# our own (no Google files); output depends on the tool versions, so commit
# the results instead of regenerating in CI.
#
# Tools used for the committed files: bsdcpio 3.5.3 (libarchive 3.7.4, macOS),
# lz4 1.10.0, GNU-compatible gzip (macOS /usr/bin/gzip), xz 5.8.4.
set -eu
cd "$(dirname "$0")"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

make_tree() {
    root=$1
    mkdir -p "$root/dev" "$root/proc" "$root/first_stage_ramdisk" "$root/system/bin"
    printf '#!/system/bin/sh\necho jidr fixture\n' > "$root/init"
    chmod 750 "$root/init"
    printf 'system /system ext4 ro wait\n' > "$root/first_stage_ramdisk/fstab.ranchu"
    chmod 640 "$root/first_stage_ramdisk/fstab.ranchu"
    ln -s /system/bin/sh "$root/system/bin/sh"
}

archive() {
    # Sorted names, fixed mtime: stable entry order and headers.
    (cd "$1" && find . -mindepth 1 | sed 's|^\./||' | LC_ALL=C sort | cpio -o -H newc --quiet)
}

make_tree "$tmp/a"
# 9 MiB of repetitive text: the lz4 legacy stream needs two 8 MiB blocks.
yes 'jidr ramdisk fixture line' | head -c 9437184 > "$tmp/a/system/big.txt"
find "$tmp/a" -exec touch -h -t 202610130000 {} +
archive "$tmp/a" > "$tmp/a.cpio"

lz4 -l -12 -f -q "$tmp/a.cpio" newc.cpio.lz4
gzip -n -6 -c "$tmp/a.cpio" > newc.cpio.gz
xz -9 --check=crc32 -T1 -c "$tmp/a.cpio" > newc.cpio.xz

# Emulator layout: two lz4 legacy streams back to back.
mkdir -p "$tmp/b/lib/modules"
printf 'module\n' > "$tmp/b/lib/modules/virtio.ko"
find "$tmp/b" -exec touch -h -t 202610130000 {} +
archive "$tmp/b" > "$tmp/b.cpio"
rm "$tmp/a/system/big.txt"
archive "$tmp/a" > "$tmp/a-small.cpio"
lz4 -l -12 -f -q "$tmp/a-small.cpio" "$tmp/1.lz4"
lz4 -l -12 -f -q "$tmp/b.cpio" "$tmp/2.lz4"
cat "$tmp/1.lz4" "$tmp/2.lz4" > two-streams.img
