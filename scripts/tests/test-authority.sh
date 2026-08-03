#!/bin/sh

set -eu

TEST_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
KIT_ROOT=$(CDPATH= cd -- "$TEST_DIR/../.." && pwd)
TEST_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/agentic-authority.XXXXXX")

cleanup() {
  rm -rf "$TEST_ROOT"
}
trap cleanup EXIT HUP INT TERM

TARGET="$TEST_ROOT/repository"
mkdir -p "$TARGET"
"$KIT_ROOT/bin/agentic-init" --mode adopt --level lite --target "$TARGET" --name authority-test --non-interactive >/dev/null
CLI="$TARGET/.agentic/bin/agentic"
CHANGE=authority-case

sed -i.bak 's/status: proposed/status: accepted/' "$TARGET/contracts/project/constitution.yaml"
rm "$TARGET/contracts/project/constitution.yaml.bak"
mkdir -p "$TARGET/contracts/capabilities"
cat > "$TARGET/contracts/capabilities/explicit-result.yaml" <<'EOF'
schema_version: 2
id: capabilities/explicit-result
kind: capability
status: accepted
version: 1
owners: [test]
applies_to:
  contexts: []
  entities: []
  capabilities: []
  operations: []
  interfaces: []
sources: []
rules:
  - id: EXPLICIT-RESULT-001
    statement: Rule introduced by the change
EOF
"$CLI" --root "$TARGET" change init "$CHANGE" --title "Authority case" >/dev/null

cat > "$TARGET/contracts/features/$CHANGE.yaml" <<'EOF'
schema_version: 2
id: features/authority-case
kind: feature
status: accepted
version: 1
owners: [test]
applies_to:
  contexts: []
  entities: []
  capabilities: []
  operations: []
  interfaces: []
sources: []
risk: R1
governing_contracts: [project/constitution]
outcome: authority validation
non_scope: []
completion_semantics: validated
failure_semantics: {}
introduced_decisions: []
introduced_rules:
  - id: FEATURE-RULE-001
    statement: authorized feature rule
deviations: []
unknowns: []
required_probes: []
evidence_requirements: []
residual_risks: []
EOF

cat > "$TARGET/.agentic/changes/$CHANGE/change.yaml" <<'EOF'
schema_version: 1
id: authority-case
title: Authority case
status: assessing
risk: R1
data_change: false
affected:
  contexts: []
  entities: []
  capabilities: []
  operations: []
  interfaces: []
  paths: []
feature_contract: features/authority-case
EOF

write_decision() {
  kind=$1
  artifact=$2
  locator=$3
  extra=${4:-}
  cat > "$TARGET/.agentic/changes/$CHANGE/contract-assessment.yaml" <<EOF
schema_version: 2
change: authority-case
governing_contracts:
  - id: project/constitution
contract_candidates: []
decisions:
  - id: CD-001
    statement: Apply an authorized rule
    scope: feature-local
    effect: contract-application
    status: resolved
    reason: Explicit source authorizes the rule
    authorities:
      - kind: $kind
        refs:
          - artifact: $artifact
            locator: $locator
$extra
    evidence_refs: []
    resulting_contract_refs: []
contract_gaps: []
platform_unknowns: []
conflicts: []
result: ready
EOF
}

pass_challenge() {
  "$CLI" --root "$TARGET" contract challenge-input "$CHANGE" > "$TARGET/.agentic/changes/$CHANGE/contract-challenge.yaml"
  sed -i.bak 's/challenger: TODO/challenger: authority-challenger/' "$TARGET/.agentic/changes/$CHANGE/contract-challenge.yaml"
  rm "$TARGET/.agentic/changes/$CHANGE/contract-challenge.yaml.bak"
  sed -i.bak 's/result: pending/result: passed/' "$TARGET/.agentic/changes/$CHANGE/contract-challenge.yaml"
  rm "$TARGET/.agentic/changes/$CHANGE/contract-challenge.yaml.bak"
}

assert_resolve_fails() {
  expected=$1
  if "$CLI" --root "$TARGET" contract resolve "$CHANGE" >"$TEST_ROOT/out" 2>"$TEST_ROOT/err"; then
    printf 'resolve unexpectedly passed: %s\n' "$expected" >&2
    exit 1
  fi
  grep -q "$expected" "$TEST_ROOT/err"
}

cat > "$TARGET/.agentic/changes/$CHANGE/contract-assessment.yaml" <<'EOF'
schema_version: 2
change: authority-case
governing_contracts: [{id: project/constitution}]
contract_candidates: []
decisions:
  - id: CD-001
    statement: Unauthorised decision
    scope: feature-local
    effect: contract-application
    status: resolved
    reason: A reason is not authority
contract_gaps: []
platform_unknowns: []
conflicts: []
result: ready
EOF
assert_resolve_fails 'authorityがありません'

for banned in challenger-finding agent-inference contract-gap implementation-convenience existing-code-only test-only; do
  write_decision "$banned" evidence/source "finding:GOV-001"
  assert_resolve_fails '認可源ではありません'
