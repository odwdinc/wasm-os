#!/usr/bin/env bash
# Build pipeline step 2 of 3: compile the kernel and produce a bootable BIOS
# disk image.  Automatically runs wasm-pack.sh first so .wasm bytes are fresh.
#
# Usage: build-image.sh [debug|release]
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

PROFILE="${1:-debug}"
CARGO_FLAGS=()
if [ "$PROFILE" = "release" ]; then
    CARGO_FLAGS+=(--release)
fi

# 1. Compile userland .wat → .wasm (kernel embeds these at compile time).
echo "[1/3] compiling userland modules..."
"$SCRIPT_DIR/wasm-pack.sh"

# 2. Build the kernel for x86_64-unknown-none.
echo "[2/3] building kernel (profile: $PROFILE)..."
cargo build --package kernel "${CARGO_FLAGS[@]}"

KERNEL="target/x86_64-unknown-none/$PROFILE/kernel"
if [ ! -f "$KERNEL" ]; then
    echo "error: kernel ELF not found at $KERNEL"
    exit 1
fi

# 3. Run the host-side runner to wrap the ELF into a BIOS disk image.
echo "[3/3] creating disk image..."
HOST=$(rustc -vV 2>/dev/null | grep '^host:' | sed 's/host: //')
cargo run \
    --manifest-path runner/Cargo.toml \
    --target "$HOST" \
    --quiet \
    -- "$KERNEL"

echo "done. image: target/x86_64-unknown-none/$PROFILE/kernel-bios.img"
