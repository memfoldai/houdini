import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import vm from "node:vm";
import { test } from "node:test";
import assert from "node:assert";

const CODE = readFileSync(fileURLToPath(new URL("../capture.js", import.meta.url)), "utf8");

function load({ hostname, pathname }) {
  const posted = [];
  const state = { user: "", assistant: "" };
  let tick = null;
  const sandbox = {
    location: { hostname, pathname, origin: `https://${hostname}` },
    document: {
      querySelectorAll: (selector) => {
        const s = String(selector);
        const isUser = s.includes('"user"') || s.includes("user-query") || s.includes("user-message");
        const text = isUser ? state.user : state.assistant;
        return text ? [{ innerText: text }] : [];
      },
    },
    chrome: { runtime: { sendMessage: (m) => posted.push(m) } },
    setInterval: (fn) => {
      tick = fn;
      return 1;
    },
    Date,
    Math,
    console,
  };
  vm.createContext(sandbox);
  vm.runInContext(CODE, sandbox);
  return { posted, state, tick: () => (tick ? tick() : null), started: () => tick !== null };
}

function stabilize(h) {
  // one changing poll, then STABLE_POLLS(2) unchanged polls → emit
  h.tick();
  h.tick();
  h.tick();
}

test("chatgpt: emits the exchange once the reply stops changing, and only once", () => {
  const h = load({ hostname: "chatgpt.com", pathname: "/c/abc123" });
  h.state.user = "what is a monad";
  h.state.assistant = "A mon";
  h.tick();
  h.state.assistant = "A monad is a design pattern";
  stabilize(h);

  assert.equal(h.posted.length, 1);
  assert.equal(h.posted[0].tool, "chatgpt-web");
  assert.equal(h.posted[0].external_id, "abc123");
  assert.deepEqual(h.posted[0].turns.map((t) => t.role), ["user", "assistant"]);
  assert.equal(h.posted[0].turns[1].text, "A monad is a design pattern");

  h.tick();
  h.tick();
  assert.equal(h.posted.length, 1, "a stable reply is not re-emitted");
});

test("claude and gemini adapters resolve by host and id", () => {
  const c = load({ hostname: "claude.ai", pathname: "/chat/conv-9" });
  c.state.user = "hi";
  c.state.assistant = "hello there";
  stabilize(c);
  assert.equal(c.posted[0].tool, "claude-web");
  assert.equal(c.posted[0].external_id, "conv-9");

  const g = load({ hostname: "gemini.google.com", pathname: "/app/xyz-1" });
  g.state.user = "explain CAP";
  g.state.assistant = "Consistency, Availability, Partition tolerance";
  stabilize(g);
  assert.equal(g.posted[0].tool, "gemini-web");
  assert.equal(g.posted[0].external_id, "xyz-1");
  assert.equal(g.posted[0].turns[1].text, "Consistency, Availability, Partition tolerance");
});

test("a new reply in the same conversation emits a second exchange", () => {
  const h = load({ hostname: "chatgpt.com", pathname: "/c/abc123" });
  h.state.user = "q1";
  h.state.assistant = "a1";
  stabilize(h);
  h.state.user = "q2";
  h.state.assistant = "a2";
  stabilize(h);
  assert.equal(h.posted.length, 2);
  assert.equal(h.posted[1].turns[1].text, "a2");
});

test("unsupported host does nothing", () => {
  const h = load({ hostname: "example.com", pathname: "/" });
  assert.equal(h.started(), false, "no polling starts on an unmatched site");
});
