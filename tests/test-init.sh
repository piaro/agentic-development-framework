#!/bin/sh

set -eu

TEST_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
KIT_ROOT=$(CDPATH= cd -- "$TEST_DIR/.." && pwd)
TMP_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/agentic-development-kit.XXXXXX")

cleanup() {
  rm -rf "$TMP_ROOT"
}
trap cleanup EXIT HUP INT TERM

assert_file() {
  [ -f "$1" ] || {
    printf 'missing file: %s\n' "$1" >&2
    exit 1
  }
}

assert_not_exists() {
  [ ! -e "$1" ] || {
    printf 'unexpected path: %s\n' "$1" >&2
    exit 1
  }
}

NEW_TARGET="$TMP_ROOT/new-lite"
"$KIT_ROOT/bin/agentic-init" --mode new --level lite --target "$NEW_TARGET" --name demo --non-interactive >/dev/null
assert_file "$NEW_TARGET/.git/HEAD"
assert_file "$NEW_TARGET/contracts/project/constitution.yaml"
assert_file "$NEW_TARGET/contracts/_templates/feature.yaml"
assert_file "$NEW_TARGET/.agentic/bin/agentic"
assert_file "$NEW_TARGET/.agentic/schemas/contract-assessment.schema.json"
assert_file "$NEW_TARGET/.agentic/schemas/contract-challenge.schema.json"
assert_file "$NEW_TARGET/.agents/skills/agentic-change/SKILL.md"
assert_file "$NEW_TARGET/.agents/skills/agentic-contract/SKILL.md"
assert_file "$NEW_TARGET/.agents/skills/agentic-builder/SKILL.md"
assert_file "$NEW_TARGET/.agents/skills/agentic-challenger/SKILL.md"
assert_not_exists "$NEW_TARGET/contracts/data/invariants.yaml"
"$KIT_ROOT/bin/agentic-init" --upgrade --target "$NEW_TARGET" --non-interactive >/dev/null
assert_file "$NEW_TARGET/.git/HEAD"

ADOPT_TARGET="$TMP_ROOT/existing"
mkdir -p "$ADOPT_TARGET"
printf 'keep-me\n' > "$ADOPT_TARGET/README.md"
printf '# Existing rules\n' > "$ADOPT_TARGET/AGENTS.md"

"$KIT_ROOT/bin/agentic-init" --mode adopt --level standard --target "$ADOPT_TARGET" --non-interactive >/dev/null
assert_file "$ADOPT_TARGET/contracts/_templates/domain.yaml"
assert_file "$ADOPT_TARGET/contracts/data/invariants.yaml"
assert_file "$ADOPT_TARGET/contracts/_templates/operation.yaml"
assert_file "$ADOPT_TARGET/docs/agentic/source-of-truth.md"
assert_file "$ADOPT_TARGET/evidence/platform-capabilities.yaml"
assert_file "$ADOPT_TARGET/decisions/DECISION-TEMPLATE.md"
assert_file "$ADOPT_TARGET/tests/conformance/README.md"
assert_not_exists "$ADOPT_TARGET/contracts/capabilities/async-operation.yaml"
grep -q '^keep-me$' "$ADOPT_TARGET/README.md"

"$KIT_ROOT/bin/agentic-init" --mode adopt --level standard --target "$ADOPT_TARGET" --non-interactive >/dev/null
BLOCK_COUNT=$(grep -c '<!-- agentic-development:start -->' "$ADOPT_TARGET/AGENTS.md")
[ "$BLOCK_COUNT" -eq 1 ] || {
  printf 'AGENTS.md block duplicated\n' >&2
  exit 1
}

printf 'old skill\n' > "$ADOPT_TARGET/.agents/skills/agentic-development/SKILL.md"
"$KIT_ROOT/bin/agentic-init" --mode adopt --level standard --target "$ADOPT_TARGET" --non-interactive >/dev/null
grep -q '^old skill$' "$ADOPT_TARGET/.agents/skills/agentic-development/SKILL.md"
printf '\n# user-setting\n' >> "$ADOPT_TARGET/.agentic/config.yaml"
mkdir -p "$ADOPT_TARGET/.agentic/changes/legacy-change"
cat > "$ADOPT_TARGET/.agentic/changes/legacy-change/contract-assessment.yaml" <<'EOF'
schema_version: 1
change: legacy-change
legacy-user-content: keep-me
EOF
"$KIT_ROOT/bin/agentic-init" --target "$ADOPT_TARGET" --non-interactive --upgrade --dry-run >/dev/null
grep -q '^old skill$' "$ADOPT_TARGET/.agents/skills/agentic-development/SKILL.md"
"$KIT_ROOT/bin/agentic-init" --target "$ADOPT_TARGET" --non-interactive --upgrade >/dev/null
if grep -q '^old skill$' "$ADOPT_TARGET/.agents/skills/agentic-development/SKILL.md"; then
  printf 'managed skill was not upgraded\n' >&2
  exit 1
