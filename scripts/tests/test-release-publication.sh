#!/bin/sh

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
KIT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)
RELEASE_CI=$KIT_ROOT/scripts/release-ci.sh
VERIFY_CANDIDATE=$KIT_ROOT/scripts/verify-release-candidate.sh
PUBLISH=$KIT_ROOT/scripts/publish-github-release.sh
INSPECT=$KIT_ROOT/scripts/inspect-candidate-run.sh
FAKE_GH=$KIT_ROOT/scripts/tests/fixtures/fake-gh-release.py
TEST_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/adf-release-publication-test.XXXXXX")
cleanup() {
  rm -rf "$TEST_ROOT"
}
trap cleanup EXIT HUP INT TERM

SEED=0707070707070707070707070707070707070707070707070707070707070707
PUBLIC_KEY=ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c
SOURCE_ID=remote:test-fixture
SIGNER_KEY_ID=test.framework.release
SOURCE_REVISION=1111111111111111111111111111111111111111
RELEASE_TAG=framework-adf-dev
CANDIDATE=$TEST_ROOT/candidate
BINARIES=$TEST_ROOT/binaries
BINARY=$KIT_ROOT/target/release/adf

ADF_RELEASE_SIGNING_KEY_HEX=$SEED \
  ADF_RELEASE_SIGNING_PUBLIC_KEY_HEX=$PUBLIC_KEY \
  ADF_RELEASE_SOURCE_ID=$SOURCE_ID \
  ADF_RELEASE_SIGNER_KEY_ID=$SIGNER_KEY_ID \
  ADF_RELEASE_OUTPUT_DIR=$CANDIDATE \
  sh "$RELEASE_CI" >/dev/null

python3 - "$BINARIES" "$SOURCE_REVISION" "$KIT_ROOT" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

root = Path(sys.argv[1])
revision = sys.argv[2]
root.mkdir()
targets = (
    "aarch64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
)
checksums = []
for target in targets:
    suffix = ".exe" if target.endswith("-windows-msvc") else ""
    name = f"adf-{target}{suffix}"
    path = root / name
    path.write_bytes(f"test binary for {target}\n".encode())
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    checksums.append(f"{digest}  {name}")
    record = {
        "schema_version": "1",
        "binary_name": name,
        "target": target,
        "source_revision": revision,
        "sha256": f"sha256:{digest}",
        "size": path.stat().st_size,
        "rustc_version": "rustc 1.89.0 (test fixture)",
    }
    (root / f"{name}.build.json").write_text(
        json.dumps(record, indent=2, sort_keys=True) + "\n"
    )

# The published set carries the license terms of everything linked into the
# binaries, so the fixture has to carry them too.
kit_root = Path(sys.argv[3])
for name in ("LICENSE-APACHE", "LICENSE-MIT"):
    (root / name).write_bytes((kit_root / name).read_bytes())
(root / "THIRD-PARTY-NOTICES.md").write_text(
    "# Third-party notices\n\ntest fixture\n"
)
for name in ("LICENSE-APACHE", "LICENSE-MIT", "THIRD-PARTY-NOTICES.md"):
    digest = hashlib.sha256((root / name).read_bytes()).hexdigest()
    checksums.append(f"{digest}  {name}")

(root / "SHA256SUMS").write_text("\n".join(checksums) + "\n")
PY

python3 "$KIT_ROOT/scripts/verify-release-binaries.py" \
  "$BINARIES" "$SOURCE_REVISION" >/dev/null

TAMPERED_BINARIES=$TEST_ROOT/tampered-binaries
cp -R "$BINARIES" "$TAMPERED_BINARIES"
printf 'tampered' >>"$TAMPERED_BINARIES/adf-aarch64-apple-darwin"
if python3 "$KIT_ROOT/scripts/verify-release-binaries.py" \
  "$TAMPERED_BINARIES" "$SOURCE_REVISION" >/dev/null 2>&1; then
  echo "Binary verifier accepted bytes that differ from the Build Record" >&2
  exit 1
fi

NATIVE_TARGET=$(rustc -vV | sed -n 's/^host: //p')
NATIVE_OUTPUT=$TEST_ROOT/native-binary
GITHUB_SHA=$SOURCE_REVISION \
  sh "$KIT_ROOT/scripts/build-release-binary.sh" \
  "$NATIVE_TARGET" "$NATIVE_OUTPUT" >/dev/null
case "$NATIVE_TARGET" in
  *-windows-msvc) NATIVE_SUFFIX=.exe ;;
  *) NATIVE_SUFFIX= ;;
