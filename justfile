# Build and test redo-rs

# Run all tests (default)
test: test-smoke test-suite test-regression test-equivalence

# Fast smoke tests: verify the binary works at all (debug + release)
test-smoke:
    cargo test --test smoke -- --test-threads=1
    @echo "--- Release smoke tests ---"
    cargo test --release --test smoke -- --test-threads=1

# Run the original apenwarr/redo test suite
test-suite: build
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/target/release:$PATH"
    export REDO_LOG=0
    # Create helper files the tests need
    mkdir -p redo
    [ -L redo/sh ] || ln -sf /bin/sh redo/sh
    [ -x redo/py ] || (echo '#!/bin/sh'; echo 'exec python3 "$@"') > redo/py && chmod +x redo/py
    [ -x t/flush-cache ] || (echo '#!/usr/bin/env python3'; cat t/flush-cache.in) > t/flush-cache && chmod +x t/flush-cache
    PASS=0; FAIL=0; SKIP=0
    for d in t/[0-9s][0-9][0-9]*/; do
        test_name=$(basename "$d")
        case "$test_name" in 110-*|111-*|999-*) SKIP=$((SKIP+1)); continue;; esac
        cd "$d"; rm -rf ../../.redo
        if timeout 60 redo all >/dev/null 2>&1; then
            PASS=$((PASS+1))
        else
            echo "FAIL: $test_name"
            FAIL=$((FAIL+1))
        fi
        cd ../../
    done
    echo "Suite: $PASS passed, $FAIL failed, $SKIP skipped"
    [ "$FAIL" -eq 0 ]

# Run the standalone regression tests
test-regression:
    cargo test --test target_dir_collision -- --test-threads=1

# Run the hegel property-based equivalence tests (slow)
test-equivalence:
    cargo test --test equivalence -- --test-threads=1

# Build release binary
build:
    cargo build --release

# Build debug binary
build-debug:
    cargo build

# Clean build artifacts and test state
clean:
    cargo clean
    rm -rf .redo t/*/.redo redo/sh redo/py t/flush-cache bin/