fi
grep -q 'Feature Contractを上位Contractの代用にしない' "$ADOPT_TARGET/AGENTS.md"
grep -q '^# user-setting$' "$ADOPT_TARGET/.agentic/config.yaml"
grep -q '^legacy-user-content: keep-me$' "$ADOPT_TARGET/.agentic/changes/legacy-change/contract-assessment.yaml"
"$ADOPT_TARGET/.agentic/bin/agentic" --version | grep -q '^agentic 3.0.0$'

LEGACY_TARGET="$TMP_ROOT/upgrade-v2.1"
mkdir -p "$LEGACY_TARGET"
"$KIT_ROOT/bin/agentic-init" --mode adopt --level lite --target "$LEGACY_TARGET" --name legacy --non-interactive >/dev/null
sed -i.bak 's/kit_version: "3.0.0"/kit_version: "2.1.0"/' "$LEGACY_TARGET/.agentic/installation.yaml"
rm "$LEGACY_TARGET/.agentic/installation.yaml.bak"
mkdir -p "$LEGACY_TARGET/contracts/features"
cp "$KIT_ROOT/tests/fixtures/upgrade-v2.1/legacy-feature.yaml" "$LEGACY_TARGET/contracts/features/legacy-feature.yaml"

"$KIT_ROOT/bin/agentic-init" --target "$LEGACY_TARGET" --non-interactive --upgrade >/dev/null
"$LEGACY_TARGET/.agentic/bin/agentic" --version | grep -q '^agentic 3.0.0$'
if "$LEGACY_TARGET/.agentic/bin/agentic" --root "$LEGACY_TARGET" contract lint >"$TMP_ROOT/legacy-lint.out" 2>"$TMP_ROOT/legacy-lint.err"; then
  printf '2.1形式のFeature Contractが手動移行なしでlintを通過しました\n' >&2
  exit 1
fi
grep -q 'introduced_decisionsは空でない文字列の配列である必要があります' "$TMP_ROOT/legacy-lint.err"
grep -q '^  - id: LEGACY-DEC-001$' "$LEGACY_TARGET/contracts/features/legacy-feature.yaml"

sed -i.bak '/^introduced_decisions:/,/^deviations:/c\
introduced_decisions: [LEGACY-DEC-001]\
deviations: []' "$LEGACY_TARGET/contracts/features/legacy-feature.yaml"
rm "$LEGACY_TARGET/contracts/features/legacy-feature.yaml.bak"
"$LEGACY_TARGET/.agentic/bin/agentic" --root "$LEGACY_TARGET" contract lint >/dev/null

"$KIT_ROOT/bin/agentic-init" --mode adopt --level system --target "$ADOPT_TARGET" --non-interactive >/dev/null
assert_file "$ADOPT_TARGET/contracts/capabilities/async-operation.yaml"
assert_file "$ADOPT_TARGET/tests/scenarios/README.md"
assert_file "$ADOPT_TARGET/probes/README.md"
grep -q '^level: "system"$' "$ADOPT_TARGET/.agentic/installation.yaml"

CRITICAL_TARGET="$TMP_ROOT/critical"
mkdir -p "$CRITICAL_TARGET"
"$KIT_ROOT/bin/agentic-init" --mode adopt --level critical --target "$CRITICAL_TARGET" --non-interactive >/dev/null
assert_file "$CRITICAL_TARGET/contracts/project/threat-model.yaml"
assert_file "$CRITICAL_TARGET/contracts/project/failure-model.yaml"
assert_file "$CRITICAL_TARGET/evidence/_templates/release-evidence.yaml"
assert_file "$CRITICAL_TARGET/tests/failure-injection/README.md"

DRY_TARGET="$TMP_ROOT/dry-run"
"$KIT_ROOT/bin/agentic-init" --mode new --level lite --target "$DRY_TARGET" --non-interactive --dry-run >/dev/null
assert_not_exists "$DRY_TARGET"

CLI_TARGET="$TMP_ROOT/cli"
mkdir -p "$CLI_TARGET"
"$KIT_ROOT/bin/agentic-init" --mode adopt --level standard --target "$CLI_TARGET" --name cli-demo --non-interactive >/dev/null

pass_contract_challenge() {
  change_id=$1
  challenge_path="$CLI_TARGET/.agentic/changes/$change_id/contract-challenge.yaml"
  "$CLI_TARGET/.agentic/bin/agentic" --root "$CLI_TARGET" contract challenge-input "$change_id" > "$challenge_path"
  sed -i.bak 's/challenger: TODO/challenger: pre-contract-challenger/' "$challenge_path"
  rm "$challenge_path.bak"
  sed -i.bak 's/independent_context: false/independent_context: true/' "$challenge_path"
  rm "$challenge_path.bak"
  sed -i.bak 's/result: pending/result: passed/' "$challenge_path"
  rm "$challenge_path.bak"
}

