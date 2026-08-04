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

## Authority and decision requests

A reason explains a decision; authority permits it. An outcome you report as `satisfied` must rest on one of:

- an explicit clause in an existing accepted Contract;
- an explicit requirement in the request the change came from;
- a recorded human decision;
- an existing accepted Decision record.

Agent inference, challenger findings, contract gaps, implementation convenience, existing code, and tests are evidence only. They may open a decision request but cannot resolve it.

Raise a decision request when existing authority cannot select the product or architecture rule. Carry the question, the known facts, the options and their impact, your recommendation, and the authority required to decide. The change then waits in `needs-human-decision`.

After the answer is recorded, durable rationale lives in `decisions/` and the current rule lives in `contracts/`. Nothing downstream may keep referring to the decision request itself.

## Contract relevance

Context, entity, capability, and path overlaps surface candidate contracts but do not prove that a contract governs this change. Decide each candidate deliberately: either it governs the change, or the overlap is incidental for a specific reason you can state.

Operation identity and interface identity are structural requirements, as are explicit references from the feature contract. Mechanical matching narrows what you read; it does not replace reading it.
