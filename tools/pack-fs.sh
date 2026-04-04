#!/usr/bin/env bash
# tools/pack-fs.sh — Pack files into a WasmFS filesystem image (Sprint D.3)
#
# Usage:  pack-fs.sh [file ...]
# Output: $REPO_ROOT/fs.img
#
# With no arguments produces an empty (all-zeros directory block) image.
# With file arguments packs each file into the WasmFS flat format:
#
#   Block 0        — directory (8 × 64-byte entries, 512 bytes total)
#   Blocks 1..N    — file data (one contiguous run per file)
#
# Requires: python3
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if ! command -v python3 &>/dev/null; then
    echo "error: python3 not found — required to build fs.img"
    exit 1
fi

python3 - "$ROOT" "$@" <<'PYEOF'
import sys, os, struct

root  = sys.argv[1]
files = sys.argv[2:]
out   = os.path.join(root, "fs.img")

BLOCK_SIZE     = 512
DIR_ENTRY_SIZE = 64
MAX_ENTRIES    = BLOCK_SIZE // DIR_ENTRY_SIZE   # 8
DATA_START     = 1                               # data begins at block 1
FLAG_VALID     = 0x01

if len(files) > MAX_ENTRIES:
    print(f"error: too many files ({len(files)} > {MAX_ENTRIES})", file=sys.stderr)
    sys.exit(1)

# Read all file contents up-front so we can compute start blocks.
entries      = []
current_blk  = DATA_START
for path in files:
    name = os.path.basename(path)
    if len(name.encode("utf-8")) > 32:
        print(f"error: filename '{name}' exceeds 32 bytes", file=sys.stderr)
        sys.exit(1)
    with open(path, "rb") as f:
        data = f.read()
    blocks = max(1, (len(data) + BLOCK_SIZE - 1) // BLOCK_SIZE)
    entries.append((name, current_blk, data))
    current_blk += blocks

# Build directory block (512 bytes, 8 slots of 64 bytes each).
dir_block = bytearray(BLOCK_SIZE)
for i, (name, start_blk, data) in enumerate(entries):
    off = i * DIR_ENTRY_SIZE
    name_b = name.encode("utf-8")[:32].ljust(32, b"\x00")
    dir_block[off:off+32]    = name_b
    dir_block[off+32:off+36] = struct.pack("<I", start_blk)
    dir_block[off+36:off+40] = struct.pack("<I", len(data))
    dir_block[off+40]        = FLAG_VALID
    # bytes 41–63 remain zero (reserved)

# Build data region (each file padded to a block boundary).
data_region = bytearray()
for _, _, data in entries:
    padding = (BLOCK_SIZE - len(data) % BLOCK_SIZE) % BLOCK_SIZE
    if len(data) == 0:
        padding = BLOCK_SIZE
    data_region.extend(data)
    data_region.extend(b"\x00" * padding)

with open(out, "wb") as f:
    f.write(bytes(dir_block))
    f.write(bytes(data_region))

total_blocks = 1 + (len(data_region) // BLOCK_SIZE)
print(f"fs.img: {len(entries)} file(s), {total_blocks} block(s), {os.path.getsize(out)} bytes")
for name, start_blk, data in entries:
    print(f"  {name:<32s}  {len(data):6d} bytes  @ block {start_blk}")
PYEOF
