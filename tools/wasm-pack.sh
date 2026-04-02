#!/usr/bin/env bash
# Compile all .wat source files under userland/ into .wasm binaries.
# Requires: wat2wasm (part of the wabt toolkit)
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
