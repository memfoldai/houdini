import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import vm from "node:vm";
import { test } from "node:test";
import assert from "node:assert";

const CODE = readFileSync(fileURLToPath(new URL("../capture_actions.js", import.meta.url)), "utf8");

// Load capture_actions.js into a sandbox that mocks the page, capturing the
// click handler it registers and the messages it posts.
function load(hostname) {
  const posted = [];
  let handler = null;
  const sandbox = {
    location: { hostname },
    document: {
      addEventListener: (type, fn) => {
        if (type === "click") handler = fn;
      },
    },
    chrome: { runtime: { sendMessage: (m) => posted.push(m) } },
    Date,
    Math,
    console,
    globalThis: {},
  };
  sandbox.globalThis = sandbox;
  vm.createContext(sandbox);
  vm.runInContext(CODE, sandbox);
  const clickLabel = (label) =>
    handler &&
    handler({ target: { getAttribute: (k) => (k === "aria-label" ? label : null), parentElement: null } });
  return { posted, clickLabel, active: () => handler !== null, matchVerb: sandbox.__bbMatchVerb };
}

test("classifies workspace controls into normalized verbs", () => {
  const h = load("mail.google.com");
  // Gmail
  assert.equal(h.matchVerb("Send ‪(Ctrl-Enter)‬"), "send");
  assert.equal(h.matchVerb("Send & Archive"), "send", "more specific wins");
  assert.equal(h.matchVerb("Reply all"), "reply");
  assert.equal(h.matchVerb("Archive"), "archive");
  assert.equal(h.matchVerb("Report spam"), "spam");
  assert.equal(h.matchVerb("Move to trash"), "delete", "trash beats move");
  assert.equal(h.matchVerb("Remove star"), null);
  // Drive
  assert.equal(h.matchVerb("Share"), "share");
  assert.equal(h.matchVerb("Move to"), "move");
  assert.equal(h.matchVerb("Download"), "download");
  // Non-actions must not be recorded
  assert.equal(h.matchVerb("Some unrelated button"), null);
  assert.equal(h.matchVerb("Search mail"), null);
  assert.equal(h.matchVerb("Settings"), null);
});

test("emits an action payload for a recognized click", () => {
  const h = load("mail.google.com");
  assert.ok(h.active(), "click listener registered on a workspace host");
  h.clickLabel("Send ‪(Ctrl-Enter)‬");
  assert.equal(h.posted.length, 1);
  const [msg] = h.posted;
  assert.equal(msg.actions.length, 1);
  assert.equal(msg.actions[0].app, "mail.google.com");
  assert.equal(msg.actions[0].action, "send");
  assert.equal(msg.actions[0].kind, "mutating");
  assert.equal(msg.actions[0].target, undefined);
  assert.match(msg.actions[0].ext_id, /^mail\.google\.com:/);
});

test("two tabs do not mint the same id on a same-millisecond first click", () => {
  // Freeze time so both "tabs" see the identical Date.now() and counter=1.
  const realNow = Date.now;
  Date.now = () => 1_700_000_000_000;
  try {
    const a = load("mail.google.com");
    const b = load("mail.google.com");
    a.clickLabel("Send");
    b.clickLabel("Send");
    const idA = a.posted[0].actions[0].ext_id;
    const idB = b.posted[0].actions[0].ext_id;
    assert.notEqual(idA, idB, "per-tab nonce keeps the ids distinct");
  } finally {
    Date.now = realNow;
  }
});

test("emits read-only kind for non-mutating workspace controls", () => {
  const h = load("drive.google.com");
  h.clickLabel("Download");
  assert.equal(h.posted.length, 1);
  assert.equal(h.posted[0].actions[0].action, "download");
  assert.equal(h.posted[0].actions[0].kind, "read_only");
});

test("ignores clicks on unrecognized controls", () => {
  const h = load("drive.google.com");
  h.clickLabel("Sort direction");
  assert.equal(h.posted.length, 0);
});

test("does nothing on a non-workspace host", () => {
  const h = load("example.com");
  assert.equal(h.active(), false, "no listener on unmatched host");
});
