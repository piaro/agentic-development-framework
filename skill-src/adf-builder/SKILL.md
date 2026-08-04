---
name: adf-builder
description: Implement a change the control plane has cleared for building, and record the evidence that each governing contract clause is met. Use when `adf next` issues an action whose role is Builder, or when a change is in ready-to-build or needs-evidence. Do not use it to decide what the change should do - that is settled before the build starts.
---

# Agentic Builder

The control plane only issues an `implement-change` action once every requirement that
had to be settled before building is settled. Your job is to build what was decided,
not to decide anything further.

## Get the assigned work

Through MCP, call `adf_next` with the change id. Without MCP, run
`adf next <change-id>`. Stop if `role` is not `Builder`.

The action's `context` holds the change, the contracts that govern it, the decisions
behind them, and the affected code. Read every contract in that context before writing
code. Use decisions for rationale only - the contract states the rule you must meet.

## `implement-change`

Implement against the contracts in the context. While you build:

- Do not weaken a validation condition, widen an interface, or relax an invariant to
  make the implementation easier.
- Do not add behavior no contract asks for.
- Keep to the architecture and dependency rules the context carries.

Submit a `summary` of what you implemented.

### When implementation uncovers a decision

Stop building. Do not resolve it yourself and do not encode a guess in the code.
Report it so the change can return to analysis, where a decision request carries the
options and their impact to the person who may decide. A specification settled inside
an implementation is invisible to everyone who reviews the change later.

This is the most common way the control plane gets bypassed. Resist it.

## `record-evidence`

Show that each requirement instance in the action actually holds. For each one, call
`adf_add_evidence` with what you observed, then submit an `outcomes` entry:

- `instance_key` and `definition_digest` copied from the action verbatim
- `status`: `satisfied`, `unsatisfied`, or `inconclusive`
- `summary`: what the evidence shows
- `basis_refs`: the tests, probes, and artifacts you actually ran or read

Evidence is traceable to a contract clause or it is not evidence. A mock does not
demonstrate that a platform behaves as assumed, and a passing test that never exercised
the path proves nothing about it. Where you could not establish the requirement, report
`inconclusive` and say what would settle it. Reporting `satisfied` because the change
looks correct defeats the whole mechanism.

Every residual risk needs someone who accepts it and a date by which it is revisited.

## Submit and continue

Call `adf_submit` with the action id, the context digest from the action, and the
payload. The control plane validates it and returns the next action - usually a
challenge run from a context independent of yours.

If submission is rejected as stale, the inputs moved under you. Call `adf_next`
again and work from the fresh action rather than retrying the old payload.
