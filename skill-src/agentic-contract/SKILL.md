---
name: agentic-contract
description: Perform Contract Readiness Assessment and govern project, domain, capability, architecture, data-invariant, operation, and feature contracts. Use before implementing a non-trivial change, when a shared rule may be missing or conflicting, when data ownership/cardinality/lifecycle/protocol must be decided, or when a feature may be silently defining system-wide behavior.
---

# Agentic Contract

## Required inputs

- `.agentic/changes/<id>/change.yaml`
- `contracts/catalog.yaml` and existing accepted contracts
- `docs/agentic/source-of-truth.md`
- existing code, tests, specifications, decisions, issues, and incidents relevant to the affected area

Read `references/governance-rules.md` before classifying decisions. For a data-changing feature, also read `references/data-integrity.md`.

## Workflow

1. Verify the declared affected contexts, entities, capabilities, operations, interfaces, and paths against repository facts.
2. List every semantic or architectural decision the change needs.
3. Classify each decision as feature-local or upper-contract. Record reasoning; do not silently choose.
4. Run `.agentic/bin/agentic contract candidates <id>`. Treat its matches as discovery facts, not semantic decisions.
5. Record every candidate in `contract_candidates` as `included` or `excluded`, with the exact `matched_by` values and a concrete reason. Never exclude a candidate merely to make resolution pass.
6. Resolve existing governing contracts. Identify missing, proposed, contradictory, or stale contracts, and verify coverage for every affected context, entity, capability, operation, and interface.
7. For data changes, require applicable Data Invariants and Operation Contracts. Define transaction groups, consistency, idempotency, failure points, concurrency, deletion, and external effects.
8. Separate platform assumptions from verified capabilities. Add required probes for unverified dependencies.
9. Write `.agentic/changes/<id>/contract-assessment.yaml` with governing contracts, contract candidates, decisions, gaps, conflicts, platform unknowns, and result.
10. If an upper contract is missing, set the change to `blocked-contract-decision`, propose the contract and options, and obtain the required human decision before accepting it.
11. When all candidates and decisions are resolved, all required contracts are accepted, coverage is complete, and probes exist, set assessment `result: ready`, run `agentic contract lint`, then `agentic contract resolve <id>`.

The CLI validates recorded facts and blocks unresolved work. It does not decide business semantics. A Decision explains why; the accepted Contract stores the current rule.
