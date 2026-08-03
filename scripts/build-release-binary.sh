#!/bin/sh

set -eu

if [ "$#" -ne 2 ]; then
  echo "usage: build-release-binary.sh <expected-target> <output-dir>" >&2
  exit 2
fi

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
KIT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
RUST_ROOT=$KIT_ROOT
EXPECTED_TARGET=$1
OUTPUT_DIR=$2
SOURCE_REVISION=${GITHUB_SHA:?GITHUB_SHA is required}

case "$EXPECTED_TARGET" in
  x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu|\
  x86_64-apple-darwin|aarch64-apple-darwin|x86_64-pc-windows-msvc)
    ;;
  *)
    echo "Unsupported Release binary target: $EXPECTED_TARGET" >&2
    exit 2
    ;;
esac
case "$SOURCE_REVISION" in
  *[!0-9a-f]*)
    echo "GITHUB_SHA must be a lowercase hexadecimal Git SHA" >&2
    exit 2
    ;;
esac
if [ "${#SOURCE_REVISION}" -ne 40 ]; then
  echo "GITHUB_SHA must contain 40 hexadecimal characters" >&2
  exit 2
fi

ACTUAL_TARGET=$(rustc -vV | sed -n 's/^host: //p')
if [ "$ACTUAL_TARGET" != "$EXPECTED_TARGET" ]; then
  echo "Runner Rust host is $ACTUAL_TARGET, expected $EXPECTED_TARGET" >&2
  exit 1
fi

AGENTIC_BUILD_SOURCE_REVISION=$SOURCE_REVISION cargo build \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --release \
  --locked

case "$EXPECTED_TARGET" in
  *-windows-msvc)
    SOURCE_BINARY=$RUST_ROOT/target/release/agentic.exe
    BINARY_NAME=agentic-$EXPECTED_TARGET.exe
    ;;
  *)
    SOURCE_BINARY=$RUST_ROOT/target/release/agentic
    BINARY_NAME=agentic-$EXPECTED_TARGET
    ;;
esac
if [ ! -f "$SOURCE_BINARY" ]; then
  echo "Built Release binary is missing: $SOURCE_BINARY" >&2
  exit 1
fi

mkdir -p "$OUTPUT_DIR"
cp "$SOURCE_BINARY" "$OUTPUT_DIR/$BINARY_NAME"
RUSTC_VERSION=$(rustc --version)
python3 - "$OUTPUT_DIR/$BINARY_NAME" "$EXPECTED_TARGET" \
  "$SOURCE_REVISION" "$RUSTC_VERSION" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

binary = Path(sys.argv[1])
record = {
    "schema_version": "1",
    "binary_name": binary.name,
    "target": sys.argv[2],
    "source_revision": sys.argv[3],
    "sha256": "sha256:" + hashlib.sha256(binary.read_bytes()).hexdigest(),
    "size": binary.stat().st_size,
    "rustc_version": sys.argv[4],
}
binary.with_name(binary.name + ".build.json").write_text(
    json.dumps(record, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY

echo "Release binary built: $OUTPUT_DIR/$BINARY_NAME"
