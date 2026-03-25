#!/bin/sh
# Bootstrap: build release with cargo, then use redo to cross-build.
set -e

echo "=== Bootstrap: building release with cargo ==="
cargo build --release

echo "=== Using release redo to build debug ==="
rm -rf .redo
export PATH="$PWD/target/release:$PATH"
redo debug

echo "=== Using debug redo to rebuild release ==="
rm -rf .redo
export PATH="$PWD/target/debug:$PATH"
redo release

echo "=== Done ==="
