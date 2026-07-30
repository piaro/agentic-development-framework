#!/bin/sh

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
KIT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../../.." && pwd)
RUST_ROOT=$KIT_ROOT/prototype/vnext/rust
SOURCE_RULES=$KIT_ROOT/prototype/vnext/fixtures/db-sqs/rules.yaml
SOURCE_SCHEMAS=$KIT_ROOT/prototype/vnext/schemas/v1
BASE_LOCK=$KIT_ROOT/prototype/vnext/fixtures/db-sqs/framework-lock.yaml
OUTPUT_DIR=${AGENTIC_RELEASE_OUTPUT_DIR:-"$KIT_ROOT/dist/vnext"}
SOURCE_ID=${AGENTIC_RELEASE_SOURCE_ID:-remote:official}
SIGNER_KEY_ID=${AGENTIC_RELEASE_SIGNER_KEY_ID:-framework.release.prototype}
PUBLIC_KEY=${AGENTIC_RELEASE_SIGNING_PUBLIC_KEY_HEX:?AGENTIC_RELEASE_SIGNING_PUBLIC_KEY_HEX is required}
: "${AGENTIC_RELEASE_SIGNING_KEY_HEX:?AGENTIC_RELEASE_SIGNING_KEY_HEX is required}"

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required to build the vNext Release Publisher" >&2
  exit 2
fi
if [ -e "$OUTPUT_DIR/framework-release.tar" ] ||
  [ -e "$OUTPUT_DIR/candidate-framework.lock" ] ||
  [ -e "$OUTPUT_DIR/publish-receipt.json" ]; then
  echo "Release CI outputs already exist in $OUTPUT_DIR" >&2
  exit 2
fi

WORK_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/agentic-release-ci.XXXXXX")
cleanup() {
  rm -rf "$WORK_ROOT"
}
trap cleanup EXIT HUP INT TERM

mkdir -p "$WORK_ROOT/source/schemas" "$WORK_ROOT/first" "$WORK_ROOT/second"
cp "$SOURCE_RULES" "$WORK_ROOT/source/rules.yaml"
cp -R "$SOURCE_SCHEMAS" "$WORK_ROOT/source/schemas/v1"

cargo build \
  --manifest-path "$RUST_ROOT/Cargo.toml" \
  --release \
  --locked
BINARY=$RUST_ROOT/target/release/agentic-vnext-rust

build_release() {
  destination=$1
  "$BINARY" release build "$WORK_ROOT/source" \
    --lock "$BASE_LOCK" \
    --source-id "$SOURCE_ID" \
    --key-id "$SIGNER_KEY_ID" \
    --expected-public-key "$PUBLIC_KEY" \
    --output "$destination/framework-release.tar" \
    --lock-output "$destination/candidate-framework.lock" \
    --format json >"$destination/publish-receipt.json"
}

# Two independent builds must be byte-for-byte identical. This catches
# accidental timestamps, host metadata, ordering, and nondeterministic signing.
build_release "$WORK_ROOT/first"
build_release "$WORK_ROOT/second"
cmp "$WORK_ROOT/first/framework-release.tar" "$WORK_ROOT/second/framework-release.tar"
cmp "$WORK_ROOT/first/candidate-framework.lock" "$WORK_ROOT/second/candidate-framework.lock"

export AGENTIC_RELEASE_CI_BINARY=$BINARY
export AGENTIC_RELEASE_SOURCE_ID=$SOURCE_ID
export AGENTIC_RELEASE_SIGNER_KEY_ID=$SIGNER_KEY_ID
"$SCRIPT_DIR/verify-release-archive.sh" \
  "$WORK_ROOT/first/framework-release.tar" \
  "$WORK_ROOT/first/candidate-framework.lock"
"$SCRIPT_DIR/verify-release-archive.sh" \
  "$WORK_ROOT/second/framework-release.tar" \
  "$WORK_ROOT/second/candidate-framework.lock"

# Build once more at the final paths so the receipt contains paths that remain
# valid in the uploaded CI artifact. Existing files are never overwritten.
mkdir -p "$OUTPUT_DIR"
build_release "$OUTPUT_DIR"
cmp "$WORK_ROOT/first/framework-release.tar" "$OUTPUT_DIR/framework-release.tar"
cmp "$WORK_ROOT/first/candidate-framework.lock" "$OUTPUT_DIR/candidate-framework.lock"
"$SCRIPT_DIR/verify-release-archive.sh" \
  "$OUTPUT_DIR/framework-release.tar" \
  "$OUTPUT_DIR/candidate-framework.lock"

echo "Release CI artifact is ready at $OUTPUT_DIR"
