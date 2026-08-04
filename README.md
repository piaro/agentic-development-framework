# Agentic Development Framework

A control plane for repositories worked on by AI agents. People and agents build
contracts together - what this repository holds true, who decided it, and what
evidence closes it - and every change is worked against them and adds to them.

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

And when someone does decide, the answer usually lives in a conversation that
ends. The next change asks again, or does not ask and answers differently.

## Contracts are the point

A contract is what this repository currently holds true: the rule, who had the
authority to set it, and what evidence closes it. For the deletion question
above, that is three parts: attachments go with the task, the recorded decision
that settled it, and the test that proves it. Contracts are the durable
artifact here. Everything else - the actions, the roles, the checks - exists to
build them, use them, and keep them honest.

**People and agents write them together.** The agent investigates the code,
finds that a rule is missing, lays out the options and their impact, and says
which it would pick. A person decides. That decision is recorded, and the rule
it settled becomes a contract clause citing it as authority. Neither party does
this alone: the agent cannot decide, and the person should not have to
reconstruct the question from scratch.

**Every change is worked against them.** Before implementation, the contracts
that govern the change are resolved and challenged. During it, they are what
the implementation must satisfy. After it, the change completes only when each
clause has evidence traceable to it.

**They grow.** A question answered once does not come back - the next change
that touches deletion finds the clause and resolves against it instead of
asking again. An incident becomes a
clause with a test behind it. The repository ends each change knowing more about
itself than it did before, and that accumulation is the actual product.

They are layered, because a rule that governs one feature and a rule that
governs the system are not the same kind of thing:

```text
project        system-wide invariants, and where a human must decide
  domain       meaning, ownership, relationships, lifecycle
  capability   behaviour shared across features
  architecture standard implementations and dependency rules
  data         states that must hold across every operation sequence
  operation    one read or write: preconditions, effects, failure, retry
feature        what this change adds, and which of the above govern it
```

A feature contract is not a copy of the ones above it. It states the delta and
names which ones apply. When a decision turns out to hold beyond the feature -
say the deletion rule is really "no child record outlives its parent", a fact
about the data rather than about tasks - it belongs in the layer that already
governs that, and moving it there is how the repository stops relearning it.

The alternative is what usually happens: the knowledge lives in whoever was in
the room, the agent re-derives it every session, and the two disagree.

## What makes that trustworthy

Contracts are only worth building if an agent cannot talk its way past the
process that enforces them. Work is issued one action at a time - an agent asks what to do, does
that one thing, submits the result, and asks again - and each of those steps is
decided by the control plane rather than by what the agent remembers.

**The order of work is computed, not prompted.** A kernel derives the next
action from the records in the repository, and the action's identity is a digest
of what it was derived from. An agent cannot skip a step by forgetting one, or
invent one the state does not call for. It also means work survives a crash: a
different process reaches the same action from the same records.

**Authority is a checked property, not a judgement call.** Reporting a
requirement as met requires a clause in an accepted contract, an explicit
requirement in the request, a recorded human decision, or an accepted decision
record. An agent's own reasoning is evidence and never authority - not by
convention, but because the submission is rejected without one of those four.

**What the code does is read, not described.** Detectors parse the actual source
in sixteen languages. A call they cannot account for stops the change instead of
being passed over, and nothing is classified by name: `save` and `execute` mean
different things in different frameworks, so they need a binding a person
reviewed.

**Contracts going stale is a feature.** Each result is bound to digests of what
it was based on, so changing a contract, the code, or the authority behind it
marks the work that depended on it stale and asks for it again. A contract that
drifted from the code does not silently keep passing.

**Nothing reviews its own work.** Implementation and challenge are separate
roles, and a post-build challenge runs in a context that did not build the
change. The rules deciding all of this come from a signed framework release the
project pins - a project cannot quietly widen what counts as verified.

