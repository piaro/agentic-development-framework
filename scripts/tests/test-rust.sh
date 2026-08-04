#!/bin/sh

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
KIT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)
RUST_ROOT=$KIT_ROOT

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required for the Rust prototype" >&2
  exit 1
fi

cargo fmt --manifest-path "$RUST_ROOT/Cargo.toml" --check
cargo clippy \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --all-targets \
  --locked \
  -- \
  -D warnings
cargo test --manifest-path "$RUST_ROOT/Cargo.toml" --locked
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- benchmark "$KIT_ROOT/testdata/benchmarks/major-frameworks-v1" \
  --format text
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- benchmark "$KIT_ROOT/testdata/benchmarks/real-projects-v1" \
  --format text
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- detector-audit "$KIT_ROOT" \
  --format text
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- catalog signal-domains \
  --format json
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-canonicalization "$KIT_ROOT/testdata/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-schema "$KIT_ROOT/testdata/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-rules "$KIT_ROOT/testdata/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-detection "$KIT_ROOT/testdata/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-kernel "$KIT_ROOT/testdata/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-context "$KIT_ROOT/testdata/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-project "$KIT_ROOT/testdata/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-lock "$KIT_ROOT/testdata/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-submission "$KIT_ROOT/testdata/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-application "$KIT_ROOT/testdata/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-store "$KIT_ROOT/testdata/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-persistent "$KIT_ROOT/testdata/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-explain "$KIT_ROOT/testdata/golden/v1"

sh "$KIT_ROOT/scripts/tests/test-release-ci.sh"
sh "$KIT_ROOT/scripts/tests/test-release-publication.sh"
sh "$KIT_ROOT/scripts/tests/test-bootstrap.sh"
