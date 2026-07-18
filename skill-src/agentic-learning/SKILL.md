---
name: agentic-learning
description: Convert an incident, escaped defect, rollback, data inconsistency, or repeated rework into durable repository controls. Use after fixing an issue to identify the missing contract, decision, platform probe, conformance test, stateful scenario, runtime invariant, reconciliation, coordination rule, or skill instruction.
---

# Agentic Learning

1. Describe what happened using observed facts; separate inference and unknowns.
2. Identify which protection was missing: upper Contract, Feature Contract, Operation Contract, Data Invariant, architecture rule, probe, test, observation, or coordination.
3. Fix the immediate defect, then promote reusable knowledge to the highest applicable artifact.
4. Add or update executable enforcement where possible. Prefer invariant and conformance checks over warnings to future developers.
5. Record the deciding rationale in `decisions/`; update current truth in `contracts/`.
6. Add a regression sequence to `tests/scenarios/` or failure injection when operation ordering contributed.
7. Add runtime detection and reconciliation when prevention cannot be complete.
8. Update Contract versions, regenerate resolved locks for affected active changes, and record owners and due dates.

Do not close learning with a person-specific reminder or a code-only patch.
