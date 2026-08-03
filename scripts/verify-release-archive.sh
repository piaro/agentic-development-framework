#!/bin/sh

set -eu

if [ "$#" -ne 2 ]; then
  echo "usage: verify-release-archive.sh <release.tar> <candidate-framework.lock>" >&2
  exit 2
fi

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
KIT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
CALLER_ROOT=$(pwd)
BINARY=${AGENTIC_RELEASE_CI_BINARY:-"$KIT_ROOT/target/release/agentic"}
PUBLIC_KEY=${AGENTIC_RELEASE_SIGNING_PUBLIC_KEY_HEX:?AGENTIC_RELEASE_SIGNING_PUBLIC_KEY_HEX is required}
SOURCE_ID=${AGENTIC_RELEASE_SOURCE_ID:-remote:official}
SIGNER_KEY_ID=${AGENTIC_RELEASE_SIGNER_KEY_ID:-framework.release.prototype}

# These values are inserted into a temporary YAML trust store. Restricting
# their alphabet keeps that mechanical serialization unambiguous.
case "$SOURCE_ID" in
  ''|*[!A-Za-z0-9._:-]*)
    echo "AGENTIC_RELEASE_SOURCE_ID contains unsupported characters" >&2
    exit 2
    ;;
esac
case "$SIGNER_KEY_ID" in
  ''|*[!A-Za-z0-9._:-]*)
    echo "AGENTIC_RELEASE_SIGNER_KEY_ID contains unsupported characters" >&2
    exit 2
    ;;
esac
case "$PUBLIC_KEY" in
  *[!A-Fa-f0-9]*)
    echo "AGENTIC_RELEASE_SIGNING_PUBLIC_KEY_HEX must be hexadecimal" >&2
    exit 2
    ;;
esac
if [ "${#PUBLIC_KEY}" -ne 64 ]; then
  echo "AGENTIC_RELEASE_SIGNING_PUBLIC_KEY_HEX must contain 64 hexadecimal characters" >&2
  exit 2
fi
if [ ! -x "$BINARY" ]; then
  echo "Release CI binary is not executable: $BINARY" >&2
  exit 2
fi

case "$1" in
  /*) ARCHIVE=$1 ;;
  *) ARCHIVE=$CALLER_ROOT/$1 ;;
esac
case "$2" in
  /*) CANDIDATE_LOCK=$2 ;;
  *) CANDIDATE_LOCK=$CALLER_ROOT/$2 ;;
esac

VERIFY_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/agentic-release-verify.XXXXXX")
cleanup() {
  rm -rf "$VERIFY_ROOT"
}
trap cleanup EXIT HUP INT TERM

mkdir -p "$VERIFY_ROOT/project/.agentic"
{
  echo 'schema_version: "2"'
  echo 'keys:'
  echo "  - id: $SIGNER_KEY_ID"
  echo '    algorithm: ed25519'
  echo "    public_key: $PUBLIC_KEY"
  echo '    allowed_sources:'
  echo "      - $SOURCE_ID"
  echo '    status: active'
} >"$VERIFY_ROOT/project/.agentic/trusted-release-keys.yaml"

# install-archive owns path traversal checks, extraction limits, signature
# verification, inventory digests, and Rule/Schema compatibility validation.
"$BINARY" release install-archive "$ARCHIVE" \
  --lock "$CANDIDATE_LOCK" \
  --project "$VERIFY_ROOT/project"
