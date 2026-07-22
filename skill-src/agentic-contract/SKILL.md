---
name: agentic-contract
description: Perform Contract Readiness Assessment and govern project, domain, capability, architecture, data-invariant, operation, and feature contracts. Use before implementing a non-trivial change, when a shared rule may be missing or conflicting, when data ownership/cardinality/lifecycle/protocol must be decided, or when a feature may be silently defining system-wide behavior.
---

# Agentic Contract

## Required inputs

- `.agentic/changes/<id>/change.yaml`
- `.agentic/changes/<id>/contract-challenge.yaml`
- `contracts/catalog.yaml` and existing accepted contracts
- `docs/agentic/source-of-truth.md`
- existing code, tests, specifications, decisions, issues, and incidents relevant to the affected area

Read `references/governance-rules.md` before classifying decisions. For a data-changing feature, also read `references/data-integrity.md`.

## Workflow

1. Verify the declared affected contexts, entities, capabilities, operations, interfaces, and paths against repository facts.
2. List every semantic or architectural decision the change needs. Give each decision a stable change-local id.
3. Classify each decision as feature-local or upper-contract and record its effect. Record reasoning; do not silently choose.
4. Run `.agentic/bin/agentic contract candidates <id>`. Treat its matches as discovery facts, not semantic decisions.
5. Record every candidate in `contract_candidates` as `included` or `excluded`, with the exact `matched_by` values and a concrete reason. Never exclude a candidate merely to make resolution pass.
6. Resolve existing governing contracts. Identify missing, proposed, contradictory, or stale contracts, and verify coverage for every affected context, entity, capability, operation, and interface.
7. Separate authority from evidence. Only an explicit accepted Contract clause, Issue requirement, recorded human decision, or accepted Decision record can authorize a resolved decision. Agent inference, Challenger findings, contract gaps, implementation convenience, code, and tests are evidence, not authority.
8. For data changes, require applicable Data Invariants and Operation Contracts. Define transaction groups, consistency, idempotency, failure points, concurrency, deletion, and external effects.
9. Separate platform assumptions from verified capabilities. Add required probes for unverified dependencies.
10. Write `.agentic/changes/<id>/contract-assessment.yaml` with governing contracts, contract candidates, decisions, authority, evidence, resulting contract clauses, gaps, conflicts, platform unknowns, and result. Put resolved specification-extension decision ids in the Feature Contract's `introduced_decisions`.
11. If existing authority cannot determine a specification or architecture choice, set the decision to `needs-human-decision`, add a concise `request` with the question, discovery role/reference, options, impact, recommendation, and required decider, and set both change and assessment to `blocked-contract-decision`. A Decision Request is temporary workflow information, not a source of truth.
12. After a human decision, record the durable rationale in `decisions/`, update the current rule in `contracts/`, and reference that record as authority. Downstream contracts and changes must not depend on the Decision Request.
13. When the proposed Contract content and Assessment are complete, hand off to `$agentic-challenger` in pre-implementation mode. Apply findings by reopening Assessment; never use a finding as authority.
14. After a fresh pre-implementation Challenge passes, accept the authorized Contracts, set assessment `result: ready`, run `agentic contract lint`, then `agentic contract resolve <id>`.

The CLI validates recorded facts and blocks unresolved work. It does not decide business semantics. A Decision Request carries a temporary question, a Decision explains the durable rationale, and the accepted Contract stores the current rule.
