#!/usr/bin/env node
// Team analytics collector (System 2). Zero external dependencies: Node's own
// http server and node:sqlite (available flag-free since Node 22.13 / 23.4).
// Devices POST their redacted analytics rows; the laptop pulls a week or opens
// the rendered wrapped. Storage is one SQLite file on a mounted volume so the
// container is disposable and the data survives it.
import { createServer } from "node:http";
import { DatabaseSync } from "node:sqlite";
import { timingSafeEqual } from "node:crypto";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

import { collect, parseNdjson, rowKey } from "../lib/parse.mjs";
import { weekWindow, filterToWeek, compute, lastWeekArg } from "../lib/metrics.mjs";
import { buildCards } from "../lib/cards.mjs";
import { renderHtml } from "../lib/render.mjs";

const PORT = Number(process.env.PORT ?? 8787);
const DB_PATH = process.env.DB_PATH ?? "/data/collector.sqlite";
const WRAPPED_DIR = process.env.WRAPPED_DIR ?? "/data/wrapped";
const TEAM_NAME = process.env.TEAM_NAME ?? "Houdini";
const INGEST_TOKEN = process.env.INGEST_TOKEN ?? "";
const ADMIN_TOKEN = process.env.ADMIN_TOKEN ?? "";
const MAX_BODY = 32 * 1024 * 1024;
const ARCHIVE_EVERY_MS = 6 * 60 * 60 * 1000;

mkdirSync(dirname(DB_PATH), { recursive: true });
const db = new DatabaseSync(DB_PATH);
db.exec("PRAGMA journal_mode = WAL");
db.exec("PRAGMA busy_timeout = 5000");
db.exec(`CREATE TABLE IF NOT EXISTS rows (
  key TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  day TEXT NOT NULL,
  person TEXT,
  device TEXT,
  received_ms INTEGER NOT NULL,
  body TEXT NOT NULL
)`);
db.exec("CREATE INDEX IF NOT EXISTS idx_rows_day ON rows(day)");

const upsert = db.prepare(`INSERT INTO rows (key, kind, day, person, device, received_ms, body)
  VALUES (:key, :kind, :day, :person, :device, :received, :body)
  ON CONFLICT(key) DO UPDATE SET
    day = excluded.day, person = excluded.person, device = excluded.device,
    received_ms = excluded.received_ms, body = excluded.body`);
// A device push is its FULL current export in one POST (every row of a push
// shares one received_ms), so a row of that device left at an older
// received_ms was re-keyed or dropped upstream — a re-scan re-dated a day, a
// relabel moved a dimension — and keeping it would double-count. Prune per
// push. A chunked uploader would break this contract; the device sends one body.
const pruneStale = db.prepare("DELETE FROM rows WHERE device = :device AND received_ms < :received");
const selectWindow = db.prepare("SELECT body FROM rows WHERE day >= :start AND day <= :end ORDER BY day");

// Length is compared first because timingSafeEqual throws on a mismatch; an
// empty configured token means the endpoint is disabled (fail closed), never
// open-to-all.
function authorized(req, token) {
  if (!token) return false;
  const header = req.headers.authorization ?? "";
  const url = new URL(req.url, "http://x");
  const presented = header.startsWith("Bearer ") ? header.slice(7) : url.searchParams.get("key") ?? "";
  const a = Buffer.from(presented);
  const b = Buffer.from(token);
  return a.length === b.length && timingSafeEqual(a, b);
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    let size = 0;
    const chunks = [];
    req.on("data", (c) => {
      size += c.length;
      if (size > MAX_BODY) {
        reject(new Error("body too large"));
        req.destroy();
        return;
      }
      chunks.push(c);
    });
    req.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    req.on("error", reject);
  });
}

function json(res, code, obj) {
  const s = JSON.stringify(obj);
  res.writeHead(code, { "content-type": "application/json", "content-length": Buffer.byteLength(s) });
  res.end(s);
}

