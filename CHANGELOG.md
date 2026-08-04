# Changelog

Notable changes, newest first. Versions follow [Semantic Versioning](https://semver.org).

Nothing has been released yet. Until 0.1.0, the CLI surface and the stored
record shapes may change without notice; see the compatibility section of the
README for what will be promised once it ships.

## Unreleased

### Added

- `project init` places the agent skills and the project guide from inside the
  binary, and appends its guidance block to `AGENTS.md`. A downloaded binary is
  now enough to work a change end to end.
- Continuous integration runs the regression suite on Linux and macOS with the
  minimum supported Rust toolchain, and checks that the third-party license
  notices can still be produced.
- MIT and Apache-2.0 licensing, with a generator for the third-party notices
  that ship alongside the published binaries.

### Changed

- Work submitted after a restart or a dropped connection is accepted when the
  control plane would still issue that same action, rather than being refused
  because the process had forgotten it. An action that is no longer current is
  refused as `ACTION_NOT_CURRENT` and names the action to work on instead.

- The three agent skills are organised by the role the control plane assigns -
  analyst, builder, challenger - rather than by the command an agent used to
  run. The order of work comes from `agentic next`, not from the skills.
- The Rust implementation is the repository root, so `cargo build` and
  `cargo test` run at the top level. The published binary is named `agentic`.

### Removed

- The Python CLI, its installer, and the contract and evidence templates that
  came with it. Those templates used a record shape the current control plane
  cannot load.
