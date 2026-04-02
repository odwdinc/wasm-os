#!/usr/bin/env bash
# Compile hello.wat -> hello.wasm
set -e
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if ! command -v wat2wasm &>/dev/null; then
    echo "error: wat2wasm not found. Run scripts/setup.sh for install instructions."
    exit 1
fi

wat2wasm "$DIR/hello.wat" -o "$DIR/hello.wasm"
echo "built: $DIR/hello.wasm"