function ingest(rows) {
  let stored = 0;
  let skipped = 0;
  let pruned = 0;
  const now = Date.now();
  const devices = new Set();
  db.exec("BEGIN");
  try {
    for (const r of rows) {
      const key = rowKey(r);
      if (!key || !r.day) {
        skipped++;
        continue;
      }
      if (r.device != null) devices.add(String(r.device));
      upsert.run({
        key,
        kind: r.kind,
        day: String(r.day),
        person: r.person == null ? null : String(r.person),
        device: r.device == null ? null : String(r.device),
        received: now,
        body: JSON.stringify(r),
      });
      stored++;
    }
    for (const device of devices) pruned += Number(pruneStale.run({ device, received: now }).changes);
    db.exec("COMMIT");
  } catch (e) {
    db.exec("ROLLBACK");
    throw e;
  }
  return { stored, skipped, pruned };
}

function windowRows(window) {
  const rows = selectWindow.all({ start: window.start, end: window.end }).map((r) => JSON.parse(r.body));
  return collect(rows);
}

function renderWrapped(weekArg) {
  const window = weekWindow(new Date(), weekArg ?? lastWeekArg());
  const { cells, spans, delegations } = windowRows(window);
  const wc = filterToWeek(cells, window);
  const ws = filterToWeek(spans, window);
  const wd = filterToWeek(delegations, window);
  const metrics = compute(wc, ws, wd);
  const cards = buildCards(metrics, { team: TEAM_NAME, weekLabel: window.label });
  return { window, metrics, html: renderHtml(cards, { team: TEAM_NAME, weekLabel: window.label }) };
}

const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url, "http://x");
    const path = url.pathname;

    if (req.method === "GET" && (path === "/health" || path === "/")) {
      res.writeHead(200, { "content-type": "text/plain" });
      res.end(path === "/" ? "houdini team-wrapped collector\nPOST /v1/ingest · GET /v1/export?week= · GET /v1/wrapped/<monday>\n" : "ok");
      return;
    }

    if (req.method === "POST" && path === "/v1/ingest") {
      if (!authorized(req, INGEST_TOKEN)) return json(res, 401, { error: "unauthorized" });
      const body = await readBody(req);
      const rows = parseNdjson(body).filter(Boolean);
      const { stored, skipped, pruned } = ingest(rows);
      return json(res, 200, { ok: true, received: rows.length, stored, skipped, pruned });
    }

    if (req.method === "GET" && path === "/v1/export") {
      if (!authorized(req, ADMIN_TOKEN)) return json(res, 401, { error: "unauthorized" });
      const window = weekWindow(new Date(), url.searchParams.get("week") ?? lastWeekArg());
      const rows = selectWindow.all({ start: window.start, end: window.end });
      res.writeHead(200, { "content-type": "application/x-ndjson" });
      for (const r of rows) res.write(r.body + "\n");
      res.end();
      return;
    }

    if (req.method === "GET" && path === "/v1/board") {
      if (!authorized(req, ADMIN_TOKEN)) return json(res, 401, { error: "unauthorized" });
      res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      res.end(renderBoard());
      return;
    }

    if (req.method === "GET" && path.startsWith("/v1/wrapped")) {
      if (!authorized(req, ADMIN_TOKEN)) return json(res, 401, { error: "unauthorized" });
      const fromPath = path.replace(/^\/v1\/wrapped\/?/, "");
      const weekArg = fromPath || url.searchParams.get("week") || undefined;
      const { html } = renderWrapped(weekArg);
      res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      res.end(html);
      return;
    }

    res.writeHead(404, { "content-type": "text/plain" });
    res.end("not found");
  } catch (e) {
    json(res, e.message === "body too large" ? 413 : 500, { error: e.message });
  }
});

// "Automatic weekly": on boot and every few hours, write the last complete
// week's wrapped to the volume. Overwrites are intentional — laptops that were
// asleep at the Monday boundary keep uploading for days, and each pass folds in
// whatever has arrived since.
function archive() {
  try {
    const { window, metrics, html } = renderWrapped(undefined);
    mkdirSync(WRAPPED_DIR, { recursive: true });
    const slug = TEAM_NAME.replace(/\s+/g, "-").toLowerCase();
    const out = join(WRAPPED_DIR, `${slug}-${window.start}.html`);
    writeFileSync(out, html);
    log(`archived ${window.label}: ${metrics.peopleCount} people, ${metrics.totalHours}h -> ${out}`);
  } catch (e) {
    log(`archive failed: ${e.message}`);
  }
}

