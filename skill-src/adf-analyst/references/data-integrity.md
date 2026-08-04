# Data Integrity Assessment

For every write operation, reconstruct:

- preconditions and actor/tenant scope;
- read set and mutation set;
- transaction atomic groups;
- external effects and source of truth;
- postconditions and applicable invariant ids;
- strong or eventual consistency and convergence bound;
- idempotency and duplicate semantics;
- timeout, cancellation, partial success, retry, and compensation;
- concurrent writers, ordering, deletion, migration, and backfill.

Prefer executable enforcement in this order: database constraint, transactional service, idempotent/outbox protocol, invariant test, runtime checker and reconciliation. Documentation alone is not evidence.
