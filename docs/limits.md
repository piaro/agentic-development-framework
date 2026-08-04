# What is analyzed, and what is not

The kit stops a change when it cannot account for something, rather than passing
over it. That makes knowing where the edges are worth reading before you hit one:
a stop is usually a gap you can close, and sometimes a limit no amount of review
will resolve.

## Languages

Sixteen languages have a detector. Their calls are read from the syntax, not from
names in text or comments.

| Language | Extensions |
|---|---|
| Python | `.py` |
| JavaScript | `.js`, `.mjs`, `.cjs` |
| JSX | `.jsx`, and JSX inside `.js` |
| TypeScript | `.ts`, `.mts`, `.cts` |
| TSX | `.tsx` |
| Java | `.java` |
| Kotlin | `.kt`, `.kts` |
| Go | `.go` |
| Rust | `.rs` |
| Ruby | `.rb` |
| PHP | `.php` |
| C# | `.cs` |
| Swift | `.swift` |
| Scala | `.scala`, `.sc` |
| C | `.c`, `.h` |
| GDScript | `.gd` |

Extensions are matched without regard to case.

**C++ is listed but not analyzed.** `.cc`, `.cpp`, `.cxx`, `.hh`, `.hpp`, and
`.hxx` are recognized as source, so they are never silently skipped, and a change
that includes them stops with `unsupported-language`. There is no detector behind
them. This is a decision, not an oversight: C++ needs real name resolution to say
anything trustworthy about a call, and a detector that guesses would be worse
than none.

If a language you need is missing, the change stops until either the source moves
out of `analysis.roots` or a detector exists. Narrowing the analyzed scope so the
gap disappears is the one thing not to do - the gap is the finding.

## Calls that are not resolved

Within a supported language, some calls cannot be attributed with confidence.
These are reported as coverage gaps rather than guessed at:

- **Aliases.** A resource reached through a local variable that was assigned from
  another expression is not traced back to its origin.
- **Dynamic dispatch.** A call whose receiver is chosen at runtime is observed as
  an unclassified call rather than attributed to what it will reach.
- **Dependency injection.** A resource supplied by a container or a framework is
  not resolved to the concrete type behind it.
- **Wrappers.** A call that reaches a database or a queue through the project's
  own indirection is attributed to the wrapper, not to what it wraps. Binding the
  wrapper is the way through.
- **Computed properties.** `client["send"]` is read; `client[method]` is not.

Every one of these is a gap to improve. Reporting one is useful; reporting it as
though the kit claimed to handle it is not.

## Names the kit will not interpret

Some method names mean different things in different frameworks, and guessing
from the name is how a control plane quietly stops being trustworthy.

- `save`, `send`, `execute`, and similar names are not classified by name alone.
  They need a binding that records what this resource and method actually do,
  who owns that answer, and the accepted decision that authorizes it.
- Django's `.save()`, SQLAlchemy's `execute`, and the JavaScript S3 client's
  `send` are deliberately left unclassified. `execute` reads and writes; which
  one it is here is not something a name can tell you.
- Authorization boundaries and data sensitivity are never inferred. A method is
  a security-relevant call because a reviewed binding says so.

Framework candidates are proposed for the major ORMs, message brokers, HTTP
clients, and object storage SDKs, but a candidate is a starting point for review
and is never applied on its own.

## When a change stops

`blocked-detection` means the detectors could not account for everything in
scope. What to do depends on the gap kind, which the diagnostics name:

| Kind | What it means | What resolves it |
|---|---|---|
| `unmapped-observation` | a call was seen but has no logical identity | review and promote a binding |
| `unsupported-observation` | the receiver is bound, the method is not classified | bind the method explicitly |
| `unbound-source-artifact` | a source file is not declared in the observation | add it, or narrow the analyzed scope |
| `ambiguous-symbol-binding` | two symbols share a short name | use the qualified key |
| `invalid-binding` | a binding is missing an owner or an accepted decision | complete it |
| `unsupported-language` | no detector for this language | nothing - see above |
| `parse-error` | a supported language that did not parse | fix the source; if it is valid, report it |

## Boundaries of what the kit verifies

- **It does not judge meaning.** A contract that says the wrong thing, a decision
  recorded against the wrong authority, or a summary that misstates what the
  evidence shows will all be accepted. Structure, references, state, digests, and
  coverage are what get checked.
- **Detection quality is measured against reviewed corpora, not against your
  repository.** The benchmarks report perfect precision and recall on the corpus
  that has been reviewed, which says the detectors do what they were built to do
  on those inputs. It is not a claim about a repository nobody has looked at.
- **One project per process.** An MCP server is fixed to one project root at
  startup.
- **Local stdio only.** There is no remote MCP server; authentication and tenant
  separation are not designed yet.
- **Canonical JSON handles integers only.** Floating-point values are rejected
  rather than normalized across languages, because the normalization is not
  defined.
