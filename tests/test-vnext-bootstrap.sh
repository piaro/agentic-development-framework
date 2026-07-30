#!/bin/sh

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
KIT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
BOOTSTRAP=$KIT_ROOT/prototype/vnext/bootstrap/install.sh
FAKE_GH=$KIT_ROOT/tests/fixtures/fake-gh-release.py
RUST_BINARY=$KIT_ROOT/prototype/vnext/rust/target/debug/agentic-vnext-rust
TEST_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/agentic-bootstrap-test.XXXXXX")
cleanup() {
  rm -rf "$TEST_ROOT"
}
trap cleanup EXIT HUP INT TERM

case "$(uname -s):$(uname -m)" in
  Linux:x86_64) TARGET=x86_64-unknown-linux-gnu ;;
  Linux:aarch64|Linux:arm64) TARGET=aarch64-unknown-linux-gnu ;;
  Darwin:x86_64) TARGET=x86_64-apple-darwin ;;
  Darwin:arm64|Darwin:aarch64) TARGET=aarch64-apple-darwin ;;
  *)
    echo "bootstrap test does not support this host" >&2
    exit 1
    ;;
esac
BINARY=agentic-vnext-rust-$TARGET
REPOSITORY=example/agentic-development-kit
STATE=$TEST_ROOT/github
INSTALL_ROOT=$TEST_ROOT/installed

