# Contract Governance Rules

## Promote beyond Feature Contract when

- more than one feature, phase, route, job, or consumer uses the rule;
- ownership, identity, cardinality, lifecycle, history, retention, or deletion is decided;
- a context translation, public interface, storage format, or protocol is decided;
- tenant, authorization, privacy, money, publication, or irreversible action is affected;
- a shared platform capability or reference implementation is selected;
- the decision is high impact even if it currently has one caller.

## Contract kinds

- `project`: system-wide invariants and human decision boundaries
- `domain`: business meaning, ownership, relationships, lifecycle
- `capability`: behavior shared by features
- `architecture`: standard implementation and dependency rules
- `data-invariants`: valid states across every operation sequence
- `operation`: preconditions, mutations, transaction, effects, postconditions
- `feature`: current change delta; must not override an upper contract

## Assessment outcomes

- `ready`: all governing contracts are accepted and no unresolved item remains
- `blocked-contract-decision`: an upper rule is missing or unresolved
- `blocked-platform-probe`: required platform behavior is unverified
- `blocked-conflict`: contracts or active mutations conflict

Record options, impact, recommendation, and unknowns before asking a human to decide.

## Authority and Decision Requests

A reason explains a decision; authority permits it. A resolved or accepted Assessment decision must cite one of:

- an explicit clause in an existing accepted Contract;
- an explicit Issue requirement;
- a recorded human decision;
- an existing accepted Decision record.

Agent inference, Challenger findings, contract gaps, implementation convenience, existing code, and tests are evidence only. They may open a Decision Request but cannot resolve it.

Use `needs-human-decision` when existing authority cannot select the product or architecture rule. Keep the temporary question, discovery role/reference, options, impact, recommendation, and required decider in the Assessment. After resolution, keep durable rationale in `decisions/` and current truth in `contracts/`; downstream artifacts must not depend on the Decision Request.

Before Contract resolution, require a fresh Contract / Authority Challenge. Its findings can block or reopen Assessment but cannot authorize a specification.

## Contract candidate decisions

Context, entity, capability, path, and similar overlaps discover candidates but do not prove semantic relevance. Record each candidate as:

- `included`: the contract governs this change;
- `excluded`: the overlap is incidental, with a specific reason.

Operation identity and interface identity are structural requirements. Explicit Feature or Assessment references are also required. Do not use the CLI's mechanical matching as a substitute for repository analysis.
