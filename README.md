# Agentic Development Kit

A control plane for repositories worked on by AI agents. It decides what happens
next in a change, and refuses to call that change done until the evidence its
contracts require actually exists.

日本語の解説は [`docs/concepts.ja.md`](docs/concepts.ja.md) にあります。

## The problem

An agent asked to implement something will implement something. What it will not
do reliably is notice that the specification it needs was never written down, and
stop. It fills the gap - plausibly, invisibly, and in code that nobody reviews as
a decision, because it does not look like one.

The gap is not the agent's to fill. Whether deleting a task should cascade to its
attachments is a product decision. An agent can find that the question exists,
lay out the options, and say which it would pick. It cannot be the one who
decides, and no amount of prompting makes it the right party.

## What this does

Work is issued one action at a time. An agent asks what to do, does that one
thing, submits the result, and asks again. What it may do is decided by the
control plane, not by what the agent remembers:

- A decision with no authority behind it stops the change and goes to a person,
  with the options and their impact.
- Contracts, decisions, evidence, and results are records in the repository, so
  a decision made in one change is available to the next one.
- Implementation and challenge run in separate contexts, so nothing reviews its
  own work.
- A change completes when each contract clause has evidence traceable to it.

The control plane checks structure, references, state, digests, and coverage. It
does not judge meaning: a wrong contract, recorded by a person or an agent, is
accepted. That boundary is deliberate and is what keeps it honest about what it
verifies.

## Getting started

You need the `agentic` binary and a git repository. Nothing else - no Python, no
Rust toolchain.

```sh
agentic project init --project /path/to/project
```

That places the configuration, the pinned framework release, the three agent
skills, a guide at `docs/agentic/README.md`, and a block in `AGENTS.md`. Nothing
existing is overwritten, and nothing is committed for you.

If the repository already has code, let the detectors list what they can see and
review it before it counts:

```sh
agentic project observe --project . --output .agentic/repository-observation.draft.yaml
# fill in the logical IDs, owners, and the accepted decisions that authorize them
agentic project validate-bindings --draft .agentic/repository-observation.draft.yaml
agentic project promote-bindings --draft .agentic/repository-observation.draft.yaml
```

Candidates are never applied on their own. A name that looks like a database
write is a candidate, not a fact.

Then start a change:

```sh
agentic change init change.first-feature \
  --title "First feature" \
  --intent "Why this change exists"

agentic next change.first-feature
```

From here, `next` says what to do. Agents normally reach it over MCP:

```sh
agentic mcp --project /path/to/project
```

## The loop

```text
        agentic change init
                │
                ▼
    ┌──▶ agentic next ──── one action, with the context for it
    │           │
    │           ├─ Analyst    review detected signals, confirm what the change
    │           │             touches, write contracts, ask a person when no
    │           │             authority settles it, record what they answered
    │           ├─ Builder    implement, then record evidence per clause
    │           └─ Challenger try to falsify it, before and after the build
    │           │
    └───────────┴─ agentic submit ──── validated, stored, reevaluated
                │
                ▼
          ready to merge
```

Three skills, one per role, are placed into the project by `project init`. The
order of work is not in them - it comes from `next`. A challenge after the build
runs in a context independent of the one that built it.

| State | What is assigned |
|---|---|
| `needs-analysis` | review detected candidates, answer the requirements |
| `needs-human-decision` | put the question to a person |
| `needs-decision-recording` | record their answer as a decision and a contract |
| `needs-pre-build-challenge` | falsify the request, the authority, the contracts |
| `ready-to-build` | implement |
| `needs-evidence` | record evidence for each clause |
| `needs-post-build-challenge` | falsify the implementation |
| `ready-to-merge` | nothing |

`agentic explain <change-id>` says why the change is where it is.

## Commands

| Command | What it does |
|---|---|
| `project init` | set a repository up |
| `project observe` | list what the detectors can see, for review |
| `project validate-bindings` | report missing bindings and what cannot be checked |
| `project promote-bindings` | make a reviewed draft the observation of record |
| `change init` | start a change |
| `next` | issue the next action |
| `explain` | say why the change is where it is |
| `contract-health` | check the contracts across the repository |
| `mcp` | serve the same operations to agents over MCP |
| `release` | build, fetch, install, switch, and roll back framework releases |
| `binary` | install, update, and roll back the CLI itself |

`migration`, `benchmark`, `detector-audit`, and `catalog` also exist and are
experimental - see [`COMPATIBILITY.md`](COMPATIBILITY.md).

## What it supports, and what it does not

Detectors read the source and report what they cannot account for rather than
passing over it. See [`docs/limits.md`](docs/limits.md) for the supported
languages, the calls that are not resolved, and what a blocked change means.

The short version: sixteen languages have detectors, C++ deliberately does not,
aliases and dynamic dispatch are not resolved, and a gap stops the change instead
of being ignored.

## Documentation

| Document | What is in it |
|---|---|
| [`docs/limits.md`](docs/limits.md) | supported languages, known gaps, what a stop means |
| [`COMPATIBILITY.md`](COMPATIBILITY.md) | what is promised and what may change |
| [`docs/publishing.md`](docs/publishing.md) | releasing, for whoever holds the signing key |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | building, testing, and what is out of scope |
| [`SECURITY.md`](SECURITY.md) | reporting a vulnerability, and what counts as one |
| [`docs/concepts.ja.md`](docs/concepts.ja.md) | the contract hierarchy and data integrity model, in Japanese |
| [`docs/implementation.md`](docs/implementation.md) | the implementation of record, in Japanese |

## License

MIT or Apache-2.0, at your option. See [`LICENSE-MIT`](LICENSE-MIT) and
[`LICENSE-APACHE`](LICENSE-APACHE). Contributions are accepted under both.

Published binaries link their dependencies statically and ship the terms those
dependencies require, as `THIRD-PARTY-NOTICES.md`.
