import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Agentic Development Framework — Decisions agents can’t invent",
  description:
    "A repository control plane that connects specifications, human decisions, implementation, and evidence for AI-assisted development.",
};

const githubUrl = "https://github.com/piaro/agentic-development-framework";

function Arrow() {
  return <span aria-hidden="true">↗</span>;
}

export default function Home() {
  return (
    <main>
      <header className="site-header">
        <a className="brand" href="#top" aria-label="ADF home">
          <span className="brand-mark" aria-hidden="true">
            A
          </span>
          <span>
            Agentic Development
            <small>Framework</small>
          </span>
        </a>

        <nav aria-label="Main navigation">
          <a href="#problem">Why ADF</a>
          <a href="#workflow">How it works</a>
          <a href="#guarantees">Guarantees</a>
        </nav>

        <a className="header-cta" href="#start">
          Get started <span aria-hidden="true">↓</span>
        </a>
      </header>

      <section className="hero section-shell" id="top">
        <div className="hero-copy">
          <p className="eyebrow">
            <span className="status-dot" aria-hidden="true" /> Open-source
            control plane for agentic development
          </p>
          <h1>
            AI agents should implement decisions.
            <em>Not make them.</em>
          </h1>
          <p className="hero-lede">
            ADF turns product rules, human decisions, implementation, and proof
            into durable contracts that every agent must follow.
          </p>
          <div className="hero-actions">
            <a className="button button-primary" href="#workflow">
              See the workflow <span aria-hidden="true">→</span>
            </a>
            <a className="button button-secondary" href={githubUrl}>
              View on GitHub <Arrow />
            </a>
          </div>
          <p className="hero-note">
            Built in Rust · Works with Codex and Claude Code · Apache-2.0 / MIT
          </p>
        </div>

        <div className="workflow-card" aria-label="ADF change workflow preview">
          <div className="terminal-bar">
            <div className="terminal-dots" aria-hidden="true">
              <span />
              <span />
              <span />
            </div>
            <span>change.checkout-timeout</span>
            <span className="terminal-live">live</span>
          </div>
          <div className="workflow-body">
            <p className="command">$ adf next change.checkout-timeout</p>
            <div className="action-card active-action">
              <span className="step-icon">01</span>
              <div>
                <small>ANALYST · IMPACT ASSESSMENT</small>
                <strong>Find what this change could affect</strong>
                <p>2 contracts apply · 1 decision is missing</p>
              </div>
              <span className="action-state">active</span>
            </div>
            <div className="decision-card">
              <div className="decision-heading">
                <span>Human decision required</span>
                <span className="decision-badge">STOPPED SAFELY</span>
              </div>
              <p>
                Should a timed-out checkout release its reserved inventory?
              </p>
              <div className="decision-options">
                <span>A · Release immediately</span>
                <span>B · Hold for recovery</span>
              </div>
            </div>
            <div className="action-card muted-action">
              <span className="step-icon">02</span>
              <div>
                <small>BUILDER</small>
                <strong>Implement against the contract</strong>
              </div>
              <span className="lock" aria-label="Waiting">
                ◇
              </span>
            </div>
            <div className="action-card muted-action">
              <span className="step-icon">03</span>
              <div>
                <small>CHALLENGER</small>
                <strong>Try to falsify the result</strong>
              </div>
              <span className="lock" aria-label="Waiting">
                ◇
              </span>
            </div>
          </div>
          <div className="workflow-footer">
            <span>Deterministic next action</span>
            <span>Crash-safe</span>
            <span>Human authority</span>
          </div>
        </div>
      </section>

      <section className="proof-strip" aria-label="Framework highlights">
        <div>
          <strong>16</strong>
          <span>languages parsed</span>
        </div>
        <div>
          <strong>3</strong>
          <span>independent roles</span>
        </div>
        <div>
          <strong>1</strong>
          <span>action at a time</span>
        </div>
        <div>
          <strong>0</strong>
          <span>silent assumptions</span>
        </div>
      </section>

      <section className="problem-section section-shell" id="problem">
        <div className="section-heading">
          <p className="eyebrow muted">The problem</p>
          <h2>An agent will fill every gap you leave.</h2>
          <p>
            Missing specifications rarely look like errors. They become
            plausible code—and invisible product decisions.
          </p>
        </div>

        <div className="comparison-grid">
          <article className="comparison-card without-card">
            <div className="card-label">
              <span className="bad-dot" aria-hidden="true" /> Without ADF
            </div>
            <div className="chat-stack">
              <div className="chat-bubble request">
                Delete the task and its attachments.
              </div>
              <div className="chat-bubble agent">
                Done. I added cascading deletion.
              </div>
            </div>
            <div className="hidden-decision">
              <span>Hidden decision</span>
              <strong>Attachments should always be deleted.</strong>
              <p>No owner · no recorded reason · no durable proof</p>
            </div>
            <p className="card-summary">
              The conversation ends. The assumption remains in code.
            </p>
          </article>

          <article className="comparison-card with-card">
            <div className="card-label">
              <span className="good-dot" aria-hidden="true" /> With ADF
            </div>
            <div className="contract-preview">
              <div className="contract-topline">
                <span>contract.task-lifecycle</span>
                <span>accepted</span>
              </div>
              <p className="contract-rule">
                No attachment outlives its parent task.
              </p>
              <dl>
                <div>
                  <dt>Authority</dt>
                  <dd>decision.task-deletion</dd>
                </div>
                <div>
                  <dt>Evidence</dt>
                  <dd>test_delete_cascades</dd>
                </div>
                <div>
                  <dt>Mode</dt>
                  <dd>direct</dd>
                </div>
              </dl>
            </div>
            <p className="card-summary">
              The decision survives the session—and governs the next change.
            </p>
          </article>
        </div>
      </section>

      <section className="workflow-section" id="workflow">
        <div className="section-shell">
          <div className="section-heading split-heading">
            <div>
              <p className="eyebrow muted">How it works</p>
              <h2>One change. Three accountable roles.</h2>
            </div>
            <p>
              The control plane decides the order. Each role receives only the
              context and authority it needs.
            </p>
          </div>

          <div className="role-grid">
            <article className="role-card analyst-card">
              <span className="role-number">01</span>
              <div className="role-symbol" aria-hidden="true">
                ?
              </div>
              <h3>Analyst</h3>
              <p>
                Finds affected rules, exposes missing decisions, and prepares
                the question for a person to answer.
              </p>
              <ul>
                <li>Assess intended impact</li>
                <li>Resolve governing contracts</li>
                <li>Record human authority</li>
              </ul>
            </article>

            <article className="role-card builder-card">
              <span className="role-number">02</span>
              <div className="role-symbol" aria-hidden="true">
                +
              </div>
              <h3>Builder</h3>
              <p>
                Implements only after the rules are settled, then connects each
                affected clause to evidence.
              </p>
              <ul>
                <li>Build against accepted rules</li>
                <li>Keep changes inside scope</li>
                <li>Register reproducible proof</li>
              </ul>
            </article>

            <article className="role-card challenger-card">
              <span className="role-number">03</span>
              <div className="role-symbol" aria-hidden="true">
                ×
              </div>
              <h3>Challenger</h3>
              <p>
                Reviews in an independent context and actively tries to disprove
                the change before it can finish.
              </p>
              <ul>
                <li>Challenge before the build</li>
                <li>Falsify claims after the build</li>
                <li>Block uncovered clauses</li>
              </ul>
            </article>
          </div>

          <div className="loop-line" aria-label="The ADF execution loop">
            <span>change init</span>
            <i aria-hidden="true">→</i>
            <span>next action</span>
            <i aria-hidden="true">→</i>
            <span>submit result</span>
            <i aria-hidden="true">↺</i>
            <strong>ready to merge</strong>
          </div>
        </div>
      </section>

      <section className="guarantees-section section-shell" id="guarantees">
        <div className="section-heading split-heading">
          <div>
            <p className="eyebrow muted">Why you can trust it</p>
            <h2>The process is enforced, not prompted.</h2>
          </div>
          <p>
            ADF checks structure, references, state, digests, and coverage. It
            is explicit about what still needs human judgment.
          </p>
        </div>

        <div className="guarantee-grid">
          <article>
            <span className="guarantee-index">A</span>
            <h3>Computed order</h3>
            <p>
              The next action comes from repository state. An agent cannot skip
              a step by forgetting it.
            </p>
          </article>
          <article>
            <span className="guarantee-index">B</span>
            <h3>Checked authority</h3>
            <p>
              Agent reasoning counts as evidence, never as permission to invent
              a product rule.
            </p>
          </article>
          <article>
            <span className="guarantee-index">C</span>
            <h3>Parsed source</h3>
            <p>
              Detectors read actual code across sixteen languages instead of
              trusting a description of it.
            </p>
          </article>
          <article>
            <span className="guarantee-index">D</span>
            <h3>Stale by design</h3>
            <p>
              Change a contract, its authority, or the code behind it, and the
              dependent work must be checked again.
            </p>
          </article>
        </div>

        <div className="boundary-callout">
          <div className="boundary-mark" aria-hidden="true">
            ≠
          </div>
          <div>
            <small>The honest boundary</small>
            <h3>ADF verifies process—not meaning.</h3>
          </div>
          <p>
            A wrong contract can still be accepted. People remain responsible
            for deciding what the system should do.
          </p>
        </div>
      </section>

      <section className="start-section" id="start">
        <div className="section-shell start-grid">
          <div>
            <p className="eyebrow">Get started</p>
            <h2>Bring durable decisions into your repository.</h2>
            <p className="start-lede">
              Build the current version, initialize ADF, and let the framework
              guide the first change one action at a time.
            </p>
            <a className="button button-light" href={githubUrl}>
              Read the full guide <Arrow />
            </a>
          </div>

          <div className="install-card">
            <div className="install-tabs">
              <span className="active-tab">Build from source</span>
              <span>Rust 1.89+</span>
            </div>
            <pre>
              <code>
                <span className="code-comment"># Build ADF</span>{"\n"}
                <span className="code-prompt">$</span> git clone {githubUrl}
                {"\n"}
                <span className="code-prompt">$</span> cd
                agentic-development-framework{"\n"}
                <span className="code-prompt">$</span> cargo build --release
                {"\n\n"}
                <span className="code-comment"># Initialize a repository</span>
                {"\n"}
                <span className="code-prompt">$</span> adf project init
                --project /path/to/project
              </code>
            </pre>
            <div className="install-footer">
              <span>Nothing existing is overwritten.</span>
              <span aria-hidden="true">✓</span>
            </div>
          </div>
        </div>
      </section>

      <footer className="site-footer">
        <div className="footer-brand">
          <span className="brand-mark" aria-hidden="true">
            A
          </span>
          <div>
            <strong>Agentic Development Framework</strong>
            <p>Keep decisions human. Make evidence durable.</p>
          </div>
        </div>
        <div className="footer-links">
          <a href={githubUrl}>GitHub <Arrow /></a>
          <a href={`${githubUrl}/blob/main/README.md`}>Documentation <Arrow /></a>
          <a href={`${githubUrl}/blob/main/SECURITY.md`}>Security <Arrow /></a>
        </div>
      </footer>
    </main>
  );
}
