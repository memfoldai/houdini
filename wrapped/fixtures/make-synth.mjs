#!/usr/bin/env node
// Writes a synthetic multi-person week (Mon 2026-07-20 .. Sun 2026-07-26) in the
// exact aum/3 export schema so the generator and collector can be exercised
// without real teammates. Each profile is shaped to win a different superlative
// and to give the two Alma trophies unambiguous winners.
import { writeFileSync } from "node:fs";

const DAYS = ["2026-07-20", "2026-07-21", "2026-07-22", "2026-07-23", "2026-07-24", "2026-07-25", "2026-07-26"];
const rows = [];

function cell(person, device_name, day, hour, tool, shape, intent, delegate_tool, turns, chars) {
  const provider = tool === "openclaw" ? "openclaw" : tool === "codex" ? "openai" : tool === "claude-code" ? "anthropic" : "openai";
  rows.push({
    schema: "aum/3", kind: "analytics_cell", device: device_name, person, device_name,
    day, hour, taxonomy_version: 4, prompt_version: 4,
    tool, tool_name: displayName(tool), provider, surface: tool.endsWith("-web") ? "web" : "cli",
    model: null, intent, shape, domain: "software_engineering", depth: delegate_tool === "none" ? 1 : 2,
    delegation: delegate_tool === "none" ? "none" : "agent_run", delegate_tool,
    turns, sessions: 1, chars,
  });
}
function span(person, device_name, day, tool, sessions, total, longest) {
  rows.push({
    schema: "aum/3", kind: "session_span", device: device_name, person, device_name,
    day, tool, tool_name: displayName(tool), sessions, total_minutes: total, longest_minutes: longest,
  });
}
function displayName(t) {
  return { openclaw: "Alma", "claude-code": "Claude Code", codex: "Codex", "chatgpt-web": "ChatGPT", "claude-web": "Claude", "gemini-web": "Gemini" }[t] ?? t;
}

// Dana — The Delegator + Alma-for-Claude-Code winner: heavy Alma runs driving Claude Code.
for (const d of DAYS) {
  cell("dana", "Dana's MBP", d, 14, "openclaw", "doing", "modify_or_debug_code", "claude_code", 9, 800);
  cell("dana", "Dana's MBP", d, 15, "openclaw", "doing", "write_code", "claude_code", 6, 640);
  span("dana", "Dana's MBP", d, "openclaw", 3, 95, 55);
}

// Ravi — The Scholar + Alma-for-Research winner: Alma used to ask, constantly.
for (const d of DAYS) {
  cell("ravi", "Ravi-Linux", d, 10, "openclaw", "asking", "multi_source_synthesis", "none", 11, 1400);
  cell("ravi", "Ravi-Linux", d, 11, "openclaw", "asking", "learning_or_explanation", "none", 8, 1100);
  cell("ravi", "Ravi-Linux", d, 12, "claude-web", "asking", "codebase_or_system_understanding", "none", 4, 500);
  span("ravi", "Ravi-Linux", d, "openclaw", 4, 120, 40);
}

// Kestrel — The Night Owl + Marathoner: long, late sessions.
for (const d of DAYS) {
  cell("kestrel", "kestrel-studio", d, 1, "codex", "doing", "automate_or_script", "none", 7, 900);
  cell("kestrel", "kestrel-studio", d, 23, "codex", "doing", "write_code", "none", 5, 700);
  span("kestrel", "kestrel-studio", d, "codex", 2, 180, 130);
}

// Ada — The Explorer: touches everything.
for (const d of DAYS) {
  for (const [h, t] of [[9, "chatgpt-web"], [13, "claude-code"], [16, "gemini-web"], [18, "codex"]]) {
    cell("ada", "ada-air", d, h, t, "doing", "review_or_critique", "none", 3, 400);
    span("ada", "ada-air", d, t, 1, 22, 18);
  }
}

// Milo — The Novelist: fewer, enormous prompts.
for (const d of DAYS) {
  cell("milo", "milo-mini", d, 17, "chatgpt-web", "doing", "write_prose", "none", 2, 4200);
  span("milo", "milo-mini", d, "chatgpt-web", 1, 30, 30);
}

const out = new URL("./synth-team.jsonl", import.meta.url);
writeFileSync(out, rows.map((r) => JSON.stringify(r)).join("\n") + "\n");
process.stderr.write(`wrote ${rows.length} rows to ${out.pathname}\n`);
