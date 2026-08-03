---
name: agentic-analyst
description: Carry out the Analyst work the control plane assigns for a change - reviewing detected risk signals, confirming affected data and operation boundaries, writing governing contracts, raising decision requests when no authority settles a product choice, and recording a human's answer as a Decision and Contract. Use when `agentic next` issues an action whose role is Analyst, or when a change is in needs-analysis, needs-human-decision, needs-decision-recording, or needs-post-build-analysis.
---

# Agentic Analyst

You do not decide the order of work. The control plane does. Ask it what to do, do that
one thing, submit the result, and ask again.

## Get the assigned work

Through MCP, call `agentic_next` with the change id. Without MCP, run
`agentic next <change-id>`. Either way you receive:

- `state`: where the change stands
- `action`: what to do now, including `id`, `role`, and `action`
- `action.requirement_instances`: each item you must answer, with `instance_key` and
  `definition_digest`
- `context`: the change, matching contracts, matching decisions, affected code, and
  repository artifacts selected for this action - and nothing else

Stop if `role` is not `Analyst`. Hand the work to the skill that owns that role.

Copy `instance_key` and `definition_digest` from the action into your result verbatim.
Never compute or guess them. A digest you invent will be rejected, and a digest you copy
from an older action means you are answering a question that no longer exists.

## `review-risk-signals`

The detector proposes candidates from the actual source. You judge whether each one
applies to this change.

For every candidate in `context.payload.signal_candidates`, submit an entry with:

- `fingerprint`: copied from the candidate
- `status`: `confirmed` when the candidate governs this change, `not-applicable` when
  the overlap is incidental
- `reason`: why, in terms of the code you read
- `basis_refs`: the artifacts you actually read to decide

Read the code the candidate points at. A candidate is not confirmed because its name
looks right, and not dismissed because it is inconvenient. If you cannot tell, mark the
matching outcome `inconclusive` rather than guessing.

## `analyze-requirements`

Answer each requirement instance in the action. Submit an `outcomes` entry per instance:

- `status`: `satisfied`, `unsatisfied`, or `inconclusive`
- `summary`: what you established, in the repository's own terms
- `basis_refs`: contracts, decisions, code, and evidence you relied on

Where a requirement asks for a governing rule that does not exist yet, write it as a
contract. Read `references/contract-governance.md` for which contract kind to use and
when a rule must be promoted above the current feature. For any write operation, work
through `references/data-integrity.md` before claiming the operation boundary is
confirmed.

### When no existing authority settles the question

Do not invent the answer, and do not let the implementation's convenience settle it.
Add a `decision_requests` entry with the question, the facts already known, the options
with their impact, your recommendation and its reason, and the authority required to
decide. The change moves to `needs-human-decision`.

A reason explains a choice; authority permits it. Your own inference, a challenger's
finding, a contract gap, existing code, and passing tests are evidence. None of them can
authorize a specification.

## `answer-decision-request`

This action's role is Human, not Analyst. Present the question, options, impact, and
your recommendation to the person, and wait. Submit only what they actually chose:

- `request_id` and `selection` as given
- `actor_ref` identifying who decided

Never submit your recommendation as if it were their answer, and never submit a default
because no one replied.

## `record-human-decision`

Turn the answered question into durable records, then submit.

1. Call `agentic_apply_decision` to save the Decision carrying the rationale.
2. Call `agentic_apply_contract` to update the Contract that now holds the current rule,
   citing that Decision as its authority.
3. Submit the action.

Contract updates use optimistic locking. Send `expected_clause_digests` when you change
individual clauses, `expected_digest` when you replace the whole contract, and
`expected_digest: null` when you create one. Never send both forms at once. A rejection
means someone changed the same clause first: re-read the contract and redo the edit
rather than forcing it through.

Rationale belongs in `decisions/`. The current rule belongs in `contracts/`. The decision
request is temporary - nothing downstream may keep referring to it.

## Submit and continue

Call `agentic_submit` with the action id, the context digest from the action, and the
payload. The control plane validates it, stores it, and returns the next action. Repeat
until it hands the work to another role.

If submission is rejected as stale, the inputs moved under you. Call `agentic_next`
again and redo the work against the fresh action - do not retry the old payload.
