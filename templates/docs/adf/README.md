# Agentic Development

This repository keeps its current rules in `contracts/`, the reasons behind
those rules in `decisions/`, and the facts that verify them in `evidence/`.

## Start a change

```sh
adf change init <change-id> --title "Change title" --intent "Why it exists"
```

## Follow the issued workflow

`adf next <change-id>` returns one action and the Context required for it. Do
that action, submit its Result, and ask again. Do not reconstruct the workflow
outside ADF.

```sh
adf next <change-id>
```

Agents normally use MCP. Start `adf mcp`, then use `adf_next` and `adf_submit`.

Use the Skill for the role in the issued action.

| Role | Skill | Work |
|---|---|---|
| Analyst | `$adf-analyst` | Assess intended impact, review detected candidates, write Contracts, and raise and record decisions |
| Builder | `$adf-builder` | Implement and record evidence for Contract clauses |
| Challenger | `$adf-challenger` | Try to falsify the change before and after implementation |

A post-build challenge must use a context independent from the implementation
context.

Run `adf explain <change-id>` to see why the change is in its current state.

## Impact assessment

Every new Change starts with `assess-change-impact`. It classifies the outcome
as `impacts-identified`, `no-impact`, or `inconclusive`. An empty repository is
not automatically `no-impact`: the Analyst derives intended effects from the
Change request and creates only the minimum governance needed for those effects.

The assessment receives a compact repository, Contract, and Decision index. It
may reuse up to three prior assessments after inputs change. Later actions
receive the accepted assessment and only matching governance and artifacts,
so they do not repeat repository-wide investigation.

## Authority

Only an accepted Contract, an explicit requirement in the request, a recorded
human decision, or an accepted Decision can authorize a specification. Agent
inference, challenge findings, missing Contracts, source code, and tests are
evidence, not authority.

When no authority settles a question, return it to a person with options,
impact, a recommendation, and the required decision-maker. After the answer,
record the rationale in `decisions/` and the current rule in `contracts/`.

Do not use a Feature Contract to change a higher-level rule implicitly. Stop
the change and settle the governing Contract first.

## Cost visibility

Each action includes advisory model guidance. The orchestrator may use an
economy model for impact assessment and should escalate when the action lists
an escalation condition. ADF does not select or invoke the model.

The `adf_submit` call may include execution time, model, token counts, tool
calls, and retry counts already known to the orchestrator. ADF records the
serialized Context size itself. It does not start timers, call a model, or run
extra tracing to collect these values. Use `adf_execution_log` or
`adf execution-log <change-id>` to read the stored totals; missing values remain
unknown rather than estimated.
