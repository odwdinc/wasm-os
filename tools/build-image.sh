#!/usr/bin/env bash
# Build the kernel and produce a bootable BIOS disk image (disk.img).
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

PROFILE="${1:-debug}"
CARGO_FLAGS=()
if [ "$PROFILE" = "release" ]; then
    CARGO_FLAGS+=(--release)
fi

# 1. Build the kernel for x86_64-unknown-none
echo "[1/2] building kernel (profile: $PROFILE)..."
cargo build --package kernel "${CARGO_FLAGS[@]}"

KERNEL="target/x86_64-unknown-none/$PROFILE/kernel"

if [ ! -f "$KERNEL" ]; then
    echo "error: kernel ELF not found at $KERNEL"
    exit 1
fi

# 2. Build the runner with the host toolchain (needs std) and create the image.
#    --target overrides the workspace-level x86_64-unknown-none target for this crate.
echo "[2/2] creating disk image..."
HOST=$(rustc -vV 2>/dev/null | grep '^host:' | sed 's/host: //')
cargo run \
    --manifest-path runner/Cargo.toml \
    --target "$HOST" \
    --quiet \
    -- "$KERNEL"

echo "done. disk image written to target/x86_64-unknown-none/$PROFILE/"
