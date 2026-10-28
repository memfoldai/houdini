import { test } from "node:test";
import assert from "node:assert/strict";
import { collect } from "../lib/parse.mjs";
import { weekWindow, filterToWeek, compute, almaHistory } from "../lib/metrics.mjs";
import { assignPersonas } from "../lib/personas.mjs";
import { buildCards } from "../lib/cards.mjs";
import { renderHtml } from "../lib/render.mjs";

function cell(o) {
  return { kind: "analytics_cell", person: "p", device: "d", device_name: "d", day: "2026-07-21", hour: 12, tool: "openclaw", tool_name: "Alma", provider: "openclaw", surface: "cli", model: null, intent: "write_code", shape: "doing", domain: "software_engineering", depth: 1, delegation: "none", delegate_tool: "none", turns: 1, sessions: 1, chars: 100, ...o };
}
function span(o) {
  return { kind: "session_span", person: "p", device: "d", device_name: "d", day: "2026-07-21", tool: "openclaw", tool_name: "Alma", sessions: 1, total_minutes: 10, longest_minutes: 10, ...o };
}
function shortcut(o) {
  return { kind: "shortcut", device: "d", person: "p", day: "2026-07-21", name: "stalk", detail: "voice", runs: 1, ...o };
}
function connector(o) {
  return { kind: "connector", device: "d", person: "p", day: "2026-07-21", name: "almanac-connect", detail: "people.resolve", runs: 1, ...o };
}

const WEEK = weekWindow(new Date("2026-07-27T00:00:00Z"));

function build(rows, window = WEEK) {
  const { cells, spans, usage } = collect(rows);
  return compute(filterToWeek(cells, window), filterToWeek(spans, window), filterToWeek(usage, window), window);
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
  assert.equal(filterToWeek(rows, WEEK).length, 1);
});

test("collect is last-wins per dimension key (idempotent re-upload)", () => {
  const { cells } = collect([cell({ turns: 1 }), cell({ turns: 9 })]);
  assert.equal(cells.length, 1);
  assert.equal(cells[0].turns, 9);
});

test("connector and shortcut rows are collected and idempotent", () => {
  const { usage } = collect([connector({ runs: 3 }), connector({ runs: 8 }), shortcut({ name: "annotate", runs: 2 })]);
  assert.equal(usage.length, 2, "re-upload overwrites, never duplicates");
  assert.equal(usage.find((u) => u.kind === "connector").runs, 8);
  assert.equal(usage.find((u) => u.kind === "shortcut").name, "annotate");
});

test("alma: actions count shortcut runs and connector calls, never prompts", () => {
  const m = build([
    shortcut({ person: "ana", name: "stalk", runs: 5 }),
    shortcut({ person: "ben", name: "stalk", runs: 2 }),
    shortcut({ person: "ana", name: "visualise", runs: 3, detail: "app-runtime" }),
    connector({ person: "ana", runs: 4 }),
    cell({ person: "ana", tool: "claude-code", tool_name: "Claude Code", turns: 100 }),
  ]);
  assert.equal(m.alma.shortcutRuns, 10);
  assert.equal(m.alma.connectorRuns, 4);
  assert.equal(m.alma.actions, 14);
  assert.equal(m.alma.distinctShortcuts, 2);
  assert.equal(m.alma.peopleUsing, 2);
  assert.equal(m.alma.voicePct, 70, "3 of 10 runs were not voice-triggered");
  assert.equal(m.alma.byName[0].key, "stalk");
  assert.equal(m.alma.byName[0].runs, 7);
  assert.equal(m.alma.byName[0].people, 2);
});

test("alma: reach ranks by distinct people, which is a different order to volume", () => {
  const m = build([
    shortcut({ person: "ana", name: "trip", runs: 20 }),
    shortcut({ person: "ana", name: "visualise", runs: 1 }),
    shortcut({ person: "ben", name: "visualise", runs: 1 }),
    shortcut({ person: "cat", name: "visualise", runs: 1 }),
  ]);
  assert.equal(m.alma.byName[0].key, "trip", "volume leader");
  assert.equal(m.alma.byReach[0].key, "visualise", "reach leader");
  assert.equal(m.alma.byReach[0].people, 3);
});

