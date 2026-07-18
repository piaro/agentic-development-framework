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
