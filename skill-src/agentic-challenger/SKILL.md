---
name: agentic-challenger
description: Falsify a repository change before implementation against its request, authority, decisions, and proposed contracts, or after implementation against its resolved contracts, data invariants, mutation graph, diff, and evidence. Use before Contract resolution for R1-R3 work and after implementation for R2/R3 or stateful, concurrent, external, security-sensitive behavior.
---

# Agentic Challenger

## Independence

Determine the phase from the requested task and change state; do not silently substitute one phase for the other. Record `independent_context: false` when a fresh context was unavailable; never present that as independent challenge. R2/R3 require an independent context in both phases.

Read `references/challenge-method.md`. For data changes, also read `references/stateful-challenge.md`.

## Pre-implementation Contract / Authority Challenge

1. Start from the Issue or request, change impact, Assessment, authority refs, existing Decisions, and proposed or changed Contracts. Do not require a resolved lock or implementation diff in this phase.
2. Look for omitted decisions, undeclared entities, interfaces, protocols, lifecycle rules, external effects, feature-local rules that should be global, and scope beyond the request.
3. For every decision, try to disprove that its authority supports the whole statement. Check conflicting authority, circular self-authorization, existing non-scope, and misclassified specification extensions.
4. Findings may identify a gap or recommend opening a Decision Request, but are never authority and never select the product answer.
5. Run `agentic contract challenge-input <id>` to obtain the review hashes, then write `.agentic/changes/<id>/contract-challenge.yaml` with reviewed decision ids and findings. Use `authority-missing`, `authority-mismatch`, `authority-conflict`, `circular-authority`, and `unrecorded-specification-extension` where applicable.
6. Set `result: passed` only when no blocking finding remains. Otherwise set `result: blocked` and return to `$agentic-contract`.

## Post-implementation / Stateful Challenge

1. Start from the resolved lock, contracts, raw diff, tests, and evidence rather than the Builder's explanation.
2. Reconstruct affected domains, entities, operations, interfaces, and paths from the actual diff.
3. Perform Governance Challenge: find omitted governing contracts, contradictions, undeclared mutations, and new decisions made during implementation. Reopen Assessment when found.
4. Perform Implementation Challenge against every resolved contract clause and accepted deviation.
5. Perform Stateful Challenge across operation sequences, concurrency, duplicate, out-of-order, timeout, partial success, retry, deletion, migration, and external convergence.
6. Perform Completion Challenge: map every required clause to a test, probe, observation, or accepted residual risk.
7. For Security Signal actions, probe privilege escalation, unauthorized access, data disclosure through output or logs, and missing negative-path coverage where applicable.
8. Write `evidence/<id>/challenge.yaml`. Use finding types `missing-global-contract`, `invariant-violation`, `implementation-violation`, and `evidence-gap`.

Do not weaken the contract to clear a finding. Distinguish an implementation defect, a contract gap, a missing authority, a human Decision Request, a platform unknown, and an explicitly accepted risk.