esac
test -s "$NATIVE_OUTPUT/adf-$NATIVE_TARGET$NATIVE_SUFFIX"
test -s "$NATIVE_OUTPUT/adf-$NATIVE_TARGET$NATIVE_SUFFIX.build.json"
"$NATIVE_OUTPUT/adf-$NATIVE_TARGET$NATIVE_SUFFIX" --version |
  grep -q "$SOURCE_REVISION"

verify_candidate() {
  ADF_RELEASE_CI_BINARY=$BINARY \
    ADF_RELEASE_SIGNING_PUBLIC_KEY_HEX=$PUBLIC_KEY \
    ADF_RELEASE_SOURCE_ID=$SOURCE_ID \
    ADF_RELEASE_SIGNER_KEY_ID=$SIGNER_KEY_ID \
    ADF_RELEASE_TAG=$RELEASE_TAG \
    sh "$VERIFY_CANDIDATE" "$1"
}

verify_candidate "$CANDIDATE" >/dev/null

EXTRA=$TEST_ROOT/extra
cp -R "$CANDIDATE" "$EXTRA"
echo "not part of the candidate" >"$EXTRA/unexpected.txt"
if verify_candidate "$EXTRA" >/dev/null 2>&1; then
  echo "Candidate verifier accepted an unexpected file" >&2
  exit 1
fi

TAMPERED_RECEIPT=$TEST_ROOT/tampered-receipt
cp -R "$CANDIDATE" "$TAMPERED_RECEIPT"
python3 - "$TAMPERED_RECEIPT/publish-receipt.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
value = json.loads(path.read_text())
value["archive_digest"] = "sha256:" + "0" * 64
path.write_text(json.dumps(value))
PY
if verify_candidate "$TAMPERED_RECEIPT" >/dev/null 2>&1; then
  echo "Candidate verifier accepted a false archive digest" >&2
  exit 1
fi

TAMPERED_TRUST=$TEST_ROOT/tampered-trust
cp -R "$CANDIDATE" "$TAMPERED_TRUST"
python3 - "$TAMPERED_TRUST/distribution-trust.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
value = json.loads(path.read_text())
value["keys"][0]["status"] = "revoked"
path.write_text(json.dumps(value))
PY
if verify_candidate "$TAMPERED_TRUST" >/dev/null 2>&1; then
  echo "Candidate verifier accepted a revoked bootstrap trust key" >&2
  exit 1
fi

RUN_JSON=$(python3 - "$SOURCE_REVISION" <<'PY'
import json
import sys

print(json.dumps({
    "path": ".github/workflows/release.yml",
    "event": "workflow_dispatch",
    "status": "completed",
    "conclusion": "success",
    "head_repository": {"full_name": "example/agentic-development-framework"},
    "head_branch": "main",
    "head_sha": sys.argv[1],
}))
PY
)
INSPECTED=$(FAKE_GH_STATE=$TEST_ROOT/inspect-state \
  FAKE_GH_RUN_JSON=$RUN_JSON \
  ADF_GH_CLI=$FAKE_GH \
  GITHUB_REPOSITORY=example/agentic-development-framework \
  ADF_RELEASE_DEFAULT_BRANCH=main \
  sh "$INSPECT" 12345)
test "$INSPECTED" = "$SOURCE_REVISION"

