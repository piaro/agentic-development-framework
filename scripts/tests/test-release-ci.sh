#!/bin/sh

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
KIT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)
RELEASE_CI=$KIT_ROOT/scripts/release-ci.sh
TEST_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/adf-release-ci-test.XXXXXX")
cleanup() {
  rm -rf "$TEST_ROOT"
}
trap cleanup EXIT HUP INT TERM

SEED=0707070707070707070707070707070707070707070707070707070707070707
PUBLIC_KEY=ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c

run_release_ci() {
  output=$1
  public_key=$2
  ADF_RELEASE_SIGNING_KEY_HEX=$SEED \
    ADF_RELEASE_SIGNING_PUBLIC_KEY_HEX=$public_key \
    ADF_RELEASE_SOURCE_ID=remote:test-fixture \
    ADF_RELEASE_SIGNER_KEY_ID=test.framework.release \
    ADF_RELEASE_OUTPUT_DIR=$output \
    sh "$RELEASE_CI"
}

OUTPUT=$TEST_ROOT/output
run_release_ci "$OUTPUT" "$PUBLIC_KEY"
test -s "$OUTPUT/framework-release.tar"
test -s "$OUTPUT/candidate-framework.lock"
test -s "$OUTPUT/distribution-trust.json"
test -s "$OUTPUT/publish-receipt.json"

# A rerun must not replace an already reviewed candidate.
if run_release_ci "$OUTPUT" "$PUBLIC_KEY" >/dev/null 2>&1; then
  echo "Release CI unexpectedly overwrote existing outputs" >&2
  exit 1
fi

# A valid but different public-key pin must reject the signing secret before
# publishing any final artifact.
WRONG_OUTPUT=$TEST_ROOT/wrong-key
if run_release_ci "$WRONG_OUTPUT" \
  0000000000000000000000000000000000000000000000000000000000000000 \
  >/dev/null 2>&1; then
  echo "Release CI accepted a mismatched signer public key" >&2
  exit 1
fi
test ! -e "$WRONG_OUTPUT/framework-release.tar"
test ! -e "$WRONG_OUTPUT/candidate-framework.lock"
test ! -e "$WRONG_OUTPUT/distribution-trust.json"
test ! -e "$WRONG_OUTPUT/publish-receipt.json"

# The CI secret is mandatory and must not silently fall back to a development
# key or an unsigned artifact.
MISSING_OUTPUT=$TEST_ROOT/missing-secret
if (
  unset ADF_RELEASE_SIGNING_KEY_HEX
  ADF_RELEASE_SIGNING_PUBLIC_KEY_HEX=$PUBLIC_KEY \
    ADF_RELEASE_OUTPUT_DIR=$MISSING_OUTPUT \
    sh "$RELEASE_CI"
) >/dev/null 2>&1; then
  echo "Release CI accepted a missing signing secret" >&2
  exit 1
fi
test ! -e "$MISSING_OUTPUT/framework-release.tar"

echo "Release CI tests passed"
