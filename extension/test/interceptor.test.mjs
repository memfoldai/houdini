// Validates the real interceptor.js parsers by running it in a stubbed page
// environment and feeding realistic SSE. No browser needed; exercises the
// shipped code (fetch wrap → clone stream read → per-site parse → postMessage).
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import vm from "node:vm";
import { TextDecoder, TextEncoder } from "node:util";
import assert from "node:assert";

function makeResponse(chunks) {
  const enc = new TextEncoder();
  let i = 0;
  const body = { getReader: () => ({ read: () =>
    i < chunks.length ? Promise.resolve({ done: false, value: enc.encode(chunks[i++]) })
                       : Promise.resolve({ done: true }) }) };
  return { clone: () => ({ body }) };
}

async function run({ hostname, url, reqBody, sse }) {
  const posted = [];
  const sandbox = { location: { hostname, origin: `https://${hostname}` },
    TextDecoder, TextEncoder, JSON, Date, Math, URL, console };
  sandbox.window = { fetch: () => Promise.resolve(makeResponse(sse)),
    postMessage: (m) => posted.push(m), addEventListener: () => {} };
  vm.createContext(sandbox);
  const code = readFileSync(fileURLToPath(new URL("../interceptor.js", import.meta.url)), "utf8");
  vm.runInContext(code, sandbox);
  await sandbox.window.fetch(url, { method: "POST", body: reqBody });
  await new Promise((r) => setTimeout(r, 50));
  return posted;
}

const chatgpt = await run({
  hostname: "chatgpt.com",
  url: "https://chatgpt.com/backend-api/conversation",
  reqBody: JSON.stringify({ action: "next",
    messages: [{ author: { role: "user" }, content: { content_type: "text", parts: ["What is soil?"] } }] }),
  sse: [
    'data: {"conversation_id":"conv-abc","message":{"author":{"role":"assistant"},"content":{"content_type":"text","parts":["Soil is"]}}}\n',
    'data: {"conversation_id":"conv-abc","message":{"author":{"role":"assistant"},"content":{"content_type":"text","parts":["Soil is the top layer."]}}}\n',
    "data: [DONE]\n",
  ],
});
assert.equal(chatgpt.length, 1, "chatgpt: one message posted");
const c = chatgpt[0].payload;
assert.equal(c.tool, "chatgpt-web");
assert.equal(c.external_id, "conv-abc");
assert.deepEqual(c.turns.map((t) => [t.role, t.text]),
  [["user", "What is soil?"], ["assistant", "Soil is the top layer."]]);
console.log("PASS chatgpt-web: prompt + cumulative reply + conversation_id");

const claude = await run({
  hostname: "claude.ai",
  url: "https://claude.ai/api/organizations/org1/chat_conversations/conv-xyz/completion",
  reqBody: JSON.stringify({ prompt: "Explain photosynthesis" }),
  sse: [
    'data: {"type":"completion","completion":"Plants "}\n',
    'data: {"type":"completion","completion":"use light."}\n',
  ],
});
assert.equal(claude.length, 1, "claude: one message posted");
const k = claude[0].payload;
assert.equal(k.tool, "claude-web");
assert.equal(k.external_id, "conv-xyz");
assert.deepEqual(k.turns.map((t) => [t.role, t.text]),
  [["user", "Explain photosynthesis"], ["assistant", "Plants use light."]]);
console.log("PASS claude-web: prompt + delta reply + conversation id from URL");

// Negative: a non-conversation request on the site must post nothing.
const noise = await run({ hostname: "chatgpt.com", url: "https://chatgpt.com/backend-api/me",
  reqBody: "{}", sse: ['data: {"x":1}\n'] });
assert.equal(noise.length, 0, "non-conversation request captured nothing");
console.log("PASS ignores non-conversation requests");
console.log("\nAll interceptor parser checks passed.");
