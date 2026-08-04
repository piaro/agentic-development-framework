# Publishing a release

This is the runbook for whoever holds the signing key. It covers the one-time
setup and the two workflows that produce a published release.

Publishing happens on GitHub. The signing key never leaves the candidate job,
and no step publishes anything without an approval. The one-time setup below is
the exception: it runs on your own machine, from a clone of this repository,
because it is what creates the key in the first place.

## One-time setup

### 1. Create a signing key

The key is an Ed25519 seed: 32 random bytes as 64 hex characters.

```sh
openssl rand -hex 32
```

Keep it somewhere you would keep a password. Anyone holding it can sign a
release that installs on every project pinned to this key.

Derive the public key it corresponds to. You are setting this up before any
release exists, so run it from a clone of this repository rather than from a
published binary:

```sh
cargo build --release
ADF_RELEASE_SIGNING_KEY_HEX=<seed> ./target/release/adf release public-key
```

This prints the public key and nothing else. The seed is never printed and never
written to disk.

Take care with shell history: a seed typed on a command line stays in
`~/.zsh_history` or the equivalent. Reading it from a variable you set in one
step, or from a password manager, avoids that.

### 2. Store them in the repository

| Where | Name | Value |
|---|---|---|
| Secret | `ADF_RELEASE_SIGNING_KEY_HEX` | the seed |
| Variable | `ADF_RELEASE_SIGNING_PUBLIC_KEY_HEX` | the public key |
| Variable | `ADF_RELEASE_SOURCE_ID` | the name releases are signed as coming from, for example `remote:official` |
| Variable | `ADF_RELEASE_SIGNER_KEY_ID` | the key's identifier, for example `framework.release.2026-08` |

Both names are recorded in the framework lock and the trust store and compared
as exact strings, so changing either one later breaks projects that already
pinned them. Choose them once.

The key identifier should distinguish this key from the next one, because
rotation means both exist in the trust store at the same time. A date or a
sequence number does that; `prototype` does not.

The public key is a variable rather than a secret on purpose. The candidate job
checks the signature it produced against it, so a wrong or rotated key stops the
build instead of producing a release nobody trusts.

### 3. Create the approval gate

Create an environment named `vnext-release` and add yourself as a required
reviewer. The publish workflow will not run without it, which is what keeps a
single mistaken dispatch from putting bytes in front of users.

## Publishing

### 1. Build the candidate

Run the **Release candidate** workflow from the default branch. It runs the
regression suite, builds and signs the framework release, builds the binary for
each published platform, attests them, collects the license terms of everything
linked into them, and uploads the whole set as workflow artifacts.

Nothing is published. Stop here and look at what it produced.

### 2. Publish it

Run the **Publish a Framework Release** workflow with the candidate run's ID
and the release tag, which must be `framework-<release_id>`.

It verifies the candidate came from the default branch of this repository, waits
for your approval, downloads the artifacts again after approval, creates a draft
release, downloads what it just uploaded, compares it byte for byte, and only
then makes the release public.

A failure leaves the draft in place rather than deleting it, so you can see what
happened.

## What gets published

| Asset | What it is |
|---|---|
| `adf-<target>[.exe]` | the binary for each platform |
| `adf-<target>[.exe].build.json` | its source revision, digest, size, and compiler version |
| `SHA256SUMS` | checksums for everything above |
| `framework-release.tar` | the signed framework release |
| `candidate-framework.lock` | what a project pins to install it |
| `distribution-trust.json` | the trust store, itself attested |
| `publish-receipt.json` | what the signing produced |
| `publication-record.json` | the digest of every asset above |
| `LICENSE-APACHE`, `LICENSE-MIT`, `THIRD-PARTY-NOTICES.md` | the terms the binaries carry |

The license files are published because the binaries statically link their
dependencies. Several of those dependencies require their terms to travel with
the binary, so publishing without them would not be permitted.

## How a release reaches a project

Two separate things are published: the CLI binary, and the framework release
that a project pins - the rules and schemas the control plane evaluates against.

`bootstrap/install.sh` downloads both from the same GitHub release, verifies
their attestations, and installs them together. `project init` then reads the
framework release from the directory the binary sits in, so a project is set up
without any further download. Updating the framework release later works the
same way: fetch the assets, then `adf release install-archive` and
`adf release switch`.

`adf release fetch` is a separate path that downloads
`<base_url>/<release_id>.tar` over HTTPS. It is the only thing that needs the
source ID resolved to a location, and the project declares that mapping itself:

```yaml
# .adf/release-sources.yaml
schema_version: "1"
sources:
  - id: remote:official
    base_url: https://example.com/adf/releases
```

`project init` does not write that file. Hosting the archives at a stable URL
and pointing projects at it is a deployment choice rather than something the kit
decides, so remote fetching stays opt-in.

## Rotating or revoking a key

The trust store carries each key's status, so a key can be retired or revoked
without invalidating what it signed before. Retire a key when you are replacing
it routinely; revoke it when it may have been exposed - a revoked key stops
releases that are already installed, which is the point.

Rotating means: create the new key, add it to the trust store alongside the old
one, publish a release signed by the new key, then retire the old one once
projects have moved. The details are in `docs/implementation.md`.

## If something goes wrong

- **The candidate job fails on the public key.** The secret and the variable are
  not a pair. Derive the public key from the seed again.
- **The publish job cannot find the candidate.** Artifacts are kept for 14 days.
  After that, build a new candidate.
- **Approval is never requested.** The `vnext-release` environment has no
  required reviewer, so there is nothing to approve.
- **The tag already exists.** Publishing refuses to reuse a tag rather than
  overwriting what is already out there. Pick the next release id.
