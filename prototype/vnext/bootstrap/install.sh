#!/bin/sh

# Bootstrap only a binary whose GitHub Artifact Attestation identifies the
# expected repository, build workflow, source revision, branch, and hosted
# runner. Checksums are still checked by the verified binary during install,
# but are not treated as an independent trust root.

set -eu
umask 077

REPOSITORY=piaro/agentic-development-kit
RELEASE_TAG=
INSTALL_ROOT=${AGENTIC_INSTALL_ROOT:-${XDG_DATA_HOME:-"$HOME/.local/share"}/agentic}
GH_CLI=${AGENTIC_GH_CLI:-gh}

usage() {
  echo "usage: install.sh --tag <framework-release-tag> [--repo <owner/name>] [--install-root <path>]" >&2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --repo)
      [ "$#" -ge 2 ] || {
        usage
        exit 2
      }
      REPOSITORY=$2
      shift 2
      ;;
    --tag)
      [ "$#" -ge 2 ] || {
        usage
        exit 2
      }
      RELEASE_TAG=$2
      shift 2
      ;;
    --install-root)
      [ "$#" -ge 2 ] || {
        usage
        exit 2
      }
      INSTALL_ROOT=$2
      shift 2
      ;;
    *)
      usage
      exit 2
      ;;
  esac
done

case "$REPOSITORY" in
  */*)
    case "$REPOSITORY" in
      */*/*)
        echo "--repo must use owner/name format" >&2
        exit 2
        ;;
    esac
    ;;
  *)
    echo "--repo must use owner/name format" >&2
    exit 2
    ;;
esac
case "$RELEASE_TAG" in
  framework-[A-Za-z0-9]*)
    case "$RELEASE_TAG" in
      *[!A-Za-z0-9._-]*)
        echo "--tag contains unsupported characters" >&2
        exit 2
        ;;
    esac
    ;;
  *)
    echo "--tag must start with framework-" >&2
    exit 2
    ;;
esac
case "$INSTALL_ROOT" in
  ''|/)
    echo "--install-root must identify a dedicated directory" >&2
    exit 2
    ;;
esac
if ! command -v "$GH_CLI" >/dev/null 2>&1; then
  echo "GitHub CLI is required to verify binary provenance: $GH_CLI" >&2
  exit 2
fi

case "$(uname -s):$(uname -m)" in
  Linux:x86_64) TARGET=x86_64-unknown-linux-gnu ;;
  Linux:aarch64|Linux:arm64) TARGET=aarch64-unknown-linux-gnu ;;
  Darwin:x86_64) TARGET=x86_64-apple-darwin ;;
  Darwin:arm64|Darwin:aarch64) TARGET=aarch64-apple-darwin ;;
  *)
    echo "No published Agentic binary is available for $(uname -s)/$(uname -m)" >&2
    exit 1
    ;;
esac
BINARY=agentic-vnext-rust-$TARGET
BUILD_RECORD=$BINARY.build.json

SOURCE_REVISION=$("$GH_CLI" release view "$RELEASE_TAG" \
  --repo "$REPOSITORY" \
  --json targetCommitish \
  --jq '.targetCommitish')
case "$SOURCE_REVISION" in
  *[!0-9a-f]*)
    echo "Release target is not a lowercase Git commit SHA" >&2
    exit 1
    ;;
esac
if [ "${#SOURCE_REVISION}" -ne 40 ]; then
  echo "Release target is not a 40-character Git commit SHA" >&2
  exit 1
fi
IS_DRAFT=$("$GH_CLI" release view "$RELEASE_TAG" \
  --repo "$REPOSITORY" \
  --json isDraft \
  --jq '.isDraft')
if [ "$IS_DRAFT" != "false" ]; then
  echo "Refusing to install a draft GitHub Release" >&2
  exit 1
fi
DEFAULT_BRANCH=$("$GH_CLI" repo view "$REPOSITORY" \
  --json defaultBranchRef \
  --jq '.defaultBranchRef.name')
case "$DEFAULT_BRANCH" in
  ''|*[!A-Za-z0-9._/-]*)
    echo "Repository returned an invalid default branch" >&2
    exit 1
    ;;
esac

STAGING=$(mktemp -d "${TMPDIR:-/tmp}/agentic-bootstrap.XXXXXX")
cleanup() {
  rm -rf "$STAGING"
}
trap cleanup EXIT HUP INT TERM

"$GH_CLI" release download "$RELEASE_TAG" \
  --repo "$REPOSITORY" \
  --dir "$STAGING" \
  --pattern "$BINARY" \
  --pattern "$BUILD_RECORD" \
  --pattern SHA256SUMS \
  --pattern publication-record.json

# Verify the executable before running it. Pinning the workflow and source
# revision prevents a checksum and binary replaced together from being trusted.
"$GH_CLI" attestation verify "$STAGING/$BINARY" \
  --repo "$REPOSITORY" \
  --signer-workflow "$REPOSITORY/.github/workflows/vnext-release.yml" \
  --source-digest "$SOURCE_REVISION" \
  --source-ref "refs/heads/$DEFAULT_BRANCH" \
  --deny-self-hosted-runners >/dev/null

chmod 755 "$STAGING/$BINARY"
"$STAGING/$BINARY" binary install "$STAGING" \
  --tag "$RELEASE_TAG" \
  --source-revision "$SOURCE_REVISION" \
  --install-root "$INSTALL_ROOT"

echo "Add $INSTALL_ROOT/bin to PATH to invoke agentic."
