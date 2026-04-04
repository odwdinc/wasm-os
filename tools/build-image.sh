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

# 1. Compile userland .wat → .wasm.
echo "[1/4] compiling userland modules..."
"$SCRIPT_DIR/wasm-pack.sh"

# 2. Pack .wasm files into fs.img (WasmFS format — embedded by the kernel).
echo "[2/4] packing filesystem image..."
mapfile -d '' WASM_FILES < <(find "$ROOT/userland" -name "*.wasm" -print0 | sort -z)
"$SCRIPT_DIR/pack-fs.sh" "${WASM_FILES[@]}"

# 3. Build the kernel for x86_64-unknown-none.
echo "[3/4] building kernel (profile: $PROFILE)..."
cargo build --package kernel "${CARGO_FLAGS[@]}"

KERNEL="target/x86_64-unknown-none/$PROFILE/kernel"
if [ ! -f "$KERNEL" ]; then
    echo "error: kernel ELF not found at $KERNEL"
    exit 1
fi

# 4. Run the host-side runner to wrap the ELF into a BIOS disk image.
echo "[4/4] creating disk image..."
HOST=$(rustc -vV 2>/dev/null | grep '^host:' | sed 's/host: //')
cargo run \
    --manifest-path runner/Cargo.toml \
    --target "$HOST" \
    --quiet \
    -- "$KERNEL"

echo "done."
echo "  boot image : target/x86_64-unknown-none/$PROFILE/kernel-bios.img"
echo "  fs image   : fs.img"
