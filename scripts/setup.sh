#!/usr/bin/env bash
# Install all prerequisites for building and running the OS.
set -e

echo "=== setting up dev environment ==="

# Rust / rustup
if ! command -v rustup &>/dev/null; then
    echo "[setup] installing rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain nightly
    source "$HOME/.cargo/env"
else
    echo "[setup] rustup found, updating..."
    rustup update
fi

# Nightly toolchain + required components
echo "[setup] configuring nightly toolchain..."
rustup toolchain install nightly
rustup override set nightly
rustup target add x86_64-unknown-none
rustup component add rust-src llvm-tools-preview

# QEMU
if command -v qemu-system-x86_64 &>/dev/null; then
    echo "[setup] QEMU already installed: $(qemu-system-x86_64 --version | head -1)"
else
    echo "[setup] QEMU not found. Install it for your platform:"
    echo "  Ubuntu/Debian : sudo apt install qemu-system-x86"
    echo "  Fedora/RHEL   : sudo dnf install qemu-system-x86"
    echo "  Arch          : sudo pacman -S qemu-system-x86"
    echo "  macOS         : brew install qemu"
fi

# wabt (optional — only needed for wasm-pack.sh)
if command -v wat2wasm &>/dev/null; then
    echo "[setup] wabt already installed."
else
    echo "[setup] wabt (optional) not found. Install for .wat compilation:"
    echo "  Ubuntu/Debian : sudo apt install wabt"
    echo "  Arch          : sudo pacman -S wabt"
    echo "  macOS         : brew install wabt"
fi

# lld linker (required for x86_64-unknown-none linking)
if command -v ld.lld &>/dev/null; then
    echo "[setup] ld.lld found."
else
    echo "[setup] ld.lld not found. Install llvm/lld:"
    echo "  Ubuntu/Debian : sudo apt install lld"
    echo "  Arch          : sudo pacman -S lld"
    echo "  macOS         : brew install llvm"
fi

echo ""
echo "=== setup complete ==="
echo "Run  bash tools/run-qemu.sh  to build and boot in QEMU."
echo "To write to USB for bare metal:  dd if=<disk.img> of=/dev/sdX bs=1M status=progress"
