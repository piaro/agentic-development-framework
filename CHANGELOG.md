# Changelog

Notable changes, newest first. Versions follow [Semantic Versioning](https://semver.org).

Nothing has been released yet. What 0.1.0 will promise, and what stays free to
change, is written down in `COMPATIBILITY.md`.

## Unreleased

### Added

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
  run. The order of work comes from `agentic next`, not from the skills.
- The Rust implementation is the repository root, so `cargo build` and
  `cargo test` run at the top level. The published binary is named `agentic`.

### Removed

- The Python CLI, its installer, and the contract and evidence templates that
  came with it. Those templates used a record shape the current control plane
  cannot load.
