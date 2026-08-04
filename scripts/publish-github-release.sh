#!/bin/sh

set -eu

if [ "$#" -ne 4 ]; then
  echo "usage: publish-github-release.sh <candidate-dir> <binary-dir> <release-tag> <source-revision>" >&2
  exit 2
fi

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
CANDIDATE_DIR=$1
BINARY_DIR=$2
RELEASE_TAG=$3
SOURCE_REVISION=$4
REPOSITORY=${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}
CANDIDATE_RUN_ID=${AGENTIC_RELEASE_CANDIDATE_RUN_ID:?AGENTIC_RELEASE_CANDIDATE_RUN_ID is required}
SOURCE_ID=${AGENTIC_RELEASE_SOURCE_ID:-remote:official}
SIGNER_KEY_ID=${AGENTIC_RELEASE_SIGNER_KEY_ID:-framework.release.prototype}
GH_CLI=${AGENTIC_GH_CLI:-gh}

case "$RELEASE_TAG" in
  ''|*[!A-Za-z0-9._-]*)
    echo "Release tag contains unsupported characters" >&2
    exit 2
    ;;
esac
case "$SOURCE_REVISION" in
  *[!0-9a-f]*)
    echo "Source revision must be a lowercase hexadecimal Git SHA" >&2
    exit 2
    ;;
esac
if [ "${#SOURCE_REVISION}" -ne 40 ]; then
  echo "Source revision must contain 40 hexadecimal characters" >&2
  exit 2
fi
case "$CANDIDATE_RUN_ID" in
  ''|*[!0-9]*)
    echo "Candidate workflow run ID must be numeric" >&2
    exit 2
    ;;
