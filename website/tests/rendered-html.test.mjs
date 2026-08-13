import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

async function render() {
  const workerUrl = new URL("../dist/server/index.js", import.meta.url);
  workerUrl.searchParams.set("test", `${process.pid}-${Date.now()}`);
  const { default: worker } = await import(workerUrl.href);

  return worker.fetch(
    new Request("http://localhost/", {
      headers: { accept: "text/html" },
    }),
    {
      ASSETS: {
        fetch: async () => new Response("Not found", { status: 404 }),
      },
    },
    {
      waitUntil() {},
      passThroughOnException() {},
    },
  );
}

test("renders the ADF landing page", async () => {
  const response = await render();
  assert.equal(response.status, 200);
  assert.match(response.headers.get("content-type") ?? "", /^text\/html\b/i);

  const html = await response.text();
  assert.match(html, /<title>Agentic Development Framework/);
  assert.match(html, /Agents write code\./);
  assert.match(html, /One change moves through three minds\./);
  assert.match(html, /Production customer data never leaves approved systems\./);
  assert.match(html, /Retrying a payment capture never charges the customer twice\./);
  assert.match(html, /Every completed export records who exported what and when\./);
  assert.match(html, /Nothing existing is overwritten\./);
  assert.doesNotMatch(html, /codex-preview|Your site is taking shape/);
});

test("includes accessible navigation and responsive safeguards", async () => {
  const [page, css, layout] = await Promise.all([
    readFile(new URL("../app/page.tsx", import.meta.url), "utf8"),
    readFile(new URL("../app/globals.css", import.meta.url), "utf8"),
    readFile(new URL("../app/layout.tsx", import.meta.url), "utf8"),
  ]);

  assert.match(page, /aria-label="Main navigation"/);
  assert.match(page, /id="idea"/);
  assert.match(page, /id="process"/);
  assert.match(page, /id="start"/);
  assert.match(css, /@media \(max-width: 700px\)/);
  assert.match(css, /overflow-wrap: anywhere/);
  assert.match(css, /prefers-reduced-motion: reduce/);
  assert.match(layout, /themeColor: "#f4f2ec"/);
});
