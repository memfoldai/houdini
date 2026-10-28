#!/usr/bin/env node
import { readFileSync, writeFileSync } from "node:fs";
import { readFiles } from "./lib/parse.mjs";
import { weekWindow, filterToWeek, compute, lastWeekArg, almaHistory } from "./lib/metrics.mjs";
import { buildCards } from "./lib/cards.mjs";
import { renderHtml } from "./lib/render.mjs";

function parseArgs(argv) {
  const opts = { inputs: [] };
  for (const a of argv) {
    if (a.startsWith("--week=")) opts.week = a.slice(7);
    else if (a.startsWith("--team=")) opts.team = a.slice(7);
    else if (a.startsWith("--out=")) opts.out = a.slice(6);
    else if (a.startsWith("--personas=")) opts.personas = a.slice(11);
    else if (a.startsWith("--dictation=")) opts.dictation = a.slice(12);
    else if (a.startsWith("--")) throw new Error(`unknown flag: ${a}`);
    else opts.inputs.push(a);
  }
  return opts;
}

function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.inputs.length === 0) {
    process.stderr.write(
      "usage: generate.mjs [--week=YYYY-MM-DD] [--team=\"Name\"] [--out=file.html] [--personas=file.json] <analytics.jsonl...>\n",
    );
    process.exit(2);
  }

  const window = weekWindow(new Date(), opts.week ?? lastWeekArg());
  const { cells, spans, usage, skipped } = readFiles(opts.inputs);
  const wc = filterToWeek(cells, window);
  const ws = filterToWeek(spans, window);
  const wu = filterToWeek(usage, window);
  const metrics = compute(wc, ws, wu, window);
  const history = almaHistory(usage, window);
  const team = opts.team ?? "Houdini";
  const personas = opts.personas ? JSON.parse(readFileSync(opts.personas, "utf8")) : undefined;
  const dictation = opts.dictation ? JSON.parse(readFileSync(opts.dictation, "utf8")) : undefined;
  const cards = buildCards(metrics, { team, weekLabel: window.label, history, personas, dictation });
  const html = renderHtml(cards, { team, weekLabel: window.label });

  const out = opts.out ?? `wrapped-${window.start}.html`;
  writeFileSync(out, html);

  process.stderr.write(
    `wrapped: ${window.label} · ${metrics.peopleCount} ${metrics.peopleCount === 1 ? "person" : "people"} · ` +
      `${metrics.totalHours}h · ${wc.length} cells, ${ws.length} spans in window` +
      (skipped ? ` · ${skipped} rows skipped` : "") +
      `\nwrote ${out}\n`,
  );
}

main();
