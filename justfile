# Build and test redo-rs

# Run all tests (default)
test: test-smoke test-suite test-regression

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
    PASS=0; FAIL=0; SKIP=0; TOTAL=0
    for d in t/[0-9s][0-9][0-9]*/; do
        test_name=$(basename "$d")
        TOTAL=$((TOTAL+1))
        case "$test_name" in
            110-*|111-*|999-*)
                printf "  %-30s \033[33mskip\033[0m\n" "$test_name"
                SKIP=$((SKIP+1))
                continue
                ;;
        esac
        printf "  %-30s " "$test_name"
        cd "$d"; rm -rf ../../.redo
        if timeout 60 redo all >/dev/null 2>&1; then
            printf "\033[32mok\033[0m\n"
            PASS=$((PASS+1))
        else
            printf "\033[31mFAIL\033[0m\n"
            FAIL=$((FAIL+1))
        fi
        cd ../../
    done
    echo ""
    echo "Suite: $PASS passed, $FAIL failed, $SKIP skipped (of $TOTAL)"
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

# Bootstrap: cargo builds release, release-redo builds debug, debug-redo rebuilds release
bootstrap:
    ./bootstrap.sh

# Clean build artifacts and test state
clean:
    cargo clean
    rm -rf .redo t/*/.redo redo/sh redo/py t/flush-cache bin/ release debug
