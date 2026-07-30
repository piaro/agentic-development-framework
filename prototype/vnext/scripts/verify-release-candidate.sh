#!/bin/sh

set -eu

if [ "$#" -ne 1 ]; then
  echo "usage: verify-release-candidate.sh <candidate-dir>" >&2
  exit 2
fi

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
CANDIDATE_DIR=$1
PUBLIC_KEY=${AGENTIC_RELEASE_SIGNING_PUBLIC_KEY_HEX:?AGENTIC_RELEASE_SIGNING_PUBLIC_KEY_HEX is required}
EXPECTED_TAG=${AGENTIC_RELEASE_TAG:-}

if [ -n "$EXPECTED_TAG" ]; then
  METADATA=$(python3 "$SCRIPT_DIR/verify-release-candidate.py" \
    "$CANDIDATE_DIR" "$PUBLIC_KEY" "$EXPECTED_TAG")
else
  METADATA=$(python3 "$SCRIPT_DIR/verify-release-candidate.py" \
    "$CANDIDATE_DIR" "$PUBLIC_KEY")
fi

# The Rust verifier checks safe extraction, the Ed25519 signature, source and
# signer IDs, every file digest, Rule/Schema digests, and Framework protocols.
"$SCRIPT_DIR/verify-release-archive.sh" \
  "$CANDIDATE_DIR/framework-release.tar" \
  "$CANDIDATE_DIR/candidate-framework.lock" >/dev/null

printf '%s\n' "$METADATA"
