#!/usr/bin/env bash
# tools/pack-fs.sh — Create FAT filesystem images
#
# Usage:  pack-fs.sh [--disk-size SIZE] [file ...]
#
# Creates two images in $REPO_ROOT:
#
#   fs.img   — 2 MiB FAT12 embedded fallback.  Size is fixed and must match
#               RAM_FAT_SIZE in kernel/src/fs/fat.rs (used with include_bytes!).
#
#   disk.img — FAT virtio-blk disk at SIZE (default: 32M).  mkfs.fat
#               auto-selects FAT12/16/32 based on the size you choose.
#               Any size ≥ 2M is accepted.
#
# Both images receive the same set of files in their root directory.
#
# Requires: mkfs.fat (dosfstools), mcopy (mtools)
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

FSIMG="$ROOT/fs.img"
DISKIMG="$ROOT/disk.img"

# ── Argument parsing ──────────────────────────────────────────────────────────

DISK_SIZE="32M"
FILES=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --disk-size)
            DISK_SIZE="$2"; shift 2 ;;
        --disk-size=*)
            DISK_SIZE="${1#--disk-size=}"; shift ;;
        -*)
            echo "error: unknown option: $1"
            echo "usage: pack-fs.sh [--disk-size SIZE] [file ...]"
            exit 1 ;;
        *)
            FILES+=("$1"); shift ;;
    esac
done

# ── Parse human-readable size (e.g. 32M, 128M, 1G) into bytes ────────────────

parse_size() {
    local s="$1"
    local num="${s%%[KkMmGg]*}"
    local unit="${s#"$num"}"
    case "$unit" in
        K|k) echo $((num * 1024)) ;;
        M|m) echo $((num * 1024 * 1024)) ;;
        G|g) echo $((num * 1024 * 1024 * 1024)) ;;
        "")  echo "$num" ;;
        *)
            echo "error: unrecognised size suffix in '$s' (use K, M or G)"
            exit 1 ;;
    esac
}

DISK_BYTES=$(parse_size "$DISK_SIZE")
DISK_SECTORS=$((DISK_BYTES / 512))

# Minimum: must be bigger than the embedded fallback (2 MiB).
MIN_DISK_BYTES=$((2 * 1024 * 1024))
if (( DISK_BYTES < MIN_DISK_BYTES )); then
    echo "error: --disk-size must be at least 2M (got $DISK_SIZE)"
    exit 1
fi

# ── Tool detection ────────────────────────────────────────────────────────────

MKFSFAT=""
for candidate in mkfs.fat mkdosfs /sbin/mkfs.fat /usr/sbin/mkfs.fat \
                 /sbin/mkdosfs /usr/sbin/mkdosfs; do
    if command -v "$candidate" &>/dev/null 2>&1 || [ -x "$candidate" ]; then
        MKFSFAT="$candidate"
        break
    fi
done
if [ -z "$MKFSFAT" ]; then
    echo "error: mkfs.fat / mkdosfs not found — install dosfstools"
    echo "  Ubuntu/Debian: sudo apt install dosfstools mtools"
    exit 1
fi

if ! command -v mcopy &>/dev/null; then
    echo "error: mcopy not found — install mtools"
    echo "  Ubuntu/Debian: sudo apt install mtools"
    exit 1
fi

# ── Build fs.img — fixed 2 MiB FAT12 embedded fallback ───────────────────────

FS_BYTES=$((2 * 1024 * 1024))   # must match RAM_FAT_SIZE in kernel/src/fs/fat.rs
FS_SECTORS=$((FS_BYTES / 512))

dd if=/dev/zero of="$FSIMG" bs=512 count="$FS_SECTORS" status=none
"$MKFSFAT" -F 12 -n "KERNELFS" "$FSIMG" >/dev/null

# ── Build disk.img — configurable virtio-blk disk ────────────────────────────

dd if=/dev/zero of="$DISKIMG" bs=512 count="$DISK_SECTORS" status=none
# Let mkfs.fat pick FAT12/16/32 based on size.
"$MKFSFAT" -n "KERNELDISK" "$DISKIMG" >/dev/null

# ── Copy files into both images ───────────────────────────────────────────────

export MTOOLS_SKIP_CHECK=1
COUNT=0

for f in "${FILES[@]}"; do
    name="$(basename "$f")"
    size="$(wc -c < "$f")"
    mcopy -i "$FSIMG"   "$f" "::/$name"
    mcopy -i "$DISKIMG" "$f" "::/$name"
    echo "  packed: $name ($size bytes)"
    COUNT=$((COUNT + 1))
done

echo "fs.img:   FAT12, ${FS_BYTES} bytes, $COUNT file(s)  [embedded fallback]"
echo "disk.img: FAT,   ${DISK_BYTES} bytes, $COUNT file(s)  [virtio-blk, --disk-size $DISK_SIZE]"