FAILED_RUN_JSON=$(printf '%s' "$RUN_JSON" | python3 -c '
import json, sys
value = json.load(sys.stdin)
value["conclusion"] = "failure"
print(json.dumps(value))
')
if FAKE_GH_STATE=$TEST_ROOT/inspect-failed-state \
  FAKE_GH_RUN_JSON=$FAILED_RUN_JSON \
  ADF_GH_CLI=$FAKE_GH \
  GITHUB_REPOSITORY=example/agentic-development-framework \
  ADF_RELEASE_DEFAULT_BRANCH=main \
  sh "$INSPECT" 12345 >/dev/null 2>&1; then
  echo "Candidate run inspector accepted a failed workflow" >&2
  exit 1
fi

publish_with_state() {
  state=$1
  ADF_RELEASE_CI_BINARY=$BINARY \
    ADF_RELEASE_SIGNING_PUBLIC_KEY_HEX=$PUBLIC_KEY \
    ADF_RELEASE_SOURCE_ID=$SOURCE_ID \
    ADF_RELEASE_SIGNER_KEY_ID=$SIGNER_KEY_ID \
    ADF_RELEASE_CANDIDATE_RUN_ID=12345 \
    ADF_RELEASE_DEFAULT_BRANCH=main \
    ADF_RELEASE_REQUIRE_ATTESTATIONS=1 \
    ADF_GH_CLI=$FAKE_GH \
    FAKE_GH_STATE=$state \
    GITHUB_REPOSITORY=example/agentic-development-framework \
    sh "$PUBLISH" "$CANDIDATE" "$BINARIES" "$RELEASE_TAG" "$SOURCE_REVISION"
}

PUBLISHED_STATE=$TEST_ROOT/published-state
publish_with_state "$PUBLISHED_STATE" >/dev/null
test "$(cat "$PUBLISHED_STATE/releases/$RELEASE_TAG/state")" = "published"
test -s "$PUBLISHED_STATE/releases/$RELEASE_TAG/assets/framework-release.tar"
test -s "$PUBLISHED_STATE/releases/$RELEASE_TAG/assets/candidate-framework.lock"
test -s "$PUBLISHED_STATE/releases/$RELEASE_TAG/assets/distribution-trust.json"
test -s "$PUBLISHED_STATE/releases/$RELEASE_TAG/assets/publish-receipt.json"
test -s "$PUBLISHED_STATE/releases/$RELEASE_TAG/assets/publication-record.json"
test -s "$PUBLISHED_STATE/releases/$RELEASE_TAG/assets/SHA256SUMS"
test -s "$PUBLISHED_STATE/releases/$RELEASE_TAG/assets/adf-x86_64-unknown-linux-gnu"
# Statically linked dependencies require their terms to be published alongside.
test -s "$PUBLISHED_STATE/releases/$RELEASE_TAG/assets/LICENSE-APACHE"
test -s "$PUBLISHED_STATE/releases/$RELEASE_TAG/assets/LICENSE-MIT"
test -s "$PUBLISHED_STATE/releases/$RELEASE_TAG/assets/THIRD-PARTY-NOTICES.md"

if publish_with_state "$PUBLISHED_STATE" >/dev/null 2>&1; then
  echo "Publication unexpectedly reused an existing Release tag" >&2
  exit 1
fi

TAMPERED_STATE=$TEST_ROOT/tampered-upload-state
if FAKE_GH_TAMPER_ASSET=framework-release.tar \
  publish_with_state "$TAMPERED_STATE" >/dev/null 2>&1; then
  echo "Publication accepted a changed uploaded asset" >&2
  exit 1
fi
test "$(cat "$TAMPERED_STATE/releases/$RELEASE_TAG/state")" = "draft"

FAILED_ATTESTATION_STATE=$TEST_ROOT/failed-attestation-state
if FAKE_GH_FAIL_ATTESTATION=adf-x86_64-unknown-linux-gnu \
  publish_with_state "$FAILED_ATTESTATION_STATE" >/dev/null 2>&1; then
  echo "Publication accepted a binary without valid provenance" >&2
  exit 1
fi
test ! -e "$FAILED_ATTESTATION_STATE/releases"

FAILED_TRUST_ATTESTATION_STATE=$TEST_ROOT/failed-trust-attestation-state
if FAKE_GH_FAIL_ATTESTATION=distribution-trust.json \
  publish_with_state "$FAILED_TRUST_ATTESTATION_STATE" >/dev/null 2>&1; then
  echo "Publication accepted Distribution Trust without valid provenance" >&2
  exit 1
fi
test ! -e "$FAILED_TRUST_ATTESTATION_STATE/releases"

WRONG_TAG_STATE=$TEST_ROOT/wrong-tag-state
if ADF_RELEASE_CI_BINARY=$BINARY \
  ADF_RELEASE_SIGNING_PUBLIC_KEY_HEX=$PUBLIC_KEY \
  ADF_RELEASE_SOURCE_ID=$SOURCE_ID \
  ADF_RELEASE_SIGNER_KEY_ID=$SIGNER_KEY_ID \
  ADF_RELEASE_CANDIDATE_RUN_ID=12345 \
  ADF_RELEASE_DEFAULT_BRANCH=main \
  ADF_RELEASE_REQUIRE_ATTESTATIONS=1 \
  ADF_GH_CLI=$FAKE_GH \
  FAKE_GH_STATE=$WRONG_TAG_STATE \
  GITHUB_REPOSITORY=example/agentic-development-framework \
  sh "$PUBLISH" "$CANDIDATE" "$BINARIES" framework-wrong "$SOURCE_REVISION" \
  >/dev/null 2>&1; then
  echo "Publication accepted a tag unrelated to the Release ID" >&2
  exit 1
fi
test ! -e "$WRONG_TAG_STATE/releases"

echo "Release publication tests passed"