function localDay(d = new Date()) {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${dd}`;
}

function renderBoard() {
  const today = localDay();
  const people = db.prepare("SELECT person, MAX(received_ms) m FROM rows GROUP BY person ORDER BY m DESC").all();
  const rows = [];
  for (const p of people) {
    const last = db.prepare("SELECT body FROM rows WHERE person = :p ORDER BY received_ms DESC LIMIT 1").get({ p: p.person });
    const lastBody = JSON.parse(last.body);
    const version = lastBody.app_version ?? "pre-0.9.4";
    const almanac = lastBody.almanac_version ?? "—";
    const agg = (kind) =>
      db.prepare("SELECT json_extract(body,'$.name') n, SUM(json_extract(body,'$.runs')) r FROM rows WHERE person = :p AND kind = :k AND day = :d GROUP BY n ORDER BY r DESC")
        .all({ p: p.person, k: kind, d: today });
    const sc = agg("shortcut");
    const co = agg("connector");
    rows.push({
      person: p.person,
      version,
      almanac,
      ageMin: Math.round((Date.now() - p.m) / 60000),
      shortcuts: sc.reduce((a, x) => a + x.r, 0),
      connectors: co.reduce((a, x) => a + x.r, 0),
      scDetail: sc.map((x) => `${x.n} ×${x.r}`).join(", "),
      coDetail: co.map((x) => `${x.n || "unknown"} ×${x.r}`).join(", "),
    });
  }
  rows.sort((a, b) => b.shortcuts - a.shortcuts || b.connectors - a.connectors || a.ageMin - b.ageMin);
  const esc = (v) => String(v ?? "").replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
  const tr = rows.map((r, i) => `<tr class="${r.ageMin > 60 ? "stale" : ""}">
    <td>${i + 1}</td><td class="who">${esc(r.person)}</td><td>${esc(r.version)}</td><td>${esc(r.almanac)}</td>
    <td class="num">${r.ageMin < 60 ? r.ageMin + "m" : (r.ageMin / 60).toFixed(1) + "h"}</td>
    <td class="num big">${r.shortcuts}</td><td class="det">${esc(r.scDetail) || "—"}</td>
    <td class="num big">${r.connectors}</td><td class="det">${esc(r.coDetail) || "—"}</td>
  </tr>`).join("");
  return `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta http-equiv="refresh" content="30"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Houdini Board · ${today}</title><style>
body{font-family:"Helvetica Neue",system-ui,sans-serif;background:#0A0A0A;color:#FAF7F0;margin:0;padding:32px}
h1{font-size:20px;letter-spacing:.08em;text-transform:uppercase;margin:0 0 4px}
.sub{color:#D6FF3D;font-size:13px;font-weight:700;letter-spacing:.12em;text-transform:uppercase;margin-bottom:24px}
table{border-collapse:collapse;width:100%;font-size:14px}
th{color:#D6FF3D;text-align:left;font-size:11px;letter-spacing:.1em;text-transform:uppercase;padding:8px 12px;border-bottom:2px solid #333}
td{padding:9px 12px;border-bottom:1px solid #1e1e1e;vertical-align:top}
.who{font-weight:800}.num{font-variant-numeric:tabular-nums}.big{font-weight:800;font-size:16px}
.det{color:#999;max-width:320px}.stale td{opacity:.4}
</style></head><body>
<h1>Houdini — today's board</h1>
<div class="sub">${today} · auto-refreshes every 30s</div>
<table><tr><th>#</th><th>person</th><th>houdini</th><th>almanac</th><th>last push</th><th>shortcuts</th><th>detail</th><th>connectors</th><th>detail</th></tr>${tr}</table>
</body></html>`;
}

function log(msg) {
  process.stdout.write(`[collector] ${msg}\n`);
}

server.listen(PORT, () => {
  log(`listening on :${PORT} · db ${DB_PATH}`);
  if (!INGEST_TOKEN) log("WARNING: INGEST_TOKEN unset — ingest is disabled until you set it");
  if (!ADMIN_TOKEN) log("WARNING: ADMIN_TOKEN unset — export and wrapped are disabled until you set it");
  archive();
  setInterval(archive, ARCHIVE_EVERY_MS).unref();
});
