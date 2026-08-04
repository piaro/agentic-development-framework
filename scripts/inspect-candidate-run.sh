#!/bin/sh

set -eu

if [ "$#" -ne 1 ]; then
  echo "usage: inspect-candidate-run.sh <workflow-run-id>" >&2
  exit 2
fi

RUN_ID=$1
REPOSITORY=${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}
DEFAULT_BRANCH=${ADF_RELEASE_DEFAULT_BRANCH:?ADF_RELEASE_DEFAULT_BRANCH is required}
GH_CLI=${ADF_GH_CLI:-gh}

case "$RUN_ID" in
  ''|*[!0-9]*)
    echo "Candidate workflow run ID must be numeric" >&2
    exit 2
    ;;
esac

RUN_JSON=$("$GH_CLI" api "repos/$REPOSITORY/actions/runs/$RUN_ID")
printf '%s' "$RUN_JSON" | python3 -c '
import json
import sys

repository, default_branch = sys.argv[1:3]
run = json.load(sys.stdin)
checks = {
    "workflow path": str(run.get("path", "")).split("@", 1)[0]
        == ".github/workflows/release.yml",
    "event": run.get("event") == "workflow_dispatch",
    "status": run.get("status") == "completed",
    "conclusion": run.get("conclusion") == "success",
    "source repository": (run.get("head_repository") or {}).get("full_name")
        == repository,
    "source branch": run.get("head_branch") == default_branch,
}
failed = [label for label, valid in checks.items() if not valid]
if failed:
    raise SystemExit("Candidate workflow run rejected: " + ", ".join(failed))
revision = run.get("head_sha")
if not isinstance(revision, str) or len(revision) != 40:
    raise SystemExit("Candidate workflow run has an invalid source revision")
try:
    int(revision, 16)
except ValueError:
    raise SystemExit("Candidate workflow run has an invalid source revision")
print(revision)
' "$REPOSITORY" "$DEFAULT_BRANCH"
