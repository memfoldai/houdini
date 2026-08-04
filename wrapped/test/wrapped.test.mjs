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

test("persona cards carry each person's own receipts, no award cards remain", () => {
  const rows = [
    span({ person: "grinder", tool: "openclaw", sessions: 30, total_minutes: 2000 }),
    cell({ person: "grinder", tool: "openclaw", turns: 20, chars: 200, hour: 12 }),
    span({ person: "casual", tool: "claude-code", tool_name: "Claude Code", sessions: 5, total_minutes: 200 }),
    cell({ person: "casual", tool: "openclaw", shape: "expressing", intent: "casual_conversation", turns: 40, chars: 8000 }),
  ];
  const { cells, spans, delegations } = collect(rows);
  const m = compute(cells, spans, delegations);
  const cards = buildCards(m, { team: "T", weekLabel: "wk" });
  assert.equal(cards.filter((c) => c.kind === "trophy").length, 0, "awards removed");
  const personas = cards.filter((c) => c.kind === "person");
  assert.equal(personas.length, 2, "everyone gets a persona");
  for (const p of personas) assert.ok(Array.isArray(p.chips), "personas carry stat chips");
  const yap = personas.find((p) => p.personName === "casual");
  assert.match(yap.line, /40/, "the joke is their own number");
});

test("connector and shortcut rows are collected and idempotent", () => {
  const row = { kind: "connector", device: "d", person: "p", day: "2026-07-21", name: "almanac-calendar", detail: "calendar.find", runs: 3 };
  const again = { ...row, runs: 8 };
  const sc = { kind: "shortcut", device: "d", person: "p", day: "2026-07-21", name: "annotate", detail: "app-runtime", runs: 2 };
  const { usage } = collect([row, again, sc]);
  assert.equal(usage.length, 2, "re-upload overwrites, never duplicates");
  assert.equal(usage.find((u) => u.kind === "connector").runs, 8);
  assert.equal(usage.find((u) => u.kind === "shortcut").name, "annotate");
});

test("delegation rows drive droveClaudeCode deterministically, ignoring other tools", () => {
  const rows = [
    span({ person: "aa", total_minutes: 30 }),
    span({ person: "bb", total_minutes: 10 }),
    { kind: "delegation", device: "d", person: "aa", day: "2026-07-21", tool: "openclaw", driven_tool: "claude_code", turns: 10 },
    { kind: "delegation", device: "d", person: "bb", day: "2026-07-21", tool: "openclaw", driven_tool: "claude_code", turns: 3 },
    { kind: "delegation", device: "d", person: "aa", day: "2026-07-21", tool: "openclaw", driven_tool: "codex", turns: 99 },
  ];
  const { cells, spans, delegations } = collect(rows);
  const m = compute(cells, spans, delegations);
  assert.equal(m.people.find((p) => p.person === "aa").droveClaudeCode, 10);
  assert.equal(m.people.find((p) => p.person === "bb").droveClaudeCode, 3);
});

test("superlatives: everyone is named, badges unique while the roster fits the axes", () => {
  const mk = (person, over) => ({
    person, minutes: 200, sessions: 10, longest: 30, turns: 20, askingTurns: 0, doingTurns: 0,
    delegateTurns: 0, lateTurns: 0, earlyTurns: 0, chars: 4000, tools: new Set(["openclaw"]),
    almaResearch: 0, almaTurns: 0, almaSessions: 0, almaMinutes: 0, weekendMinutes: 0,
    dayMinutes: { "2026-07-21": 100 }, topDayMinutes: 100, topDayName: "Tuesday",
    casualTurns: 0, troubleshootTurns: 0, draftTurns: 0, configTurns: 0, drives: 0, droveClaudeCode: 0,
    ...over,
  });
  const people = [
    mk("a", { minutes: 3000 }),
    mk("b", { casualTurns: 50 }),
    mk("c", { chars: 50000 }),
    mk("d", { troubleshootTurns: 80 }),
  ];
  const badges = assignSuperlatives(people);
  assert.equal(badges.length, 4);
  assert.equal(new Set(badges.map((b) => b.person)).size, 4);
  assert.equal(new Set(badges.map((b) => b.badge)).size, 4, "distinct badges for a small roster");
  for (const b of badges) assert.ok(b.chips.length >= 1, "every persona carries receipts");
});

test("cards: the arc runs title → stats → personas → summary", () => {
  const { cells, spans } = collect([span({ person: "p", total_minutes: 60 }), cell({ person: "p", tool: "openclaw" })]);
  const m = compute(cells, spans);
  const cards = buildCards(m, { team: "T", weekLabel: "wk" });
  const kinds = cards.map((c) => c.kind);
  assert.equal(kinds[0], "title");
  assert.equal(kinds.at(-1), "summary");
  assert.ok(kinds.includes("person"));
  assert.ok(!kinds.includes("trophy"));
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
