---
name: adf-analyst
description: Carry out the Analyst work the control plane assigns for a change - assessing intended impact, reviewing detected risk signals, confirming affected data and operation boundaries, writing governing contracts, raising decision requests when no authority settles a product choice, and recording a human's answer as a Decision and Contract. Use when `adf next` issues an action whose role is Analyst, or when a change is in needs-impact-assessment, needs-analysis, needs-human-decision, needs-decision-recording, needs-post-build-impact-assessment, or needs-post-build-analysis.
---

# Agentic Analyst

You do not decide the order of work. The control plane does. Ask it what to do, do that
one thing, submit the result, and ask again.

## Get the assigned work

Through MCP, call `adf_next` with the change id. Without MCP, run
`adf next <change-id>`. Either way you receive:

- `state`: where the change stands
- `action`: what to do now, including `id`, `role`, and `action`
- `action.execution_guidance`: an advisory model tier and the conditions under which the
  orchestrator should escalate to a more capable model
- `action.requirement_instances`: each item you must answer, with `instance_key` and
  `definition_digest`
- `context`: the change, matching contracts, matching decisions, affected code, and
  repository artifacts selected for this action - and nothing else

Stop if `role` is not `Analyst`. Hand the work to the skill that owns that role.

Copy `instance_key` and `definition_digest` from the action into your result verbatim.
Never compute or guess them. A digest you invent will be rejected, and a digest you copy
from an older action means you are answering a question that no longer exists.

Use only the issued Context for the assigned action. Do not start with a repository-wide
search. When the Context contains prior Result records, reuse their still-applicable facts
and investigate only what changed.

## `assess-change-impact`

Assess what the Change is intended to affect before relying on detected source calls. This
action receives the Change, compact repository and governance indexes, the built-in Signal
Catalog, and up to three prior Impact Assessments.

An explicit Contract verification scope selects existing behavior for Evidence work; it
does not by itself declare a new application impact. Assess the Change intent honestly.
For an Evidence-only Change, `no-impact` is valid after the same active check required for
any other Change.

For each intended effect, choose a Signal from `context.payload.signal_catalog` and copy
its required binding names exactly. Record:

- `signal` and `bindings`: the catalog Signal and the logical subjects it affects
- `description`: the intended effect in repository terms
- `risk`: `low`, `medium`, or `high`
- `governing_refs`: current Contract clause references that govern the effect, or an empty
  array when the first Change must establish them; use Decisions as authority in
  `basis_refs`, not as a substitute for a current Contract

Submit one of three statuses:

- `impacts-identified`: at least one impact and no unknowns
- `no-impact`: no impacts and no unknowns; use this only after actively checking the
  Change intent and the issued indexes
- `inconclusive`: at least one unknown; this does not authorize implementation and causes
  the control plane to issue another assessment

`basis_refs` must name the issued Change, index, Contract, Decision, repository artifact,
or prior assessment actually used. An empty repository is not evidence of `no-impact`.
For a greenfield project, derive intended effects from the Change request and declare them
even when there is no source code or Contract yet.

The recommended model tier is advisory to the orchestrator. For this action it is normally
`economy`. Escalate when the issued guidance says to do so, including before concluding
`no-impact`, when authority is insufficient or contradictory, or when material security,
privacy, payment, or irreversible-data risk is plausible.

## `establish-impact-governance`

This action follows a greenfield or otherwise ungoverned assessed impact. Create only the
Contract clauses needed for the current Change by calling `adf_apply_contract`, then submit
an outcome for every issued requirement instance. Cite the new Contract in `basis_refs` and
include it in `output_refs`.

Set `evidence_mode` deliberately. Prefer a Contract-level `review` default, then override
only clauses that need stronger proof:

- `direct`: failure can leak data, corrupt state, duplicate an irreversible effect, or the
  clause cannot be established without executing the relevant path
- `inherited`: another reproducible test or probe for the same Change can also establish
  this clause
- `review`: a policy, product direction, or qualitative expectation whose correctness is
  better attacked by the independent Challenger than reduced to a binary artifact

Do not choose `direct` merely to increase coverage. Contracts without this field retain
legacy all-direct behavior, but new Contracts should state the intended mode explicitly.

If the Change request or existing accepted authority does not settle a product or
architecture choice, do not create a rule from inference. Submit a decision request so the
control plane can obtain and record a human decision first.

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

1. Call `adf_apply_decision` to save the Decision carrying the rationale.
2. Call `adf_apply_contract` to update the Contract that now holds the current rule,
   citing that Decision as its authority.
3. Submit the action.

Contract updates use optimistic locking. Send `expected_clause_digests` when you change
individual clauses, `expected_digest` when you replace the whole contract, and
`expected_digest: null` when you create one. Never send both forms at once. A rejection
means someone changed the same clause first: re-read the contract and redo the edit
rather than forcing it through.

Rationale belongs in `decisions/`. The current rule belongs in `contracts/`. The decision
request is temporary - nothing downstream may keep referring to it.

## When the change is blocked instead

`blocked-detection` means the detectors could not account for everything in
scope, so no action is issued until that is resolved. The `diagnostics` name the
gap kind, and the kind decides what you can do:

- `unmapped-observation`, `unsupported-observation`, `unbound-source-artifact`,
  `ambiguous-symbol-binding`, `invalid-binding`: a binding is missing, ambiguous,
  or unreviewed. Run `adf project observe`, review the draft, and promote it.
  This is the resolvable case, and it is the common one.
- `unsupported-language`: the source is in a language this build has no detector
  for - C++ among them. No review resolves it. Either the source moves out of
  `analysis.roots`, or the change waits. Do not report this as a defect and do
  not work around it by weakening what is analyzed.
- `parse-error`: the source is in a supported language but did not parse. Fix
  the source. If it is valid, the parser is wrong and that is worth reporting.

Never satisfy a requirement by narrowing the analyzed scope so a gap disappears.
The gap is the finding.

## Submit and continue

Call `adf_submit` with the action id, the context digest from the action, and the
payload. The control plane validates it, stores it, and returns the next action. Repeat
until it hands the work to another role.

If the orchestrator already exposes execution measurements, include them in the optional
`execution` object: `duration_ms`, `model`, `input_tokens`, `output_tokens`, `tool_calls`,
`retries`, `started_at`, and `completed_at`. Do not run another model call, timer, or tracing
step solely to collect them. ADF records the serialized Context size itself, and absent
measurements remain unknown.

If submission is rejected as stale, the inputs moved under you. Call `adf_next`
again and redo the work against the fresh action - do not retry the old payload.
