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
  const now = Date.now();
  db.exec("BEGIN");
  try {
    for (const r of rows) {
      const key = rowKey(r);
      if (!key || !r.day) {
        skipped++;
        continue;
      }
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
    db.exec("COMMIT");
  } catch (e) {
    db.exec("ROLLBACK");
    throw e;
  }
  return { stored, skipped };
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
      const { stored, skipped } = ingest(rows);
      return json(res, 200, { ok: true, received: rows.length, stored, skipped });
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
