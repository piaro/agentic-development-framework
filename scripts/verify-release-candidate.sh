#!/bin/sh

set -eu

if [ "$#" -ne 1 ]; then
  echo "usage: verify-release-candidate.sh <candidate-dir>" >&2
  exit 2
fi

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
CANDIDATE_DIR=$1
PUBLIC_KEY=${AGENTIC_RELEASE_SIGNING_PUBLIC_KEY_HEX:?AGENTIC_RELEASE_SIGNING_PUBLIC_KEY_HEX is required}
SOURCE_ID=${AGENTIC_RELEASE_SOURCE_ID:?AGENTIC_RELEASE_SOURCE_ID is required}
SIGNER_KEY_ID=${AGENTIC_RELEASE_SIGNER_KEY_ID:?AGENTIC_RELEASE_SIGNER_KEY_ID is required}
EXPECTED_TAG=${AGENTIC_RELEASE_TAG:-}

if [ -n "$EXPECTED_TAG" ]; then
  METADATA=$(python3 "$SCRIPT_DIR/verify-release-candidate.py" \
    "$CANDIDATE_DIR" "$PUBLIC_KEY" "$SOURCE_ID" "$SIGNER_KEY_ID" "$EXPECTED_TAG")
else
  METADATA=$(python3 "$SCRIPT_DIR/verify-release-candidate.py" \
    "$CANDIDATE_DIR" "$PUBLIC_KEY" "$SOURCE_ID" "$SIGNER_KEY_ID")
fi

# The Rust verifier checks safe extraction, the Ed25519 signature, source and
# signer IDs, every file digest, Rule/Schema digests, and Framework protocols.
"$SCRIPT_DIR/verify-release-archive.sh" \
  "$CANDIDATE_DIR/framework-release.tar" \
  "$CANDIDATE_DIR/candidate-framework.lock" >/dev/null

if [ "${AGENTIC_RELEASE_REQUIRE_ATTESTATIONS:-0}" = "1" ]; then
  REPOSITORY=${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}
  DEFAULT_BRANCH=${AGENTIC_RELEASE_DEFAULT_BRANCH:?AGENTIC_RELEASE_DEFAULT_BRANCH is required}
  SOURCE_REVISION=${AGENTIC_RELEASE_SOURCE_REVISION:?AGENTIC_RELEASE_SOURCE_REVISION is required}
  GH_CLI=${AGENTIC_GH_CLI:-gh}
  "$GH_CLI" attestation verify "$CANDIDATE_DIR/distribution-trust.json" \
    --repo "$REPOSITORY" \
    --signer-workflow "$REPOSITORY/.github/workflows/vnext-release.yml" \
    --source-digest "$SOURCE_REVISION" \
    --source-ref "refs/heads/$DEFAULT_BRANCH" \
    --deny-self-hosted-runners >/dev/null
elif [ "${AGENTIC_RELEASE_REQUIRE_ATTESTATIONS:-0}" != "0" ]; then
  echo "AGENTIC_RELEASE_REQUIRE_ATTESTATIONS must be 0 or 1" >&2
  exit 2
fi

printf '%s\n' "$METADATA"
