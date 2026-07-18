---
name: agentic-builder
description: Implement a repository change only after Contract Readiness succeeds. Use when a change has status ready and a fresh resolved-contract lock, and implementation must follow governing domain, capability, architecture, data-invariant, operation, and feature contracts without weakening validation conditions.
---

# Agentic Builder

## Preconditions

1. Read `.agentic/changes/<id>/change.yaml` and `.agentic/resolved/<id>.lock.yaml`.
2. Run `.agentic/bin/agentic change ready <id>`. Stop if it fails.
3. Read every contract in the lock and the referenced implementation examples. Use accepted Decisions only for rationale.

## Build

- Implement the smallest change satisfying the resolved contracts.
- Preserve Data Invariants after every transaction boundary and define convergence for derived or external stores.
- Implement Operation Contract semantics for duplicate, retry, concurrency, cancellation, timeout, partial failure, and deletion.
- Add evidence mapped to contract and clause ids.
- Do not change a governing contract, Feature Contract, or validation condition to make implementation pass. Reopen Contract Assessment instead.
- If the actual diff touches undeclared entities, operations, interfaces, or paths, update the change and invalidate the lock before continuing.

After relevant tests and probes pass, set the change to `challenging` and hand the lock, raw diff, Mutation Graph, and evidence to an independent `$agentic-challenger`. Do not provide a persuasive implementation summary as its primary input.
