# Challenge Method

## Contract / authority before implementation

- Compare the request and non-scope with every proposed Contract addition.
- Find missing decisions before reviewing the authority recorded for existing decisions.
- Try to construct a narrower decision than the proposed one; broader semantics require explicit support.
- Check new entities, records, interfaces, required inputs, roles, tenant boundaries, lifecycle, retention, deletion, protocol, errors, idempotency, and external effects.
- Treat findings, gaps, code, and tests as evidence only. When no authority selects among valid product choices, require a Decision Request.
- Bind the result to fresh Change, Assessment, and semantic Contract hashes before resolution.

## Governance

- Compare declared impact with changed files, schemas, queries, routes, events, and external calls.
- Ask whether each new rule affects another feature or future reference implementation.
- Check that all upper contracts are accepted and the resolved lock is fresh.

## Implementation

- Test empty, boundary, duplicate, malformed, unauthorized, and cross-tenant inputs.
- Test cancellation, timeout, delayed success, retry, and partial failure.
- Check serialization, storage, wire type, compatibility, and architecture boundaries.

## Completion

- Require traceable evidence for each contract clause.
- Treat mocks as insufficient evidence for platform connectivity and actual type behavior.
- Require acceptor and due date for every residual risk.