test("alma: every day of the window appears, including the silent ones", () => {
  const m = build([shortcut({ day: "2026-07-21", runs: 4 })]);
  assert.equal(m.alma.byDay.length, 7);
  assert.deepEqual(m.alma.byDay.map((d) => d.label), ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]);
  assert.equal(m.alma.byDay.find((d) => d.day === "2026-07-21").shortcuts, 4);
  assert.equal(m.alma.byDay.find((d) => d.day === "2026-07-20").shortcuts, 0);
});

test("alma: capability buckets fold the mcp prefix and underscore spellings together", () => {
  const m = build([
    connector({ detail: "mcp.file-ops.annotate", runs: 2 }),
    connector({ detail: "file-ops.annotate", runs: 3, person: "ben" }),
    connector({ detail: "recap.day", runs: 1 }),
    connector({ detail: "recap_day", runs: 1, person: "ben" }),
  ]);
  const annotate = m.alma.capabilities.find((c) => c.key === "annotate files");
  assert.equal(annotate.runs, 5, "one capability, two spellings");
  assert.equal(annotate.people, 2);
  assert.equal(m.alma.capabilities.find((c) => c.key === "day recap").runs, 2);
});

test("alma: session shape is reported from spans, one-shot people counted", () => {
  const m = build([
    span({ person: "ana", tool: "openclaw", sessions: 30, total_minutes: 2, longest_minutes: 1 }),
    span({ person: "ben", tool: "openclaw", sessions: 4, total_minutes: 40, longest_minutes: 24 }),
    span({ person: "ana", tool: "claude-code", tool_name: "Claude Code", sessions: 5, total_minutes: 600, longest_minutes: 300 }),
    shortcut({ person: "ana" }),
  ]);
  assert.equal(m.alma.sessions, 34, "alma spans only");
  assert.equal(m.alma.minutes, 42);
  assert.equal(m.alma.peopleWithSessions, 2);
  assert.equal(m.alma.oneShotPeople, 1);
  const ana = m.people.find((p) => p.person === "ana");
  assert.equal(ana.agentMinutes, 600, "agent time is per person and excludes Alma");
  assert.equal(ana.agentSessions, 5);
});

test("history: a debut is first-ever, judged against all of history not the window", () => {
  const usage = [
    shortcut({ day: "2026-06-02", name: "stalk", runs: 5 }),
    shortcut({ day: "2026-07-22", name: "stalk", runs: 9 }),
    shortcut({ day: "2026-07-22", name: "trip", runs: 4, person: "ben" }),
  ];
  const h = almaHistory(collect(usage).usage, WEEK);
  assert.deepEqual(h.debuts.map((d) => d.key), ["trip"], "stalk ran weeks earlier, so it is not new");
  assert.equal(h.debuts[0].runs, 4);
});

