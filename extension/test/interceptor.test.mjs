// Validates the real interceptor.js by running it in a stubbed page: a fetch to
// a conversation endpoint should yield a captured {user prompt (from request),
// assistant reply (from the rendered DOM)} with the conversation id from the URL.
// No browser needed; exercises the shipped code path.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import vm from "node:vm";
import { TextEncoder } from "node:util";
import assert from "node:assert";

function makeResponse() {
  // A body reader that immediately ends (we only drain it for completion).
  const body = { getReader: () => ({ read: () => Promise.resolve({ done: true }) }) };
  return { clone: () => ({ body }) };
}

async function run({ hostname, pathname, url, reqBody, assistantText }) {
  const posted = [];
  const assistantEls = assistantText ? [{ innerText: assistantText }] : [];
  const sandbox = {
    location: { hostname, origin: `https://${hostname}`, pathname },
    document: {
      querySelectorAll: (sel) =>
        String(sel).includes("assistant") || String(sel).includes("claude") ? assistantEls : [],
    },
    TextEncoder,
    JSON,
    Date,
    Math,
    URL,
    console,
    setTimeout,
  };
  sandbox.window = {
    fetch: () => Promise.resolve(makeResponse()),
    postMessage: (m) => posted.push(m),
    addEventListener: () => {},
  };
  vm.createContext(sandbox);
  const code = readFileSync(fileURLToPath(new URL("../interceptor.js", import.meta.url)), "utf8");
  vm.runInContext(code, sandbox);
  await sandbox.window.fetch(url, { method: "POST", body: reqBody });
  await new Promise((r) => setTimeout(r, 2500)); // > the interceptor's poll-until-stable (~1.5s)
  return posted;
}

const chatgpt = await run({
  hostname: "chatgpt.com",
  pathname: "/c/conv-abc",
  url: "https://chatgpt.com/backend-api/conversation",
  reqBody: JSON.stringify({
    action: "next",
    messages: [{ author: { role: "user" }, content: { content_type: "text", parts: ["What is soil?"] } }],
  }),
  assistantText: "Soil is the top layer.",
});
assert.equal(chatgpt.length, 1, "chatgpt: one message posted");
const c = chatgpt[0].payload;
assert.equal(c.tool, "chatgpt-web");
assert.equal(c.external_id, "conv-abc", "conversation id from /c/<id> URL");
assert.deepEqual(c.turns.map((t) => [t.role, t.text]),
  [["user", "What is soil?"], ["assistant", "Soil is the top layer."]]);
console.log("PASS chatgpt-web: prompt from request, reply from DOM, id from URL");

const claude = await run({
  hostname: "claude.ai",
  pathname: "/chat/conv-xyz",
  url: "https://claude.ai/api/organizations/org1/chat_conversations/conv-xyz/completion",
  reqBody: JSON.stringify({ prompt: "Explain photosynthesis" }),
  assistantText: "Plants use light.",
});
assert.equal(claude.length, 1, "claude: one message posted");
const k = claude[0].payload;
assert.equal(k.tool, "claude-web");
assert.equal(k.external_id, "conv-xyz");
assert.deepEqual(k.turns.map((t) => [t.role, t.text]),
  [["user", "Explain photosynthesis"], ["assistant", "Plants use light."]]);
console.log("PASS claude-web: prompt from request, reply from DOM, id from URL");

const noise = await run({
  hostname: "chatgpt.com",
  pathname: "/c/x",
  url: "https://chatgpt.com/backend-api/me",
  reqBody: "{}",
  assistantText: "",
});
assert.equal(noise.length, 0, "non-conversation request captured nothing");
console.log("PASS ignores non-conversation requests");
console.log("\nAll interceptor checks passed.");
