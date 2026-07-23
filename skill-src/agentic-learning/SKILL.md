---
name: agentic-learning
description: Convert an incident, escaped defect, rollback, data inconsistency, or repeated rework into durable repository controls. Use after fixing an issue to identify the missing contract, decision, platform probe, conformance test, stateful scenario, runtime invariant, reconciliation, coordination rule, or skill instruction.
---

# Agentic Learning

1. Describe what happened using observed facts; separate inference and unknowns.
2. Identify which protection was missing: upper Contract, Feature Contract, Operation Contract, Data Invariant, architecture rule, probe, test, observation, or coordination.
3. Fix the immediate defect, then promote reusable knowledge to the highest applicable artifact.
4. Add or update executable enforcement where possible. Prefer invariant and conformance checks over warnings to future developers.
5. Treat the incident and its findings as evidence, not authority. If recovery requires a new product or architecture choice, return it to Contract Assessment as a Decision Request.
6. Record an authorized deciding rationale in `decisions/`; update current truth in `contracts/`.
7. Add a regression sequence to `tests/scenarios/` or failure injection when operation ordering contributed.
8. Add runtime detection and reconciliation when prevention cannot be complete.
9. Update Contract versions, regenerate resolved locks for affected active changes, and record owners and due dates.

Do not close learning with a person-specific reminder or a code-only patch.
