import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Agentic Development Framework — Keep decisions human",
  description:
    "A control plane for repositories worked on by AI agents. Specifications, decisions, implementation, and evidence stay connected.",
};

const githubUrl = "https://github.com/piaro/agentic-development-framework";

export default function Home() {
  return (
    <main id="top">
      <header className="masthead">
        <a className="wordmark" href="#top" aria-label="Agentic Development Framework home">
          ADF
        </a>
        <nav aria-label="Main navigation">
          <a href="#idea">The idea</a>
          <a href="#process">The process</a>
          <a href="#start">Start</a>
        </nav>
        <a className="github-link" href={githubUrl}>
          GitHub <span aria-hidden="true">↗</span>
        </a>
      </header>

      <section className="hero page-width">
        <p className="kicker">A control plane for agentic development</p>
        <h1>
          Agents write code.
          <br />
          <span>People decide what it means.</span>
        </h1>
        <div className="hero-bottom">
          <p>
            ADF keeps specifications, human decisions, implementation, and
            evidence connected inside the repository—so an agent cannot quietly
            turn a missing rule into code.
          </p>
          <a className="text-link" href="#idea">
            See the problem <span aria-hidden="true">↓</span>
          </a>
        </div>
      </section>

      <section className="decision-scene page-width" id="idea">
        <div className="scene-question">
          <p className="scene-label">A seemingly simple request</p>
          <blockquote>“Delete the task and its attachments.”</blockquote>
        </div>

        <div className="fork" role="img" aria-label="A request reaches a missing product decision, which must be answered by a person before implementation continues">
          <div className="fork-line" aria-hidden="true" />
          <div className="fork-node request-node">
            <small>Request</small>
            <strong>Delete task</strong>
          </div>
          <div className="fork-node gap-node">
            <small>Missing rule</small>
            <strong>What happens to attachments?</strong>
          </div>
          <div className="fork-node answer-node">
            <small>Human decision</small>
            <strong>Delete them with the task</strong>
          </div>
          <div className="fork-node code-node">
            <small>Implementation</small>
            <strong>Now the agent can build</strong>
          </div>
        </div>

        <div className="scene-conclusion">
          <p>
            Without ADF, the middle step disappears. The agent makes a plausible
            choice, and a product decision hides inside the diff.
          </p>
          <p>
            With ADF, the gap stops the work. A person decides. The answer becomes
            a durable contract with evidence behind it.
          </p>
        </div>
      </section>

      <section className="manifesto page-width">
        <p className="section-number">01</p>
        <div>
          <h2>The contract is the product.</h2>
          <p className="large-copy">
            Not another prompt. Not a longer context file. A contract says what
            the repository currently holds true, who had the authority to decide
            it, and what evidence proves it.
          </p>
        </div>
      </section>

      <section className="contract-examples page-width" aria-labelledby="contract-examples-title">
        <div className="examples-intro">
          <p id="contract-examples-title">Contracts at different scopes</p>
          <p>
            A contract is not limited to one feature. The same shape can hold a
            system-wide boundary, a data invariant, or the exact behavior of one
            operation.
          </p>
        </div>

        <article className="contract-example featured-contract">
          <div className="contract-header">
            <span>Data · contract.task-lifecycle</span>
            <span className="accepted">accepted</span>
          </div>
          <p className="clause">No attachment outlives its parent task.</p>
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
              <dt>Governs</dt>
              <dd>Every task deletion</dd>
            </div>
          </dl>
        </article>

        <div className="contract-list">
          <article className="compact-contract">
            <div className="contract-kind">
              <span>Project</span>
              <span>01</span>
            </div>
            <div className="compact-content">
              <h3>Production customer data never leaves approved systems.</h3>
              <dl>
                <div><dt>Authority</dt><dd>policy.data-handling</dd></div>
                <div><dt>Evidence</dt><dd>audit_external_boundaries</dd></div>
              </dl>
            </div>
          </article>

          <article className="compact-contract">
            <div className="contract-kind">
              <span>Data</span>
              <span>02</span>
            </div>
            <div className="compact-content">
              <h3>An order total always equals its line items after discounts.</h3>
              <dl>
                <div><dt>Authority</dt><dd>decision.order-total</dd></div>
                <div><dt>Evidence</dt><dd>property_order_total</dd></div>
              </dl>
            </div>
          </article>

          <article className="compact-contract">
            <div className="contract-kind">
              <span>Operation</span>
              <span>03</span>
            </div>
            <div className="compact-content">
              <h3>Retrying a payment capture never charges the customer twice.</h3>
              <dl>
                <div><dt>Authority</dt><dd>decision.payment-retry</dd></div>
                <div><dt>Evidence</dt><dd>test_capture_idempotent</dd></div>
              </dl>
            </div>
          </article>

          <article className="compact-contract">
            <div className="contract-kind">
              <span>Feature</span>
              <span>04</span>
            </div>
            <div className="compact-content">
              <h3>Every completed export records who exported what and when.</h3>
              <dl>
                <div><dt>Authority</dt><dd>request.audited-exports</dd></div>
                <div><dt>Evidence</dt><dd>test_export_audit_event</dd></div>
              </dl>
            </div>
          </article>
        </div>
      </section>

      <section className="process page-width" id="process">
        <div className="process-intro">
          <p className="section-number">02</p>
          <div>
            <h2>One change moves through three minds.</h2>
            <p>
              The order comes from repository state, not from what an agent
              remembers. Each role receives a bounded job.
            </p>
          </div>
        </div>

        <ol className="process-line">
          <li>
            <span className="process-index">1</span>
            <div>
              <h3>Analyst</h3>
              <p>Find affected rules. Expose decisions no one has made yet.</p>
            </div>
          </li>
          <li>
            <span className="process-index">2</span>
            <div>
              <h3>Builder</h3>
              <p>Implement only after the rules are settled. Attach proof.</p>
            </div>
          </li>
          <li>
            <span className="process-index">3</span>
            <div>
              <h3>Challenger</h3>
              <p>Work in a separate context. Try to falsify the result.</p>
            </div>
          </li>
        </ol>
      </section>

      <section className="principles">
        <div className="page-width principles-inner">
          <p className="section-number">03</p>
          <div className="principles-heading">
            <h2>The agent cannot talk its way past the process.</h2>
          </div>
          <div className="principle-list">
            <article>
              <span>Computed</span>
              <h3>The next action is derived.</h3>
              <p>An agent cannot skip work by forgetting a step or invent work the state does not call for.</p>
            </article>
            <article>
              <span>Bound</span>
              <h3>Authority is checked.</h3>
              <p>Agent reasoning is useful evidence. It is never authority to settle a product rule.</p>
            </article>
            <article>
              <span>Independent</span>
              <h3>Nothing reviews itself.</h3>
              <p>The post-build challenge runs outside the context that produced the implementation.</p>
            </article>
            <article>
              <span>Honest</span>
              <h3>Meaning remains human.</h3>
              <p>ADF verifies structure, provenance, and coverage. A person still decides whether a rule is right.</p>
            </article>
          </div>
        </div>
      </section>

      <section className="start page-width" id="start">
        <div className="start-copy">
          <p className="section-number">04</p>
          <h2>Let the repository remember.</h2>
          <p>
            ADF is open source, written in Rust, and designed for Codex and
            Claude Code. Build the current version and initialize it in a Git
            repository.
          </p>
          <a className="primary-link" href={githubUrl}>
            Read the guide <span aria-hidden="true">↗</span>
          </a>
        </div>
        <div className="code-block">
          <div className="code-label">Build from source</div>
          <pre><code><span>$</span> git clone {githubUrl}{"\n"}<span>$</span> cd agentic-development-framework{"\n"}<span>$</span> cargo build --release{"\n\n"}<span>$</span> adf project init --project /path/to/project</code></pre>
          <p>Nothing existing is overwritten.</p>
        </div>
      </section>

      <footer className="footer page-width">
        <p>Agentic Development Framework</p>
        <p>Keep decisions human. Make evidence durable.</p>
        <div>
          <a href={`${githubUrl}/blob/main/README.md`}>Documentation</a>
          <a href={`${githubUrl}/blob/main/SECURITY.md`}>Security</a>
          <a href={githubUrl}>GitHub</a>
        </div>
      </footer>
    </main>
  );
}