test("cards: the arc runs team → Alma → a card per person → summary", () => {
  const m = build([
    shortcut({ person: "ana", name: "stalk", runs: 9 }),
    shortcut({ person: "ben", name: "visualise", runs: 4 }),
    connector({ person: "ana", runs: 3 }),
    span({ person: "ana", tool: "openclaw", sessions: 12, total_minutes: 3, longest_minutes: 1 }),
    cell({ person: "ana", tool: "claude-code", tool_name: "Claude Code", turns: 50 }),
    cell({ person: "ben", tool: "codex", tool_name: "Codex", turns: 20, intent: "review_or_critique" }),
  ]);
  const cards = buildCards(m, { team: "T", weekLabel: "wk", history: almaHistory([], WEEK) });
  const kinds = cards.map((c) => c.kind);
  assert.equal(kinds[0], "title");
  assert.equal(kinds.at(-1), "summary");
  assert.ok(kinds.includes("ranked"), "the cross-tool comparison is back");
  assert.ok(kinds.includes("bars"), "the week is charted");
  assert.ok(kinds.includes("counts"), "shortcuts are ranked by count");
  assert.equal(kinds.filter((k) => k === "person").length, 2, "one card per active person");
  assert.equal(cards.find((c) => c.kind === "bars").bars.length, 7);
  // Alma earns its place from shortcut runs, not prompts: ana ran 9 and ben 4,
  // so 13 requests rank it below Codex's 20 prompts and above nothing.
  const tools = cards.find((c) => c.kicker === "which AI got used the most");
  assert.deepEqual(tools.rows.map((r) => r.label), ["Claude Code", "Codex", "Alma"]);
  assert.equal(m.requestShare.find((t) => t.tool === "openclaw").turns, 13);
  assert.equal(m.toolShare.find((t) => t.tool === "openclaw"), undefined, "prompt-only ranking still has no Alma");
  assert.equal(cards.find((c) => c.kind === "podium"), undefined, "the top-three board was removed");
});

test("personas: everyone active gets exactly one card, with a distinct badge", () => {
  const m = build([
    cell({ person: "ana", tool: "claude-code", tool_name: "Claude Code", turns: 300 }),
    cell({ person: "ben", tool: "codex", tool_name: "Codex", turns: 40, intent: "review_or_critique" }),
    cell({ person: "cat", tool: "chatgpt-web", tool_name: "ChatGPT", turns: 25, intent: "configure_or_setup", hour: 9 }),
    shortcut({ person: "cat", name: "stalk", runs: 30 }),
    connector({ person: "ben", runs: 12 }),
    shortcut({ person: "dee", name: "save", runs: 6 }),
  ]);
  const cards = assignPersonas(m.people);
  assert.equal(cards.length, 4, "nobody is left out");
  assert.equal(new Set(cards.map((c) => c.person)).size, 4, "one card each");
  assert.equal(new Set(cards.map((c) => c.badge)).size, 4, "badges are distinct");
  for (const c of cards) assert.ok(c.line.length > 0 && !/\d/.test(c.badge), "cards read as prose, not metrics");
});

test("personas: a superlative is skipped, never handed down to the runner-up", () => {
  // ana holds both "most prompts" and "most shortcuts". The shortcut title must
  // NOT fall through to ben, whose 5 runs are not the most.
  const m = build([
    cell({ person: "ana", tool: "claude-code", tool_name: "Claude Code", turns: 300 }),
    shortcut({ person: "ana", name: "stalk", runs: 40 }),
    cell({ person: "ben", tool: "claude-code", tool_name: "Claude Code", turns: 20 }),
    shortcut({ person: "ben", name: "save", runs: 5 }),
  ]);
  const cards = assignPersonas(m.people);
  const ben = cards.find((c) => c.person === "ben");
  assert.ok(!/more shortcuts than anyone|the shortcut habit/i.test(ben.badge + ben.line), "runner-up never inherits a 'most' claim");
  assert.match(cards.find((c) => c.person === "ana").badge, /out-prompted everyone/);
});

test("personas: 'the only one on X' is withheld when two people share X", () => {
  const m = build([
    cell({ person: "ana", tool: "claude-code", tool_name: "Claude Code", turns: 200 }),
    cell({ person: "ben", tool: "codex", tool_name: "Codex", turns: 30, intent: "review_or_critique" }),
    cell({ person: "cat", tool: "codex", tool_name: "Codex", turns: 20, intent: "plan_or_organize" }),
  ]);
  const cards = assignPersonas(m.people);
  for (const c of cards) assert.ok(!/only one whose main AI was Codex/.test(c.line), "two people lead with Codex");
});

test("personas: a tie produces no superlative at all", () => {
  const m = build([
    cell({ person: "ana", tool: "claude-code", tool_name: "Claude Code", turns: 50 }),
    cell({ person: "ben", tool: "claude-code", tool_name: "Claude Code", turns: 50 }),
  ]);
  const cards = assignPersonas(m.people);
  for (const c of cards) assert.ok(!/out-prompted everyone/i.test(c.badge), "tied counts cannot claim 'the most'");
});

