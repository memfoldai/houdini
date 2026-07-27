import { test } from "node:test";
import assert from "node:assert/strict";
import { collect } from "../lib/parse.mjs";
import { weekWindow, filterToWeek, compute } from "../lib/metrics.mjs";
import { assignSuperlatives } from "../lib/superlatives.mjs";
import { buildCards } from "../lib/cards.mjs";
import { renderHtml } from "../lib/render.mjs";

function cell(o) {
  return { kind: "analytics_cell", person: "p", device: "d", device_name: "d", day: "2026-07-21", hour: 12, tool: "openclaw", tool_name: "Alma", provider: "openclaw", surface: "cli", model: null, intent: "write_code", shape: "doing", domain: "software_engineering", depth: 1, delegation: "none", delegate_tool: "none", turns: 1, sessions: 1, chars: 100, ...o };
}
function span(o) {
  return { kind: "session_span", person: "p", device: "d", device_name: "d", day: "2026-07-21", tool: "openclaw", tool_name: "Alma", sessions: 1, total_minutes: 10, longest_minutes: 10, ...o };
}

test("week window: a Monday reports the week that just ended (Mon..Sun)", () => {
  const w = weekWindow(new Date("2026-07-27T09:00:00Z"));
  assert.equal(w.start, "2026-07-20");
  assert.equal(w.end, "2026-07-26");
});

test("week window: an explicit date resolves to its containing Mon..Sun", () => {
  const w = weekWindow(new Date("2026-08-01T00:00:00Z"), "2026-07-23");
  assert.equal(w.start, "2026-07-20");
  assert.equal(w.end, "2026-07-26");
});

test("filter keeps only rows inside the window", () => {
  const rows = [cell({ day: "2026-07-19" }), cell({ day: "2026-07-20" }), cell({ day: "2026-07-27" })];
  const w = weekWindow(new Date("2026-07-27T00:00:00Z"));
  assert.equal(filterToWeek(rows, w).length, 1);
});

test("collect is last-wins per dimension key (idempotent re-upload)", () => {
  const { cells } = collect([cell({ turns: 1 }), cell({ turns: 9 })]);
  assert.equal(cells.length, 1);
  assert.equal(cells[0].turns, 9);
});

test("engaged minutes and top person come from spans", () => {
  const { cells, spans } = collect([
    span({ person: "ana", total_minutes: 120, longest_minutes: 60 }),
    span({ person: "ben", total_minutes: 30 }),
    cell({ person: "ana" }),
  ]);
  const m = compute(cells, spans);
  assert.equal(m.totalHours, 3);
  assert.equal(m.topPerson.person, "ana");
  assert.equal(m.top5[0].person, "ana");
});

test("both Alma awards select the right winner", () => {
  const rows = [
    cell({ person: "cc", tool: "openclaw", delegate_tool: "claude_code", shape: "doing", turns: 20 }),
    cell({ person: "cc", tool: "openclaw", delegate_tool: "claude_code", shape: "doing", turns: 5, hour: 13 }),
    cell({ person: "rs", tool: "openclaw", delegate_tool: "none", shape: "asking", intent: "multi_source_synthesis", turns: 30 }),
  ];
  const { cells, spans } = collect(rows);
  const m = compute(cells, spans);
  assert.equal(m.almaClaudeCode[0].person, "cc");
  assert.equal(m.almaClaudeCode[0].value, 25);
  assert.equal(m.almaResearch[0].person, "rs");
  assert.equal(m.almaResearch[0].value, 30);
});

test("superlatives: everyone is named, badges unique while the roster fits the axes", () => {
  const people = ["a", "b", "c", "d"].map((p, i) => ({
    person: p, minutes: 10, longest: i * 40, turns: 20, askingTurns: i === 0 ? 20 : 0,
    doingTurns: i === 1 ? 20 : 0, delegateTurns: i === 2 ? 15 : 0, lateTurns: i === 3 ? 18 : 0,
    earlyTurns: 0, chars: 100 * (i + 1), tools: new Set(["openclaw", i === 0 ? "codex" : "openclaw"]),
    almaClaudeCode: 0, almaResearch: 0,
  }));
  const badges = assignSuperlatives(people);
  assert.equal(badges.length, 4);
  assert.equal(new Set(badges.map((b) => b.person)).size, 4);
  assert.equal(new Set(badges.map((b) => b.badge)).size, 4, "distinct badges for a small roster");
});

test("cards: full arc includes both trophies and a summary", () => {
  const { cells, spans } = collect([span({ person: "p", total_minutes: 60 }), cell({ person: "p", tool: "openclaw", delegate_tool: "claude_code" })]);
  const m = compute(cells, spans);
  const cards = buildCards(m, { team: "T", weekLabel: "wk" });
  const kinds = cards.map((c) => c.kind);
  assert.equal(kinds[0], "title");
  assert.equal(kinds.at(-1), "summary");
  assert.equal(cards.filter((c) => c.kind === "trophy").length, 2);
});

test("cards: an empty week degrades to a friendly non-crashing reel", () => {
  const m = compute([], []);
  const cards = buildCards(m, { team: "T", weekLabel: "wk" });
  assert.ok(cards.length >= 2);
  assert.equal(cards[0].kind, "title");
});

test("render: one self-contained document, escaped, one section per card", () => {
  const { cells, spans } = collect([span({ person: "p", total_minutes: 60 }), cell({ person: "<script>", tool: "openclaw" })]);
  const m = compute(cells, spans);
  const cards = buildCards(m, { team: "T & Co", weekLabel: "wk" });
  const html = renderHtml(cards, { team: "T & Co", weekLabel: "wk" });
  assert.ok(html.startsWith("<!doctype html>"));
  assert.ok(html.includes("requestAnimationFrame"));
  assert.ok(!html.includes("<script>p"), "no unescaped injection from data");
  const sections = html.match(/class="card /g) || [];
  assert.equal(sections.length, cards.length);
});
