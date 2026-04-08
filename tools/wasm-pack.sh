#!/usr/bin/env bash
# Compile all .wat source files under userland/ into .wasm binaries,
# and build any Rust cdylib crates under userland/ for wasm32-unknown-unknown.
# Requires: wat2wasm (part of the wabt toolkit), cargo (Rust toolchain)
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

if ! command -v wat2wasm &>/dev/null; then
    echo "error: wat2wasm not found."
    echo "  Install wabt:"
    echo "    Ubuntu/Debian : sudo apt install wabt"
    echo "    Arch          : sudo pacman -S wabt"
    echo "    macOS         : brew install wabt"
    exit 1
fi

BUILT=0
while IFS= read -r -d '' WAT; do
    WASM="${WAT%.wat}.wasm"
    echo "[wasm] $WAT -> $WASM"
    wat2wasm "$WAT" -o "$WASM"
    BUILT=$((BUILT + 1))
done < <(find userland -name "*.wat" -print0)

if [ "$BUILT" -eq 0 ]; then
    echo "no .wat files found under userland/"
else
    echo "done. compiled $BUILT module(s)."
fi

# ── Rust WASM crates ──────────────────────────────────────────────────────────
# Build every userland subdirectory that contains a Cargo.toml with
# crate-type = ["cdylib"] for wasm32-unknown-unknown, then copy the
# resulting .wasm artifact next to the Cargo.toml.

RUST_BUILT=0
while IFS= read -r -d '' CRATE_TOML; do
    CRATE_DIR="$(dirname "$CRATE_TOML")"
    # Only process crates whose Cargo.toml declares a cdylib lib target.
    if ! grep -q 'cdylib' "$CRATE_TOML" 2>/dev/null; then
        continue
    fi
    # Determine the output lib name from the [lib] name field (fallback to
    # the crate package name with hyphens replaced by underscores).
    LIB_NAME=$(awk -F'"' '/^\[lib\]/{in_lib=1} in_lib && /^name/{print $2; exit}' "$CRATE_TOML")
    if [ -z "$LIB_NAME" ]; then
        PKG_NAME=$(awk -F'"' '/^\[package\]/{in_pkg=1} in_pkg && /^name/{print $2; exit}' "$CRATE_TOML")
        LIB_NAME="${PKG_NAME//-/_}"
    fi
    echo "[rust-wasm] building $CRATE_DIR -> $LIB_NAME.wasm"
    (cd "$CRATE_DIR" && cargo build --release 2>&1)
    WASM_SRC="$CRATE_DIR/target/wasm32-unknown-unknown/release/${LIB_NAME}.wasm"
    if [ -f "$WASM_SRC" ]; then
        cp "$WASM_SRC" "$CRATE_DIR/${LIB_NAME}.wasm"
        echo "[rust-wasm] -> $CRATE_DIR/${LIB_NAME}.wasm"
        RUST_BUILT=$((RUST_BUILT + 1))
    else
        echo "warning: expected artifact not found: $WASM_SRC"
    fi
done < <(find userland -name "Cargo.toml" -print0)

if [ "$RUST_BUILT" -gt 0 ]; then
    echo "done. built $RUST_BUILT Rust WASM module(s)."
fi