test("cards: a dictation sidecar adds spoken-word slides, and is skipped when absent", () => {
  const m = build([cell({ person: "ana", tool: "claude-code", tool_name: "Claude Code", turns: 40 }), shortcut({ person: "ben", runs: 3 })]);
  const base = { team: "T", weekLabel: "wk", history: almaHistory([], WEEK) };
  const without = buildCards(m, base);
  assert.equal(without.some((c) => c.kicker === "words spoken to the machine"), false, "no sidecar, no slide");
  const dictation = { words: 1000, sessions: 50, speechMinutes: 12, apps: [{ label: "Notes", value: 600 }, { label: "Claude", value: 400 }] };
  const withD = buildCards(m, { ...base, dictation });
  const stat = withD.find((c) => c.kicker === "words spoken to the machine");
  assert.equal(stat.heroNumber, 1000);
  const apps = withD.find((c) => c.kicker === "where the team dictates");
  assert.deepEqual(apps.rows.map((r) => r.label), ["Notes", "Claude"]);
});

test("coverage: only real push times can call a device quiet — rows alone never do", () => {
  const rows = [shortcut({ person: "ana", day: "2026-07-20", runs: 5 }), shortcut({ person: "ben", day: "2026-07-26", runs: 5 })];
  const { cells, spans, usage } = collect(rows);
  const noPushes = compute(cells, spans, usage, WEEK);
  assert.equal(noPushes.coverage.quiet.length, 0, "without push times, no claim");

  // ana's laptop uploaded on Sunday night: nothing since Monday was a quiet
  // week, not a missing one. ben's last upload was Wednesday: Thu–Sun unknown.
  const pushes = new Map([["ana", "2026-07-26"], ["ben", "2026-07-22"]]);
  const m = compute(cells, spans, usage, WEEK, pushes);
  assert.deepEqual(m.coverage.quiet.map((q) => q.person), ["ben"]);
  const cards = buildCards(m, { team: "T", weekLabel: "wk", history: almaHistory([], WEEK) });
  assert.match(cards.find((c) => c.kind === "bars").sub, /1 of 2 devices last checked in before Sunday/);
});

test("cards: hand-written personas replace the generated set for the week", () => {
  const m = build([cell({ person: "ana", tool: "claude-code", tool_name: "Claude Code", turns: 40 }), shortcut({ person: "ben", runs: 3 })]);
  const personas = [{ person: "ben", badge: "the stalker", line: "Ran stalk and nothing else." }];
  const cards = buildCards(m, { team: "T", weekLabel: "wk", history: almaHistory([], WEEK), personas });
  const people = cards.filter((c) => c.kind === "person");
  assert.equal(people.length, 1, "curated set is used wholesale, not merged");
  assert.equal(people[0].badge, "the stalker");
});

test("cards: an empty week degrades to a friendly non-crashing reel", () => {
  const m = build([]);
  const cards = buildCards(m, { team: "T", weekLabel: "wk" });
  assert.ok(cards.length >= 2);
  assert.equal(cards[0].kind, "title");
  assert.equal(cards.at(-1).kind, "summary");
});

test("render: one self-contained document, escaped, one section per card", () => {
  const m = build([shortcut({ person: "<script>", runs: 2 }), connector({ person: "<script>" })]);
  const cards = buildCards(m, { team: "T & Co", weekLabel: "wk", history: almaHistory([], WEEK) });
  const html = renderHtml(cards, { team: "T & Co", weekLabel: "wk" });
  assert.ok(html.startsWith("<!doctype html>"));
  assert.ok(html.includes("requestAnimationFrame"));
  assert.ok(!html.includes("<script>p"), "no unescaped injection from data");
  assert.ok(html.includes('class="bars"') || html.includes('class="counts"'), "chart markup ships");
  const sections = html.match(/class="card /g) || [];
  assert.equal(sections.length, cards.length);
});