sed -i.bak 's/status: proposed/status: accepted/' "$CLI_TARGET/contracts/project/constitution.yaml"
rm "$CLI_TARGET/contracts/project/constitution.yaml.bak"
sed -i.bak 's/status: proposed/status: accepted/' "$CLI_TARGET/contracts/data/invariants.yaml"
rm "$CLI_TARGET/contracts/data/invariants.yaml.bak"
sed -i.bak 's/contexts: \[\]/contexts: [widgets]/' "$CLI_TARGET/contracts/data/invariants.yaml"
rm "$CLI_TARGET/contracts/data/invariants.yaml.bak"
sed -i.bak 's/entities: \[\]/entities: [Widget]/' "$CLI_TARGET/contracts/data/invariants.yaml"
rm "$CLI_TARGET/contracts/data/invariants.yaml.bak"

mkdir -p "$CLI_TARGET/contracts/operations"
cat > "$CLI_TARGET/contracts/operations/update-widget.yaml" <<'EOF'
schema_version: 2
id: operations/update-widget
kind: operation
status: accepted
version: 1
owners: [backend]
applies_to:
  contexts: [widgets]
  entities: [Widget]
  capabilities: []
  operations: [operations/update-widget]
  interfaces: []
sources: []
preconditions: [widget-exists]
reads: [Widget]
mutations:
  - entity: Widget
    action: update
transaction:
  atomic_groups:
    - [Widget]
external_effects: []
postconditions: [widget-updated]
consistency:
  database: strong
idempotency:
  key: request-id
  duplicate_semantics: return-existing
failure_points: [before-commit, after-commit]
conflicts_with: []
EOF

mkdir -p "$CLI_TARGET/contracts/capabilities"
cat > "$CLI_TARGET/contracts/capabilities/unrelated-suggestion.yaml" <<'EOF'
schema_version: 2
id: capabilities/unrelated-suggestion
kind: capability
status: accepted
version: 1
owners: [product]
applies_to:
  contexts: [widgets]
  entities: []
  capabilities: []
  operations: []
  interfaces: []
  paths: []
sources: []
outcome: unrelated
input_contract: []
output_contract: []
completion_semantics: unrelated
failure_semantics: []
compatibility: unrelated
reference_implementations: []
EOF

"$CLI_TARGET/.agentic/bin/agentic" --root "$CLI_TARGET" contract lint >/dev/null
"$CLI_TARGET/.agentic/bin/agentic" --root "$CLI_TARGET" change init update-widget --title "Update widget" >/dev/null
assert_file "$CLI_TARGET/.agentic/changes/update-widget/contract-challenge.yaml"
grep -q '^schema_version: 2$' "$CLI_TARGET/.agentic/changes/update-widget/contract-assessment.yaml"
grep -q '^  authorities_sha256: TODO$' "$CLI_TARGET/.agentic/changes/update-widget/contract-challenge.yaml"
if "$CLI_TARGET/.agentic/bin/agentic" --root "$CLI_TARGET" contract resolve update-widget >/dev/null 2>&1; then
  printf 'unresolved assessment was accepted\n' >&2
  exit 1
fi

cat > "$CLI_TARGET/contracts/features/update-widget.yaml" <<'EOF'
schema_version: 2
id: features/update-widget
kind: feature
status: accepted
version: 1
owners: [backend]
applies_to:
  contexts: [widgets]
  entities: [Widget]
  capabilities: []
  operations: [operations/update-widget]
  interfaces: []
sources: []
risk: R2
governing_contracts:
  - project/constitution
  - data/core-invariants
  - operations/update-widget
outcome: "Widgetを更新する"
non_scope: []
completion_semantics: "transaction commit"
failure_semantics: {}
introduced_decisions: []
deviations: []
unknowns: []
required_probes: []
evidence_requirements: [INV-TODO-001]
residual_risks: []
EOF

cat > "$CLI_TARGET/.agentic/changes/update-widget/change.yaml" <<'EOF'
schema_version: 1
id: update-widget
title: Update widget
status: assessing
risk: R2
data_change: true
affected:
  contexts: [widgets]
  entities: [Widget]
  capabilities: []
  operations: [operations/update-widget]
  interfaces: []
  paths: [src/widgets]
feature_contract: features/update-widget
EOF

cat > "$CLI_TARGET/.agentic/changes/update-widget/contract-assessment.yaml" <<'EOF'
schema_version: 2
change: update-widget
governing_contracts:
  - id: project/constitution
  - id: data/core-invariants
  - id: operations/update-widget
