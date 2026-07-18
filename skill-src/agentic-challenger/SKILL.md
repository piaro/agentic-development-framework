---
name: agentic-challenger
description: Independently falsify a repository change against its complete resolved contracts, data invariants, operation contracts, mutation graph, and evidence. Use after implementation for R2/R3 work, for data-changing or concurrent behavior, or whenever missing global contracts, hidden state transitions, partial failures, retries, tenant leaks, or architectural divergence may exist.
---

# Agentic Challenger

## Independence

Start from the resolved lock, contracts, raw diff, tests, and evidence rather than the Builder's explanation. Record `independent_context: false` when a fresh context was unavailable; never present that as independent challenge.

Read `references/challenge-method.md`. For data changes, also read `references/stateful-challenge.md`.

## Workflow

1. Reconstruct affected domains, entities, operations, interfaces, and paths from the actual diff.
2. Perform Governance Challenge: find omitted governing contracts, feature-local decisions that should be global, contradictions, and undeclared mutations. Return `governance-gap` and reopen assessment when found.
3. Perform Implementation Challenge against every resolved contract clause and accepted deviation.
4. Perform Stateful Challenge across operation sequences, concurrency, duplicate, out-of-order, timeout, partial success, retry, deletion, migration, and external convergence.
5. Perform Completion Challenge: map every required clause to a test, probe, observation, or accepted residual risk.
6. Write `evidence/<id>/challenge.yaml`. Use finding types `missing-global-contract`, `invariant-violation`, `implementation-violation`, and `evidence-gap`.

Do not weaken the contract to clear a finding. Distinguish an implementation defect, a contract gap, a platform unknown, and an explicitly accepted risk.