done

write_decision accepted-contract project/constitution id:PROJECT-INV-001 '            version: 1'
pass_challenge
"$CLI" --root "$TARGET" contract resolve "$CHANGE" >/dev/null

sed -i.bak 's/introduced_decisions: \[\]/introduced_decisions: [CD-CIRCULAR]/' "$TARGET/contracts/features/$CHANGE.yaml"
rm "$TARGET/contracts/features/$CHANGE.yaml.bak"
cat > "$TARGET/.agentic/changes/$CHANGE/contract-assessment.yaml" <<'EOF'
schema_version: 2
change: authority-case
governing_contracts: [{id: project/constitution}]
contract_candidates: []
decisions:
  - id: CD-CIRCULAR
    statement: Change the same clause used as authority
    scope: upper-contract
    effect: specification-extension
    status: resolved
    reason: Circular authority must fail
    authorities:
      - kind: accepted-contract
        refs:
          - artifact: project/constitution
            locator: id:PROJECT-INV-001
            version: 1
    evidence_refs: []
    resulting_contract_refs:
      - contract: project/constitution
        locator: id:PROJECT-INV-001
contract_gaps: []
platform_unknowns: []
conflicts: []
result: ready
EOF
assert_resolve_fails '自身のauthorityにはできません'
sed -i.bak 's/introduced_decisions: \[CD-CIRCULAR\]/introduced_decisions: []/' "$TARGET/contracts/features/$CHANGE.yaml"
rm "$TARGET/contracts/features/$CHANGE.yaml.bak"
assert_resolve_fails '仕様拡張decisionがFeature introduced_decisionsにありません'

write_decision issue-requirement https://github.example/example/repository/issues/19 body:acceptance-criteria-1
pass_challenge
"$CLI" --root "$TARGET" contract resolve "$CHANGE" >/dev/null

cat > "$TARGET/decisions/DEC-HUMAN.md" <<'EOF'
# DEC-HUMAN: Human decision

- Status: accepted

## Decision

Apply the feature-local rule.
EOF
write_decision human-decision decisions/DEC-HUMAN.md heading:Decision '        recorded_by: product-owner
        recorded_at: 2026-07-22'
pass_challenge
"$CLI" --root "$TARGET" contract resolve "$CHANGE" >/dev/null
sed -i.bak 's/Apply the feature-local rule./Apply a different feature-local rule./' "$TARGET/decisions/DEC-HUMAN.md"
rm "$TARGET/decisions/DEC-HUMAN.md.bak"
assert_resolve_fails 'staleです: authorities_sha256'
sed -i.bak 's/Apply a different feature-local rule./Apply the feature-local rule./' "$TARGET/decisions/DEC-HUMAN.md"
rm "$TARGET/decisions/DEC-HUMAN.md.bak"
sed -i.bak 's/Status: accepted/Status: proposed/' "$TARGET/decisions/DEC-HUMAN.md"
rm "$TARGET/decisions/DEC-HUMAN.md.bak"
assert_resolve_fails 'Decision recordがacceptedではありません'
sed -i.bak 's/Status: proposed/Status: accepted/' "$TARGET/decisions/DEC-HUMAN.md"
rm "$TARGET/decisions/DEC-HUMAN.md.bak"

write_decision accepted-decision decisions/DEC-HUMAN.md heading:Decision
pass_challenge
"$CLI" --root "$TARGET" contract resolve "$CHANGE" >/dev/null

write_decision issue-requirement "''" "''"
assert_resolve_fails 'artifact/locatorがありません'

cat > "$TARGET/.agentic/changes/$CHANGE/contract-assessment.yaml" <<'EOF'
schema_version: 2
change: authority-case
governing_contracts: [{id: project/constitution}]
contract_candidates: []
decisions:
  - id: CD-REQUEST
    statement: Select retry semantics
    scope: upper-contract
    effect: specification-extension
    status: needs-human-decision
    reason: Existing authority does not select a product rule
    authorities: []
    evidence_refs:
      - kind: challenger-finding
        refs: [{artifact: evidence/finding, locator: 'finding:GOV-001'}]
    resulting_contract_refs: []
    request:
      question: How should retries after identifier changes behave?
      why_now: A counterexample shows multiple valid product choices
      discovered_by:
        role: pre-implementation-challenger
        ref: finding:GOV-001
      options:
        - id: A
          summary: Add a persistent request claim
          impact: [Adds a record and retention semantics]
        - id: B
          summary: Exclude create from retry guarantees
          impact: [Adds an exception to the upper principle]
      recommendation:
        option: B
        reason: It does not silently expand the Issue scope
      required_decider: product-owner
