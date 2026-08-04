#!/bin/sh

set -eu

if [ "$#" -ne 2 ]; then
  echo "usage: verify-release-binaries.sh <binary-dir> <source-revision>" >&2
  exit 2
fi

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
BINARY_DIR=$1
SOURCE_REVISION=$2
METADATA=$(python3 "$SCRIPT_DIR/verify-release-binaries.py" \
  "$BINARY_DIR" "$SOURCE_REVISION")

if [ "${ADF_RELEASE_REQUIRE_ATTESTATIONS:-0}" = "1" ]; then
  REPOSITORY=${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}
  DEFAULT_BRANCH=${ADF_RELEASE_DEFAULT_BRANCH:?ADF_RELEASE_DEFAULT_BRANCH is required}
  GH_CLI=${ADF_GH_CLI:-gh}
  SIGNER_WORKFLOW=$REPOSITORY/.github/workflows/release.yml
  for binary in \
    adf-aarch64-apple-darwin \
    adf-aarch64-unknown-linux-gnu \
    adf-x86_64-apple-darwin \
    adf-x86_64-pc-windows-msvc.exe \
    adf-x86_64-unknown-linux-gnu
  do
    "$GH_CLI" attestation verify "$BINARY_DIR/$binary" \
      --repo "$REPOSITORY" \
      --signer-workflow "$SIGNER_WORKFLOW" \
      --source-digest "$SOURCE_REVISION" \
      --source-ref "refs/heads/$DEFAULT_BRANCH" \
      --deny-self-hosted-runners >/dev/null
  done
elif [ "${ADF_RELEASE_REQUIRE_ATTESTATIONS:-0}" != "0" ]; then
  echo "ADF_RELEASE_REQUIRE_ATTESTATIONS must be 0 or 1" >&2
  exit 2
fi

printf '%s\n' "$METADATA"