create_release() {
  tag=$1
  revision=$2
  assets=$STATE/releases/$tag/assets
  mkdir -p "$assets"
  cp "$RUST_BINARY" "$assets/$BINARY"
  python3 - "$assets" "$BINARY" "$TARGET" "$tag" "$revision" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

root = Path(sys.argv[1])
binary_name, target, tag, revision = sys.argv[2:]
binary = root / binary_name

def digest(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()

build_name = binary_name + ".build.json"
build = {
    "schema_version": "1",
    "binary_name": binary_name,
    "target": target,
    "source_revision": revision,
    "sha256": digest(binary),
    "size": binary.stat().st_size,
    "rustc_version": "rustc 1.89.0 (bootstrap test fixture)",
}
(root / build_name).write_text(
    json.dumps(build, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
(root / "SHA256SUMS").write_text(
    f"{digest(binary).removeprefix('sha256:')}  {binary_name}\n",
    encoding="utf-8",
)
record = {
    "schema_version": "1",
    "release_id": tag.removeprefix("framework-"),
    "release_tag": tag,
    "source_revision": revision,
    "candidate_workflow_run_id": "12345",
    "source_id": "remote:test-fixture",
    "signer_key_id": "test.framework.release",
    "artifact_digest": "sha256:" + "a" * 64,
    "archive_digest": "sha256:" + "b" * 64,
    "signer_public_key": "c" * 64,
    "asset_digests": {
        "candidate-framework.lock": "sha256:" + "d" * 64,
        "framework-release.tar": "sha256:" + "e" * 64,
        "publish-receipt.json": "sha256:" + "f" * 64,
    },
    "binary_asset_digests": {
        binary_name: digest(binary),
        build_name: digest(root / build_name),
        "SHA256SUMS": digest(root / "SHA256SUMS"),
    },
}
(root / "publication-record.json").write_text(
    json.dumps(record, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY
  printf '%s\n' "$revision" >"$STATE/releases/$tag/target"
  printf 'published\n' >"$STATE/releases/$tag/state"
}

run_bootstrap() {
  tag=$1
  FAKE_GH_STATE=$STATE \
    FAKE_GH_DEFAULT_BRANCH=main \
    AGENTIC_GH_CLI=$FAKE_GH \
    sh "$BOOTSTRAP" \
      --repo "$REPOSITORY" \
      --tag "$tag" \
      --install-root "$INSTALL_ROOT"
}

TAG_ONE=framework-bootstrap-v1
REVISION_ONE=1111111111111111111111111111111111111111
create_release "$TAG_ONE" "$REVISION_ONE"
run_bootstrap "$TAG_ONE" >/dev/null
STATUS=$("$INSTALL_ROOT/bin/agentic" binary status \
  --install-root "$INSTALL_ROOT" \
  --format json)
printf '%s' "$STATUS" | python3 -c '
import json, sys
value = json.load(sys.stdin)
assert value["current"] == "framework-bootstrap-v1"
assert value["previous"] is None
'

TAG_TWO=framework-bootstrap-v2
REVISION_TWO=2222222222222222222222222222222222222222
create_release "$TAG_TWO" "$REVISION_TWO"
run_bootstrap "$TAG_TWO" >/dev/null
STATUS=$("$INSTALL_ROOT/bin/agentic" binary status \
  --install-root "$INSTALL_ROOT" \
  --format json)
printf '%s' "$STATUS" | python3 -c '
import json, sys
value = json.load(sys.stdin)
assert value["current"] == "framework-bootstrap-v2"
assert value["previous"] == "framework-bootstrap-v1"
'

"$INSTALL_ROOT/bin/agentic" binary rollback \
  --install-root "$INSTALL_ROOT" >/dev/null
STATUS=$("$INSTALL_ROOT/bin/agentic" binary status \
  --install-root "$INSTALL_ROOT" \
  --format json)
printf '%s' "$STATUS" | python3 -c '
import json, sys
value = json.load(sys.stdin)
assert value["current"] == "framework-bootstrap-v1"
assert value["previous"] == "framework-bootstrap-v2"
'

python3 - "$STATE/attestation-calls.jsonl" "$REPOSITORY" \
  "$REVISION_ONE" "$REVISION_TWO" <<'PY'
import json
import sys
from pathlib import Path

calls = [json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines()]
repository, first_revision, second_revision = sys.argv[2:]
assert len(calls) == 2
for call, revision in zip(calls, (first_revision, second_revision)):
    assert call[call.index("--repo") + 1] == repository
    assert call[call.index("--signer-workflow") + 1] == (
        repository + "/.github/workflows/vnext-release.yml"
    )
    assert call[call.index("--source-digest") + 1] == revision
    assert call[call.index("--source-ref") + 1] == "refs/heads/main"
    assert "--deny-self-hosted-runners" in call
PY

TAG_THREE=framework-bootstrap-v3
REVISION_THREE=3333333333333333333333333333333333333333
create_release "$TAG_THREE" "$REVISION_THREE"
if FAKE_GH_STATE=$STATE \
  FAKE_GH_FAIL_ATTESTATION=$BINARY \
  AGENTIC_GH_CLI=$FAKE_GH \
  sh "$BOOTSTRAP" \
    --repo "$REPOSITORY" \
    --tag "$TAG_THREE" \
    --install-root "$INSTALL_ROOT" >/dev/null 2>&1; then
  echo "bootstrap accepted a binary without valid provenance" >&2
  exit 1
fi
STATUS=$("$INSTALL_ROOT/bin/agentic" binary status \
  --install-root "$INSTALL_ROOT" \
  --format json)
printf '%s' "$STATUS" | python3 -c '
import json, sys
value = json.load(sys.stdin)
assert value["current"] == "framework-bootstrap-v1"
assert value["previous"] == "framework-bootstrap-v2"
'

TAG_DRAFT=framework-bootstrap-draft
REVISION_DRAFT=4444444444444444444444444444444444444444
create_release "$TAG_DRAFT" "$REVISION_DRAFT"
printf 'draft\n' >"$STATE/releases/$TAG_DRAFT/state"
if FAKE_GH_STATE=$STATE \
  AGENTIC_GH_CLI=$FAKE_GH \
  sh "$BOOTSTRAP" \
    --repo "$REPOSITORY" \
    --tag "$TAG_DRAFT" \
    --install-root "$INSTALL_ROOT" >/dev/null 2>&1; then
  echo "bootstrap accepted a draft GitHub Release" >&2
  exit 1
fi
test "$(wc -l <"$STATE/attestation-calls.jsonl" | tr -d ' ')" = "2"

echo "vNext bootstrap tests passed"
