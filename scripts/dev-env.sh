#!/usr/bin/env bash
# Check the development environment and report status of each prerequisite.

OK="\033[0;32m[ ok ]\033[0m"
FAIL="\033[0;31m[miss]\033[0m"
WARN="\033[0;33m[warn]\033[0m"

echo "=== dev environment ==="

# rustc
if command -v rustc &>/dev/null; then
    echo -e "$OK rust: $(rustc --version)"
else
    echo -e "$FAIL rust: not found — install via https://rustup.rs"
fi

# nightly toolchain
if rustup toolchain list 2>/dev/null | grep -q nightly; then
    echo -e "$OK nightly toolchain"
else
    echo -e "$FAIL nightly: run 'rustup toolchain install nightly'"
fi

# bare-metal target
if rustup target list --installed 2>/dev/null | grep -q "x86_64-unknown-none"; then
    echo -e "$OK target x86_64-unknown-none"
else
    echo -e "$FAIL x86_64-unknown-none: run 'rustup target add x86_64-unknown-none'"
fi

# rust-src component (needed for core/compiler-builtins)
if rustup component list --installed 2>/dev/null | grep -q "^rust-src"; then
    echo -e "$OK rust-src component"
else
    echo -e "$FAIL rust-src: run 'rustup component add rust-src'"
fi

# lld linker
if command -v ld.lld &>/dev/null; then
    echo -e "$OK ld.lld linker"
else
    echo -e "$FAIL ld.lld: install lld (Ubuntu: 'sudo apt install lld')"
fi

# QEMU
if command -v qemu-system-x86_64 &>/dev/null; then
    echo -e "$OK qemu: $(qemu-system-x86_64 --version | head -1)"
else
    echo -e "$FAIL qemu: not found — see scripts/setup.sh"
fi

# wat2wasm (optional)
if command -v wat2wasm &>/dev/null; then
    echo -e "$OK wat2wasm (wabt)"
else
    echo -e "$WARN wat2wasm: optional, needed for tools/wasm-pack.sh"
fi

echo "======================="
