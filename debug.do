# Build the debug binary using the release redo.
# Bootstrap: cargo builds release first, then release-redo builds debug.
redo-ifchange Cargo.toml $(find src -name '*.rs')
cargo build >&2
