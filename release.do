# Build the release binary using the debug redo.
# Bootstrap: cargo builds debug first, then debug-redo builds release.
redo-ifchange Cargo.toml $(find src -name '*.rs')
cargo build --release >&2