decisions: []
contract_gaps: []
platform_unknowns: []
conflicts: []
result: ready
EOF

cat > "$CLI_TARGET/.agentic/active-changes.yaml" <<'EOF'
schema_version: 1
changes:
  - id: update-widget
    status: assessing
    owner: builder
    operations: [operations/update-widget]
    depends_on: []
    conflicts_with: []
EOF

"$CLI_TARGET/.agentic/bin/agentic" --root "$CLI_TARGET" contract candidates update-widget > "$TMP_ROOT/candidates.yaml"
grep -q 'id: capabilities/unrelated-suggestion' "$TMP_ROOT/candidates.yaml"
grep -q -- '- contexts' "$TMP_ROOT/candidates.yaml"
if "$CLI_TARGET/.agentic/bin/agentic" --root "$CLI_TARGET" contract resolve update-widget >/dev/null 2>&1; then
  printf 'unassessed contract candidate was accepted\n' >&2
  exit 1
fi
cat >> "$CLI_TARGET/.agentic/changes/update-widget/contract-assessment.yaml" <<'EOF'
contract_candidates:
  - id: capabilities/unrelated-suggestion
    matched_by: [contexts]
    decision: excluded
    reason: Widget更新とは無関係なSuggestionである
EOF

pass_contract_challenge update-widget
"$CLI_TARGET/.agentic/bin/agentic" --root "$CLI_TARGET" contract resolve update-widget >/dev/null
if sed -n '/^contracts:/,/^excluded_contracts:/p' "$CLI_TARGET/.agentic/resolved/update-widget.lock.yaml" | grep -q 'capabilities/unrelated-suggestion'; then
  printf 'excluded candidate was selected\n' >&2
  exit 1
fi
sed -i.bak 's/capabilities: \[\]/capabilities: [missing-capability]/' "$CLI_TARGET/.agentic/changes/update-widget/change.yaml"
rm "$CLI_TARGET/.agentic/changes/update-widget/change.yaml.bak"
if "$CLI_TARGET/.agentic/bin/agentic" --root "$CLI_TARGET" contract resolve update-widget >/dev/null 2>&1; then
  printf 'missing capability coverage was accepted\n' >&2
  exit 1
fi
sed -i.bak 's/capabilities: \[missing-capability\]/capabilities: []/' "$CLI_TARGET/.agentic/changes/update-widget/change.yaml"
rm "$CLI_TARGET/.agentic/changes/update-widget/change.yaml.bak"
pass_contract_challenge update-widget
"$CLI_TARGET/.agentic/bin/agentic" --root "$CLI_TARGET" contract resolve update-widget >/dev/null
"$CLI_TARGET/.agentic/bin/agentic" --root "$CLI_TARGET" mutation build >/dev/null
grep -q '^  Widget:' "$CLI_TARGET/.agentic/generated/mutation-graph.yaml"
"$CLI_TARGET/.agentic/bin/agentic" --root "$CLI_TARGET" change ready update-widget >/dev/null
grep -q '^status: ready$' "$CLI_TARGET/.agentic/changes/update-widget/change.yaml"

printf '\n# changed after exclusion\n' >> "$CLI_TARGET/contracts/capabilities/unrelated-suggestion.yaml"
if "$CLI_TARGET/.agentic/bin/agentic" --root "$CLI_TARGET" change ready update-widget >/dev/null 2>&1; then
  printf 'changed excluded contract did not stale the lock\n' >&2
  exit 1
fi
pass_contract_challenge update-widget
"$CLI_TARGET/.agentic/bin/agentic" --root "$CLI_TARGET" contract resolve update-widget >/dev/null

mkdir -p "$CLI_TARGET/evidence/update-widget"
cat > "$CLI_TARGET/evidence/update-widget/index.yaml" <<'EOF'
schema_version: 1
change: update-widget
requirements:
  - id: INV-TODO-001
    contract: data/core-invariants
    clause: INV-TODO-001
    type: test
    command: test
    status: verified
    artifact: test.log
residual_risks: []
EOF
cat > "$CLI_TARGET/evidence/update-widget/challenge.yaml" <<'EOF'
schema_version: 1
change: update-widget
challenger: challenger
independent_context: true
resolved_contract: .agentic/resolved/update-widget.lock.yaml
result: passed
findings: []
EOF
"$CLI_TARGET/.agentic/bin/agentic" --root "$CLI_TARGET" evidence check update-widget >/dev/null

printf '\n# changed\n' >> "$CLI_TARGET/contracts/project/constitution.yaml"
if "$CLI_TARGET/.agentic/bin/agentic" --root "$CLI_TARGET" change ready update-widget >/dev/null 2>&1; then
  printf 'stale resolved lock was accepted\n' >&2
  exit 1
fi

sh "$KIT_ROOT/tests/test-authority.sh"

printf 'all tests passed\n'
