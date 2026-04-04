#!/usr/bin/env bash
# Build pipeline step 3 of 3: build everything, then boot in QEMU.
#
# Usage: run-qemu.sh [debug|release|headless]
#
#   debug    (default) — build debug, show VGA window, serial → stdio
#   release            — build release, show VGA window, serial → stdio
#   headless           — build debug, no VGA window (serial only, good for CI)
#
# The serial port (COM1) is always connected to stdio via the QEMU monitor
# multiplexer (-serial mon:stdio).  In headless mode that is the only UI.
# Press Ctrl-A X to quit QEMU from the serial console.
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

ARG="${1:-debug}"

case "$ARG" in
    headless)
        PROFILE="debug"
        DISPLAY_ARGS="-display none"
        ;;
    release)
        PROFILE="release"
        DISPLAY_ARGS=""
        ;;
    debug|"")
        PROFILE="debug"
        DISPLAY_ARGS=""
        ;;
    *)
        echo "usage: run-qemu.sh [debug|release|headless]"
        exit 1
        ;;
esac

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

FS_IMG="$ROOT/fs.img"
FS_DRIVE=""
if [ -f "$FS_IMG" ]; then
    FS_DRIVE="-drive format=raw,file=$FS_IMG,if=ide,index=1,media=disk"
fi

echo "booting $IMG  [profile=$PROFILE${DISPLAY_ARGS:+, headless}]..."
echo "serial → stdio  (Ctrl-A X to quit)"
# shellcheck disable=SC2086
qemu-system-x86_64 \
    -drive format=raw,file="$IMG" \
    $FS_DRIVE \
    -m 512M \
    -serial mon:stdio \
    -no-reboot \
    -no-shutdown \
    $DISPLAY_ARGS
