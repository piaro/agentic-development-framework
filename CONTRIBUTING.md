# Contributing

Thanks for looking. This is a small project maintained by one person, so the
most useful thing you can send is a clear report of something that did not work.

## Before you build something large

Open an issue first and describe what you are trying to do. Some directions are
deliberately not taken, and it is better to find that out before you write the
code than after. See [Out of scope](#out-of-scope).

## Getting set up

You need Rust 1.89 or newer. Nothing else.

```sh
git clone https://github.com/piaro/agentic-development-framework
cd agentic-development-framework
cargo test --locked
```

Before sending a change, run what CI runs:

```sh
sh scripts/tests/test-rust.sh
```

That covers formatting, lint, the unit and integration tests, the detector
quality corpus, the golden expectations, and the signed release and install
flows. It takes a few minutes and builds in release mode, so expect the first
run to be slow.

## What a change should include

- **Tests that would fail without it.** For a bug, a test that reproduces the
  bug first. For behaviour, a test that pins the behaviour rather than the
  implementation.
- **Documentation in the same change.** The CLI, the schemas, the skills, the
  README, and the tests describe one system; leaving one behind is how they
  drift apart.
- **A commit message that says why.** Follow
  [Conventional Commits](https://www.conventionalcommits.org) for the subject,
  then spend the body on the reason rather than the diff. A reader who
  disagrees with the change should be able to tell what you were solving.

## Where things are

| Path | What is there |
|---|---|
| `Cargo.toml`, `src/`, `tests/` | the implementation; the binary is named `adf` |
| `schemas/` | the record and output schemas, also carried in a framework release |
| `skill-src/`, `templates/` | what the binary embeds and `project init` places |
| `scripts/` | release helpers, with the acceptance tests in `scripts/tests/` |
| `bootstrap/` | the install scripts a published binary is fetched by |
| `testdata/` | golden expectations, fixtures, and the detector quality corpora |
| `docs/` | the implementation of record and the design history |

## Things worth knowing about the code

- **Golden expectations are contracts, not snapshots.** `testdata/golden/`
  pins behaviour that must not drift. If a change alters them, the change is
  either wrong or is a deliberate protocol change that belongs in its own
  commit with the reasoning written down.
- **The control plane never decides meaning.** It checks structure, references,
  state, digests, and coverage. Anything that makes it infer a product decision,
  classify an ambiguous API by name, or accept a candidate without a reviewed
  binding will be turned down, however convenient it is.
- **Detectors are table-driven and share one contract.** A new language plugs
  into `src/source_detection.rs` and is expected to pass the cross-language
  conformance tests. Partial support that reports its own coverage gaps is fine;
  partial support that stays quiet about them is not.
- **`project init` places only what the current record shape can load.** A
  template that a project cannot read leaves that project unable to load its own
  contracts, so additions there need a test proving they load.

## Out of scope

- **Automating semantic judgement.** Choosing among valid product options,
  deciding whether a rule governs a change, or accepting risk stays with people.
- **Letting a project supply its own detection catalog or executable rules.**
  Detection catalogs come from signed Framework Releases. This is what keeps a
  compromised or careless project from widening what counts as verified.
- **Weakening a check to make a workflow smoother.** If a check is wrong, argue
  that it is wrong. Removing friction is not on its own a reason.

## Licensing

Contributions are accepted under the same terms as the project: MIT or
Apache-2.0, at the user's option. By sending a pull request you agree that your
contribution may be distributed under both, with no additional conditions.
