# Changelog

Notable changes, newest first. Versions follow [Semantic Versioning](https://semver.org).

Nothing has been released yet. What 0.1.0 will promise, and what stays free to
change, is written down in `COMPATIBILITY.md`.

## Unreleased

### Changed

- Accepted Decisions now remain available to later Changes, while proposed,
  rejected, and superseded Decisions remain scoped to their originating Change.
  Repository bindings and shared Contracts can therefore retain their recorded
  authority after the originating Change is complete.

- Requirement dependencies now apply only to dependency instances selected for
  the current Change. An unselected Signal-specific dependency no longer leaves
  an otherwise valid workflow permanently blocked, while every selected
  dependency instance must still be satisfied.

- The project is the Agentic Development Framework. The binary is `adf`,
  projects keep their records in `.adf/`, the MCP tools are `adf_*`, the skills
  are `adf-analyst`, `adf-builder`, and `adf-challenger`, and the release
  environment variables use the `ADF_` prefix. Renaming after the first release
  would have broken the asset names this promises to keep stable, so it
  happened before there was anything to break.
- Migration now reads the retired CLI's project from `.agentic/` and writes this
  framework's project to `.adf/`. They used to be the same directory, so a
  project part way through migration is a state that can now be recognized
  rather than a config that parses as neither format.

### Added

- Every newly created Change now starts with an intent-first Impact Assessment.
  It distinguishes identified impact, no impact, and an inconclusive assessment,
  and gives empty greenfield repositories a bootstrap path that creates only the
  governance needed for the first Change.
- Action-specific Contexts reuse accepted Impact Assessment Results and select
  matching governance and repository artifacts for implementation. Each action
  also exposes an advisory model tier and explicit escalation conditions.
- Result submissions may carry execution measurements already known to the
  orchestrator. `execution-log` and the read-only `adf_execution_log` MCP tool
  aggregate them with measured Context sizes without invoking a model, starting
  a timer, or estimating missing data. Execution metadata does not affect Result
  identity, freshness, or workflow decisions.

- English documentation: the README is the entry point, `docs/limits.md` states
  which languages are analyzed and which calls are not resolved, and the
  Japanese material moves to `docs/concepts.ja.md`. Where the two disagree, the
  English is authoritative.

- Published releases carry `LICENSE-APACHE`, `LICENSE-MIT`, and
  `THIRD-PARTY-NOTICES.md` alongside the binaries. The binaries link their
  dependencies statically, and several of those require their terms to travel
  with the binary.
- `adf release public-key` derives the public key of a signing seed, which
  publishing pins so a wrong or rotated key stops the build. There was
  previously no way to obtain it.
- `docs/publishing.md` is the runbook for whoever holds the signing key.

- `COMPATIBILITY.md` states what the first release promises: the everyday
  commands, the stored record shapes, the machine-readable output, the MCP
  interface, and the distribution formats are stable, while migration and the
  detector quality tooling may change in any release. `--help` separates the
  two so the distinction is visible where it is read.

- `project init` places the agent skills and the project guide from inside the
  binary, and appends its guidance block to `AGENTS.md`. A downloaded binary is
  now enough to work a change end to end.
- Continuous integration runs the regression suite on Linux and macOS with the
  minimum supported Rust toolchain, and checks that the third-party license
  notices can still be produced.
- MIT and Apache-2.0 licensing, with a generator for the third-party notices
  that ship alongside the published binaries.

### Changed

- A change stopped by a coverage gap now says which kind of stop it is. A
  language with no detector is called out as a limit of the kit rather than
  something a binding review can fix, and a source that failed to parse points
  at the source. Previously every stop read the same.
- Work submitted after a restart or a dropped connection is accepted when the
  control plane would still issue that same action, rather than being refused
  because the process had forgotten it. An action that is no longer current is
  refused as `ACTION_NOT_CURRENT` and names the action to work on instead.
- The three agent skills are organised by the role the control plane assigns -
  analyst, builder, challenger - rather than by the command an agent used to
  run. The order of work comes from `adf next`, not from the skills.
- The Rust implementation is the repository root, so `cargo build` and
  `cargo test` run at the top level. The published binary is named `adf`.

### Removed

- The Python CLI, its installer, and the contract and evidence templates that
  came with it. Those templates used a record shape the current control plane
  cannot load.