contract_gaps: []
platform_unknowns: []
conflicts: []
result: blocked-contract-decision
EOF
"$CLI" --root "$TARGET" contract decisions "$CHANGE" > "$TEST_ROOT/decisions.txt"
grep -q 'How should retries' "$TEST_ROOT/decisions.txt"
grep -q 'authorityではありません' "$TEST_ROOT/decisions.txt"
"$CLI" --root "$TARGET" contract decisions "$CHANGE" --format markdown > "$TEST_ROOT/decisions.md"
grep -q '^## \[CD-REQUEST\]' "$TEST_ROOT/decisions.md"
assert_resolve_fails 'human decision requests'

sed -i.bak 's/introduced_decisions: \[\]/introduced_decisions: [CD-EXTEND]/' "$TARGET/contracts/features/$CHANGE.yaml"
rm "$TARGET/contracts/features/$CHANGE.yaml.bak"
cat > "$TARGET/.agentic/changes/$CHANGE/contract-assessment.yaml" <<'EOF'
schema_version: 2
change: authority-case
governing_contracts: [{id: project/constitution}]
contract_candidates: []
decisions:
  - id: CD-EXTEND
    statement: Introduce the explicitly requested feature rule
    scope: feature-local
    effect: specification-extension
    status: resolved
    reason: The Issue explicitly requests this rule
    authorities:
      - kind: issue-requirement
        refs:
          - artifact: https://github.example/example/repository/issues/19
            locator: body:acceptance-criteria-2
    evidence_refs: []
    resulting_contract_refs:
      - contract: capabilities/explicit-result
        locator: id:EXPLICIT-RESULT-001
contract_gaps: []
platform_unknowns: []
conflicts: []
result: ready
EOF
pass_challenge
"$CLI" --root "$TARGET" contract resolve "$CHANGE" >/dev/null
grep -q 'id: capabilities/explicit-result' "$TARGET/.agentic/resolved/$CHANGE.lock.yaml"

sed -i.bak 's/result: passed/result: blocked/' "$TARGET/.agentic/changes/$CHANGE/contract-challenge.yaml"
rm "$TARGET/.agentic/changes/$CHANGE/contract-challenge.yaml.bak"
sed -i.bak 's/findings: \[\]/findings:\n- id: GOV-001\n  type: authority-mismatch\n  decision: CD-EXTEND\n  severity: blocking/' "$TARGET/.agentic/changes/$CHANGE/contract-challenge.yaml"
rm "$TARGET/.agentic/changes/$CHANGE/contract-challenge.yaml.bak"
assert_resolve_fails 'blocking finding'

pass_challenge
sed -i.bak 's/The Issue explicitly requests this rule/The reason changed after Challenge/' "$TARGET/.agentic/changes/$CHANGE/contract-assessment.yaml"
rm "$TARGET/.agentic/changes/$CHANGE/contract-assessment.yaml.bak"
assert_resolve_fails 'staleです: assessment_sha256'

sed -i.bak 's/The reason changed after Challenge/The Issue explicitly requests this rule/' "$TARGET/.agentic/changes/$CHANGE/contract-assessment.yaml"
rm "$TARGET/.agentic/changes/$CHANGE/contract-assessment.yaml.bak"
pass_challenge
"$CLI" --root "$TARGET" contract resolve "$CHANGE" >/dev/null
"$CLI" --root "$TARGET" contract authority-check "$CHANGE" >/dev/null
mkdir -p "$TARGET/.agentic/changes/completed-history"
cat > "$TARGET/.agentic/changes/completed-history/contract-assessment.yaml" <<'EOF'
schema_version: 1
change: completed-history
EOF
cat >> "$TARGET/.agentic/active-changes.yaml" <<'EOF'
- id: completed-history
  status: complete
  owner: history
  operations: []
  depends_on: []
  conflicts_with: []
EOF
"$CLI" --root "$TARGET" contract authority-check --all >/dev/null
cp "$TARGET/.agentic/changes/$CHANGE/contract-assessment.yaml" "$TEST_ROOT/assessment-v2.yaml"
sed -i.bak 's/schema_version: 2/schema_version: 1/' "$TARGET/.agentic/changes/$CHANGE/contract-assessment.yaml"
rm "$TARGET/.agentic/changes/$CHANGE/contract-assessment.yaml.bak"
if "$CLI" --root "$TARGET" contract authority-check "$CHANGE" >/dev/null 2>&1; then
  printf 'legacy assessment passed authority check\n' >&2
  exit 1
fi
cp "$TEST_ROOT/assessment-v2.yaml" "$TARGET/.agentic/changes/$CHANGE/contract-assessment.yaml"
sed -i.bak 's/schema_version: 2/schema_version: 1/' "$TARGET/.agentic/resolved/$CHANGE.lock.yaml"
rm "$TARGET/.agentic/resolved/$CHANGE.lock.yaml.bak"
if "$CLI" --root "$TARGET" change ready "$CHANGE" >/dev/null 2>&1; then
  printf 'legacy lock was accepted\n' >&2
  exit 1
fi

grep -q 'authority-missing' "$KIT_ROOT/scripts/tests/fixtures/authority/create-retry-gap/expected-contract-challenge.yaml"

printf 'authority tests passed\n'
