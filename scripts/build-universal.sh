#!/bin/sh
# Build a universal macOS binary that runs on both Apple silicon and Intel Macs.
#
# `cargo build --release` only produces a binary for the host architecture. This
# builds both slices and merges them with lipo, so the result can be handed to any
# Mac rather than only the machine that compiled it.
#
# Requires the std library for both targets:
#   rustup target add aarch64-apple-darwin x86_64-apple-darwin

set -eu

TARGETS="aarch64-apple-darwin x86_64-apple-darwin"
OUT_DIR="target/universal-apple-darwin/release"
OUT="$OUT_DIR/fftui"

cd "$(dirname "$0")/.."

SLICES=""
for target in $TARGETS; do
    echo "==> Building $target"
    cargo build --release --target "$target"
    SLICES="$SLICES target/$target/release/fftui"
done

mkdir -p "$OUT_DIR"
# shellcheck disable=SC2086
lipo -create -output "$OUT" $SLICES

echo "==> $OUT"
lipo -info "$OUT"
