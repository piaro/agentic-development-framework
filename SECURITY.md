# Security policy

## Reporting a vulnerability

Report privately through GitHub: open the repository's **Security** tab and use
**Report a vulnerability**. That opens a private advisory visible only to you and
the maintainers.

Please do not open a public issue, post the details in a discussion, or send
them in a pull request. Those are all public the moment they are created.

Tell us what you can:

- what an attacker gains, and what they need in order to try
- the version, platform, and whether a published binary or a local build
- the smallest reproduction you have

Expect a first reply within a week. This project is maintained by one person, so
the reply may be an acknowledgement rather than a fix. You will be told when a
fix ships, and credited in the advisory unless you ask otherwise.

## What this project treats as a vulnerability

The kit's job is to keep unverified work from passing as verified, and to keep a
tampered release from being installed. Anything that defeats either is in scope:

- installing a release whose signature, key status, or attestation does not
  verify, or one that a revoked or retired key signed
- causing a change to reach a completed state without the evidence its contracts
  require, or making a stale result count as fresh
- an agent writing outside what its issued action permits, or a submission
  bypassing the optimistic locking that protects contracts and decisions
- escaping the project directory while unpacking a release archive or placing
  files during `project init`
- reading, deriving, or logging a release signing key from anything other than
  the publishing job that holds it

## What it does not

These are known and deliberate, so please do not report them as vulnerabilities:

- **The control plane does not judge meaning.** It checks structure, references,
  state, digests, and coverage. A human or an agent can record a wrong contract,
  a wrong decision, or a summary that misstates what the evidence shows, and the
  kit will accept it. That is the boundary of what it claims.
- **Detectors do not find every risk in the code.** Aliases, dynamic dispatch,
  and dependency injection are not resolved, C++ is not analyzed at all, and
  coverage gaps stop the change rather than being silently ignored. Missing a
  call the detector never claimed to find is a gap to improve, not a
  vulnerability.
- **Anyone who can write to the repository can change its records.** The kit
  protects against work passing unverified, not against a person with commit
  access who intends harm.
- **Framework Releases you sign yourself are yours to protect.** The kit
  verifies signatures against the trust store a project pins. It cannot tell you
  that a key you chose to trust deserved it.
