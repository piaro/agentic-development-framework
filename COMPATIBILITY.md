# Compatibility

This says what you can build on and what may move under you.

## Versioning

Releases are `0.MINOR.PATCH` while the kit is below 1.0.

- **Patch releases** (`0.1.0` → `0.1.1`) never break the stable surface below.
- **Minor releases** (`0.1.x` → `0.2.0`) may break it. Every break is listed in
  `CHANGELOG.md` with what changed and what to do about it.

Below 1.0 that is the whole promise: a break is allowed, but it is never silent.

## Stable surface

A change here is a break and needs a minor release.

**Commands and their arguments**

`project init`, `project observe`, `project validate-bindings`,
`project promote-bindings`, `change init`, `next`, `explain`,
`execution-log`, `contract-health`, `mcp`, `release`, `binary`.

**Stored records**

The shapes of Change, Contract, Decision, Result, and Evidence records, and the
project files `.adf/config.yaml`, `.adf/framework.lock`, and
`.adf/repository-observation.yaml`. A project written by one release must
keep loading in later ones.

**Machine-readable output**

The `--format json` output of the stable commands: the next response, the
explain report, the binding validation report, and the contract health report
and gate report. The contract health policy file a project owns is part of this.

**MCP interface**

Tool names, their input and output schemas, the protocol version, and the error
codes. An agent written against one release keeps working with the next.

Optional execution measurements are observational metadata. Adding or omitting
them does not change Result identity, freshness, or the next workflow action.

**Distribution**

Published asset names, the signed release manifest, the trust store, the
framework lock, the publication and build records, and attestation. An installed
binary must be able to verify and install what a later release publishes, or the
break is called out.

## Experimental surface

These ship and are useful, but they may change in any release, including a patch
release, and the change may not be called out.

- **Migration.** `migration inspect`, `draft`, `validate-draft`,
  `generate-candidate`, `validate-candidate`, `apply-candidate`, and all their
  formats. Migrating an existing project is not what the first release is built
  for, and this is where the shape is still being learned.
- **Detector quality tooling.** `benchmark`, `detector-audit`,
  `detector-audit-check`, the corpus formats, and the audit baselines. These
  exist to develop the kit rather than to run a project.
- **The Framework Detection Catalog format.** The rules a signed Framework
  Release carries for framework-specific APIs.
- **`catalog signal-domains`** output.

## Not an interface at all

Do not build on these. They change whenever there is a reason to change them.

- **The Rust library.** The crate is not published, and its modules and types
  are internal. Only the binary is an interface.
- **Human-readable output.** The text `next` and the other commands print, and
  the wording of diagnostics and errors. The error *codes* are stable; the
  sentences are not.
- **Anything under `testdata/`.** Golden expectations and fixtures pin the
  kit's own behaviour and have no meaning outside its test suite.

## What counts as a break

- Removing or renaming a command, an argument, an MCP tool, or an error code
- Requiring a field that was optional, or removing one that was required
- Changing a stored record shape so that an existing project stops loading
- Renaming a published asset, or changing how a release is verified, so that an
  installed binary can no longer install the next one

## What does not

- Adding a command, an argument, an optional field, or a new error code
- Changing wording, ordering, or formatting of human-readable output
- **Detection results changing as detectors improve.** A release may observe
  calls an earlier one missed, which can surface new candidates and stop a
  change that previously proceeded. This is the kit doing its job, not a
  compatibility break - but it is why a release is worth reading the changelog
  for before rolling it out across a repository.
- Fixing a check that was wrong, even when work that used to pass now stops
