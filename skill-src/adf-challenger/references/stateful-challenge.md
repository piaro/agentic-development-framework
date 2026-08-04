# Stateful Challenge

Use Operation Contracts and the Mutation Graph to generate sequences, not isolated requests.

- create, update, delete, recreate;
- finalize, cancel, retry, finalize again;
- duplicate and reverse order;
- concurrent writers and stale reads;
- fail before commit, after commit, before publish, and after publish before acknowledgement;
- lose, delay, or duplicate an event;
- mix old and new code during migration;
- fail external synchronization and then reconcile.

Check every applicable Data Invariant after each step and after convergence deadlines. Report the shortest reproducible counterexample.
