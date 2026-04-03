#!/usr/bin/env bash
# Build pipeline step 3 of 3: build everything, then boot in QEMU.
#
# Usage: run-qemu.sh [debug|release]
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

PROFILE="${1:-debug}"

# 1. Run the full build pipeline (wasm-pack → kernel → disk image).
"$SCRIPT_DIR/build-image.sh" "$PROFILE"

# 2. Boot the disk image in QEMU.
IMG="target/x86_64-unknown-none/$PROFILE/kernel-bios.img"
if [ ! -f "$IMG" ]; then
    echo "error: disk image not found at $IMG"
    exit 1
fi

if ! command -v qemu-system-x86_64 &>/dev/null; then
    echo "error: qemu-system-x86_64 not found."
    echo "  Install QEMU:"
    echo "    Ubuntu/Debian : sudo apt install qemu-system-x86"
    echo "    Arch          : sudo pacman -S qemu-system-x86"
    echo "    macOS         : brew install qemu"
    exit 1
fi

echo "booting $IMG..."
qemu-system-x86_64 \
    -drive format=raw,file="$IMG" \
    -m 512M \
    -serial stdio \
    -no-reboot \
    -no-shutdown