esac
case "$REPOSITORY" in
  */*) ;;
  *)
    echo "GITHUB_REPOSITORY must be owner/name" >&2
    exit 2
    ;;
esac
if ! command -v "$GH_CLI" >/dev/null 2>&1; then
  echo "GitHub CLI is required: $GH_CLI" >&2
  exit 2
fi

export AGENTIC_RELEASE_TAG=$RELEASE_TAG
export AGENTIC_RELEASE_SOURCE_REVISION=$SOURCE_REVISION
METADATA=$("$SCRIPT_DIR/verify-release-candidate.sh" "$CANDIDATE_DIR")
BINARY_METADATA=$("$SCRIPT_DIR/verify-release-binaries.sh" \
  "$BINARY_DIR" "$SOURCE_REVISION")

# Query matching refs rather than only Releases. Reusing a loose tag could
# silently point reviewed assets at an unrelated commit.
MATCHING_REFS=$("$GH_CLI" api \
  "repos/$REPOSITORY/git/matching-refs/tags/$RELEASE_TAG" \
  --jq 'length')
if [ "$MATCHING_REFS" != "0" ]; then
  echo "Release tag already exists and will not be reused: $RELEASE_TAG" >&2
  exit 1
fi

WORK_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/agentic-release-publish.XXXXXX")
cleanup() {
  rm -rf "$WORK_ROOT"
}
trap cleanup EXIT HUP INT TERM

PUBLICATION_RECORD=$WORK_ROOT/publication-record.json
RELEASE_NOTES=$WORK_ROOT/release-notes.md
python3 - "$METADATA" "$BINARY_METADATA" "$PUBLICATION_RECORD" "$RELEASE_NOTES" \
  "$RELEASE_TAG" "$SOURCE_REVISION" "$CANDIDATE_RUN_ID" \
  "$SOURCE_ID" "$SIGNER_KEY_ID" "$CANDIDATE_DIR" "$BINARY_DIR" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

metadata = json.loads(sys.argv[1])
binary_metadata = json.loads(sys.argv[2])
record_path = Path(sys.argv[3])
notes_path = Path(sys.argv[4])
tag, revision, run_id, source_id, signer_key_id = sys.argv[5:10]
candidate = Path(sys.argv[10])
binaries = Path(sys.argv[11])

def digest(root: Path, name: str) -> str:
    return "sha256:" + hashlib.sha256((root / name).read_bytes()).hexdigest()

binary_names = [entry["name"] for entry in binary_metadata["binaries"]]
binary_assets = ["SHA256SUMS", "LICENSE-APACHE", "LICENSE-MIT", "THIRD-PARTY-NOTICES.md"]
for name in binary_names:
    binary_assets.extend([name, name + ".build.json"])

record = {
    "schema_version": "1",
    "release_id": metadata["release_id"],
    "release_tag": tag,
    "source_revision": revision,
    "candidate_workflow_run_id": run_id,
    "source_id": source_id,
    "signer_key_id": signer_key_id,
    "artifact_digest": metadata["artifact_digest"],
    "archive_digest": metadata["archive_digest"],
    "signer_public_key": metadata["signer_public_key"],
    "asset_digests": {
        "candidate-framework.lock": digest(candidate, "candidate-framework.lock"),
        "distribution-trust.json": digest(candidate, "distribution-trust.json"),
        "framework-release.tar": digest(candidate, "framework-release.tar"),
        "publish-receipt.json": digest(candidate, "publish-receipt.json"),
    },
    "binary_asset_digests": {
        name: digest(binaries, name) for name in binary_assets
    },
}
record_path.write_text(
    json.dumps(record, indent=2, sort_keys=True) + "\n", encoding="utf-8"
)
notes_path.write_text(
    "\n".join(
        [
            f"Framework Release `{metadata['release_id']}`.",
            "",
            f"- Candidate workflow run: `{run_id}`",
            f"- Source revision: `{revision}`",
            f"- Release source: `{source_id}`",
            f"- Signer key: `{signer_key_id}`",
            f"- Artifact digest: `{metadata['artifact_digest']}`",
            f"- Archive digest: `{metadata['archive_digest']}`",
            f"- Native binaries: `{binary_metadata['binary_count']}`",
            "",
            "Trust Store and active Framework lock are not changed by this release.",
            "",
        ]
    ),
    encoding="utf-8",
)
PY

# Create a non-public draft first. If upload or verification fails, the draft
# remains available for investigation and is never silently deleted.
"$GH_CLI" release create "$RELEASE_TAG" \
  "$CANDIDATE_DIR/framework-release.tar" \
  "$CANDIDATE_DIR/candidate-framework.lock" \
  "$CANDIDATE_DIR/distribution-trust.json" \
  "$CANDIDATE_DIR/publish-receipt.json" \
  "$BINARY_DIR/SHA256SUMS" \
  "$BINARY_DIR/LICENSE-APACHE" \
  "$BINARY_DIR/LICENSE-MIT" \
  "$BINARY_DIR/THIRD-PARTY-NOTICES.md" \
  "$BINARY_DIR/agentic-aarch64-apple-darwin" \
  "$BINARY_DIR/agentic-aarch64-apple-darwin.build.json" \
  "$BINARY_DIR/agentic-aarch64-unknown-linux-gnu" \
  "$BINARY_DIR/agentic-aarch64-unknown-linux-gnu.build.json" \
  "$BINARY_DIR/agentic-x86_64-apple-darwin" \
  "$BINARY_DIR/agentic-x86_64-apple-darwin.build.json" \
  "$BINARY_DIR/agentic-x86_64-pc-windows-msvc.exe" \
  "$BINARY_DIR/agentic-x86_64-pc-windows-msvc.exe.build.json" \
  "$BINARY_DIR/agentic-x86_64-unknown-linux-gnu" \
  "$BINARY_DIR/agentic-x86_64-unknown-linux-gnu.build.json" \
  "$PUBLICATION_RECORD" \
  --repo "$REPOSITORY" \
  --target "$SOURCE_REVISION" \
  --title "Framework Release $RELEASE_TAG" \
  --notes-file "$RELEASE_NOTES" \
  --draft \
  --latest=false

DOWNLOADED=$WORK_ROOT/downloaded
mkdir -p "$DOWNLOADED"
"$GH_CLI" release download "$RELEASE_TAG" \
  --repo "$REPOSITORY" \
  --dir "$DOWNLOADED" \
  --pattern framework-release.tar \
  --pattern candidate-framework.lock \
  --pattern distribution-trust.json \
  --pattern publish-receipt.json \
  --pattern SHA256SUMS \
  --pattern LICENSE-APACHE \
  --pattern LICENSE-MIT \
  --pattern THIRD-PARTY-NOTICES.md \
  --pattern agentic-aarch64-apple-darwin \
  --pattern agentic-aarch64-apple-darwin.build.json \
  --pattern agentic-aarch64-unknown-linux-gnu \
  --pattern agentic-aarch64-unknown-linux-gnu.build.json \
  --pattern agentic-x86_64-apple-darwin \
  --pattern agentic-x86_64-apple-darwin.build.json \
  --pattern agentic-x86_64-pc-windows-msvc.exe \
  --pattern agentic-x86_64-pc-windows-msvc.exe.build.json \
  --pattern agentic-x86_64-unknown-linux-gnu \
  --pattern agentic-x86_64-unknown-linux-gnu.build.json \
  --pattern publication-record.json

cmp "$CANDIDATE_DIR/framework-release.tar" "$DOWNLOADED/framework-release.tar"
cmp "$CANDIDATE_DIR/candidate-framework.lock" "$DOWNLOADED/candidate-framework.lock"
cmp "$CANDIDATE_DIR/distribution-trust.json" "$DOWNLOADED/distribution-trust.json"
cmp "$CANDIDATE_DIR/publish-receipt.json" "$DOWNLOADED/publish-receipt.json"
cmp "$PUBLICATION_RECORD" "$DOWNLOADED/publication-record.json"
for asset in \
  SHA256SUMS \
  LICENSE-APACHE \
  LICENSE-MIT \
  THIRD-PARTY-NOTICES.md \
  agentic-aarch64-apple-darwin \
  agentic-aarch64-apple-darwin.build.json \
  agentic-aarch64-unknown-linux-gnu \
  agentic-aarch64-unknown-linux-gnu.build.json \
  agentic-x86_64-apple-darwin \
  agentic-x86_64-apple-darwin.build.json \
  agentic-x86_64-pc-windows-msvc.exe \
  agentic-x86_64-pc-windows-msvc.exe.build.json \
  agentic-x86_64-unknown-linux-gnu \
  agentic-x86_64-unknown-linux-gnu.build.json
do
  cmp "$BINARY_DIR/$asset" "$DOWNLOADED/$asset"
done

DOWNLOADED_CANDIDATE=$WORK_ROOT/downloaded-candidate
mkdir -p "$DOWNLOADED_CANDIDATE"
cp "$DOWNLOADED/framework-release.tar" "$DOWNLOADED_CANDIDATE/framework-release.tar"
cp "$DOWNLOADED/candidate-framework.lock" "$DOWNLOADED_CANDIDATE/candidate-framework.lock"
cp "$DOWNLOADED/distribution-trust.json" "$DOWNLOADED_CANDIDATE/distribution-trust.json"
cp "$DOWNLOADED/publish-receipt.json" "$DOWNLOADED_CANDIDATE/publish-receipt.json"
"$SCRIPT_DIR/verify-release-candidate.sh" "$DOWNLOADED_CANDIDATE" >/dev/null
DOWNLOADED_BINARIES=$WORK_ROOT/downloaded-binaries
mkdir -p "$DOWNLOADED_BINARIES"
for asset in \
  SHA256SUMS \
  LICENSE-APACHE \
  LICENSE-MIT \
  THIRD-PARTY-NOTICES.md \
  agentic-aarch64-apple-darwin \
  agentic-aarch64-apple-darwin.build.json \
  agentic-aarch64-unknown-linux-gnu \
  agentic-aarch64-unknown-linux-gnu.build.json \
  agentic-x86_64-apple-darwin \
  agentic-x86_64-apple-darwin.build.json \
  agentic-x86_64-pc-windows-msvc.exe \
  agentic-x86_64-pc-windows-msvc.exe.build.json \
  agentic-x86_64-unknown-linux-gnu \
  agentic-x86_64-unknown-linux-gnu.build.json
do
  cp "$DOWNLOADED/$asset" "$DOWNLOADED_BINARIES/$asset"
done
"$SCRIPT_DIR/verify-release-binaries.sh" \
  "$DOWNLOADED_BINARIES" "$SOURCE_REVISION" >/dev/null

# Publishing is the final external state transition and happens only after the
# uploaded draft has been downloaded and revalidated byte-for-byte.
"$GH_CLI" release edit "$RELEASE_TAG" \
  --repo "$REPOSITORY" \
  --draft=false \
  --latest=false

echo "Framework Release $RELEASE_TAG published from candidate run $CANDIDATE_RUN_ID"
