---
name: agentic-change
description: Initialize and control a repository change before implementation. Use when starting a feature, schema change, integration, refactor, migration, or other non-trivial work that must identify affected domains, entities, capabilities, operations, interfaces, data mutations, risk, dependencies, and governing contracts.
---

# Agentic Change

## Workflow

1. Read `AGENTS.md`, `.agentic/config.yaml`, `contracts/catalog.yaml`, and `docs/agentic/source-of-truth.md`.
2. Run `.agentic/bin/agentic change init <id> --title "..."` when the change does not exist.
3. Investigate the request and existing implementation. Fill `.agentic/changes/<id>/change.yaml` with affected contexts, entities, capabilities, operations, interfaces, and paths.
4. Set `data_change: true` for any persisted-state mutation, migration, external write, or derived-store synchronization.
5. Classify R0-R3 from impact and irreversibility, not diff size. Raise multi-entity, external, concurrent, or asynchronous mutations to at least R2.
6. Add the change to `.agentic/active-changes.yaml`, including operation ids and dependencies.
7. Use `$agentic-contract` to complete Contract Readiness Assessment. Any role that discovers an unauthorized specification or architecture choice must return here; the Contract workflow records the canonical Decision Request.
8. Use `$agentic-challenger` in pre-implementation mode before Contract resolution. Reopen Assessment for every blocking governance or authority finding.
9. After a fresh Challenge passes and authorized Contracts are accepted, run `agentic contract resolve <id>` and `agentic change ready <id>`.

Do not infer that a decision is feature-local merely because only one current feature requests it. Shared entity semantics, ownership, cardinality, lifecycle, protocol, security, persistence, or future reference implementations require an upper contract.