And the boundary, which matters as much as the rest: the control plane checks
structure, references, state, digests, and coverage. It does not judge meaning.
A contract that says the wrong thing is accepted. Knowing exactly what it does
not verify is what makes the rest worth trusting.

## Installing

> No release has been published yet, so today the way to get `adf` is to build
> it. The download below works from the first release onward.

**Build it.** You need Rust 1.89 or newer, and nothing else:

```sh
git clone https://github.com/piaro/agentic-development-framework
cd agentic-development-framework
cargo build --release
```

A project pins a signed framework release - the rules and schemas it is
evaluated against - and a downloaded binary arrives with one beside it. A binary
you built does not, so build one to develop against. The key here is
throwaway; a real one belongs in the publishing job:

```sh
SEED=$(openssl rand -hex 32)
PUBLIC_KEY=$(ADF_RELEASE_SIGNING_KEY_HEX=$SEED ./target/release/adf release public-key)
ADF_RELEASE_SIGNING_KEY_HEX=$SEED \
ADF_RELEASE_SIGNING_PUBLIC_KEY_HEX=$PUBLIC_KEY \
  sh scripts/release-ci.sh
# the release is in dist/framework
```

Then point initialization at it with `--candidate-dir dist/framework`.

**Or download it**, once a release exists. The bootstrap script fetches the
binary for your platform along with the framework release it pins, checks that
GitHub attested both to a build of this repository, and installs them together.
It needs the [GitHub CLI](https://cli.github.com) for that check:

```sh
sh bootstrap/install.sh --tag framework-<release-id>
# add the printed bin directory to PATH
```

It refuses to install anything whose attestation does not verify, so a tampered
or unattested download stops there rather than landing on your machine.
Offline installs, key rotation, and rolling back are in
[`docs/implementation.md`](docs/implementation.md).

Running users need no Python and no Rust toolchain - the binary carries
everything, including the agent skills it places into a project.

## Getting started

You need `adf` and a git repository.

```sh
adf project init --project /path/to/project
```

That places the configuration, the pinned framework release, the three agent
skills, a guide at `docs/adf/README.md`, and a block in `AGENTS.md`. Nothing
existing is overwritten, and nothing is committed for you.

If the repository already has code, let the detectors list what they can see and
review it before it counts:

```sh
adf project observe --project . --output .adf/repository-observation.draft.yaml
# fill in the logical IDs, owners, and the accepted decisions that authorize them
adf project validate-bindings --draft .adf/repository-observation.draft.yaml
adf project promote-bindings --draft .adf/repository-observation.draft.yaml
```

Candidates are never applied on their own. A name that looks like a database
write is a candidate, not a fact.

Then start a change:

```sh
adf change init change.first-feature \
  --title "First feature" \
  --intent "Why this change exists"

adf next change.first-feature
```

From here, `next` says what to do. Agents normally reach it over MCP:

```sh
adf mcp --project /path/to/project
```

## The loop

```text
        adf change init
                │
                ▼
    ┌──▶ adf next ──── one action, with the context for it
    │           │
    │           ├─ Analyst    review detected signals, confirm what the change
    │           │             touches, write contracts, ask a person when no
    │           │             authority settles it, record what they answered
    │           ├─ Builder    implement, then record evidence per clause
    │           └─ Challenger try to falsify it, before and after the build
    │           │
    └───────────┴─ adf submit ──── validated, stored, reevaluated
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

`adf explain <change-id>` says why the change is where it is.

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
| [`docs/concepts.ja.md`](docs/concepts.ja.md) | what it solves and how to use it, in Japanese |
| [`docs/implementation.md`](docs/implementation.md) | the implementation of record, in Japanese |

## License

MIT or Apache-2.0, at your option. See [`LICENSE-MIT`](LICENSE-MIT) and
[`LICENSE-APACHE`](LICENSE-APACHE). Contributions are accepted under both.

Published binaries link their dependencies statically and ship the terms those
dependencies require, as `THIRD-PARTY-NOTICES.md`.
