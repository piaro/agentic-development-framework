#!/bin/sh

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
KIT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
RUST_ROOT=$KIT_ROOT/prototype/vnext/rust

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required for the vNext Rust prototype" >&2
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
  -- benchmark "$KIT_ROOT/prototype/vnext/benchmarks/major-frameworks-v1" \
  --format text
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-canonicalization "$KIT_ROOT/prototype/vnext/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-schema "$KIT_ROOT/prototype/vnext/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-rules "$KIT_ROOT/prototype/vnext/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-detection "$KIT_ROOT/prototype/vnext/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-kernel "$KIT_ROOT/prototype/vnext/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-context "$KIT_ROOT/prototype/vnext/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-project "$KIT_ROOT/prototype/vnext/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-lock "$KIT_ROOT/prototype/vnext/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-submission "$KIT_ROOT/prototype/vnext/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-application "$KIT_ROOT/prototype/vnext/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-store "$KIT_ROOT/prototype/vnext/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-persistent "$KIT_ROOT/prototype/vnext/golden/v1"
cargo run \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --locked \
  --quiet \
  -- verify-explain "$KIT_ROOT/prototype/vnext/golden/v1"

sh "$KIT_ROOT/tests/test-vnext-release-ci.sh"
sh "$KIT_ROOT/tests/test-vnext-release-publication.sh"
sh "$KIT_ROOT/tests/test-vnext-bootstrap.sh"
