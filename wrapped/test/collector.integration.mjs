#!/usr/bin/env node
// Spawns the real collector entrypoint against a throwaway DB and exercises the
// full path: unauthorized rejection, ingest, export, and a rendered wrapped.
import { spawn } from "node:child_process";
import { readFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import assert from "node:assert/strict";

const PORT = 8791;
const INGEST = "ingest-secret-abc";
const ADMIN = "admin-secret-xyz";
const base = `http://127.0.0.1:${PORT}`;
const dir = mkdtempSync(join(tmpdir(), "collector-it-"));

const child = spawn(process.execPath, ["collector/server.mjs"], {
  cwd: new URL("..", import.meta.url).pathname,
  env: {
    ...process.env,
    PORT: String(PORT),
    DB_PATH: join(dir, "c.sqlite"),
    WRAPPED_DIR: join(dir, "wrapped"),
    TEAM_NAME: "Integration Team",
    INGEST_TOKEN: INGEST,
    ADMIN_TOKEN: ADMIN,
  },
  stdio: ["ignore", "inherit", "inherit"],
});

async function waitReady() {
  for (let i = 0; i < 50; i++) {
    try {
      const r = await fetch(`${base}/health`);
      if (r.ok) return;
    } catch {}
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error("collector did not become ready");
}

function cleanup() {
  child.kill("SIGKILL");
  rmSync(dir, { recursive: true, force: true });
}

try {
  await waitReady();

  const noAuth = await fetch(`${base}/v1/ingest`, { method: "POST", body: "{}" });
  assert.equal(noAuth.status, 401, "ingest rejects a missing token");

  const synth = readFileSync(new URL("../fixtures/synth-team.jsonl", import.meta.url), "utf8");
  const real = readFileSync(
    join(process.env.HOME, "Library/Application Support/ai.memfold.houdini/data/analytics.jsonl"),
    "utf8",
  );
  const ing = await fetch(`${base}/v1/ingest`, {
    method: "POST",
    headers: { authorization: `Bearer ${INGEST}` },
    body: synth + "\n" + real,
  });
  const ingBody = await ing.json();
  assert.equal(ing.status, 200);
  assert.ok(ingBody.stored > 0, "rows stored");

  // Re-ingest is idempotent: same keys overwrite, count of stored rows is stable.
  const again = await fetch(`${base}/v1/ingest`, {
    method: "POST",
    headers: { authorization: `Bearer ${INGEST}` },
    body: synth,
  });
  const againBody = await again.json();
  assert.equal(againBody.pruned, 0, "an identical re-push prunes nothing");

  // A push is the device's full snapshot: pushing a REDUCED export for one
  // device deletes that device's rows the new push no longer contains (a
  // re-scan re-dated a day, a relabel moved a dimension), and touches nobody else.
  const synthLines = synth.trim().split("\n");
  const firstDevice = JSON.parse(synthLines[0]).device;
  const deviceRows = synthLines.filter((l) => JSON.parse(l).device === firstDevice);
  const reduced = await fetch(`${base}/v1/ingest`, {
    method: "POST",
    headers: { authorization: `Bearer ${INGEST}` },
    body: deviceRows[0],
  });
  const reducedBody = await reduced.json();
  assert.equal(reducedBody.pruned, deviceRows.length - 1, "superseded rows of that device are pruned");

  const exp = await fetch(`${base}/v1/export?week=2026-07-22&key=${ADMIN}`);
  assert.equal(exp.status, 200);
  const ndjson = await exp.text();
  const lines = ndjson.trim().split("\n").filter(Boolean);
  assert.ok(lines.length > 0, "export returns rows in the window");
  for (const l of lines) JSON.parse(l);

  const wrapped = await fetch(`${base}/v1/wrapped/2026-07-20?key=${ADMIN}`);
  assert.equal(wrapped.status, 200);
  const html = await wrapped.text();
  assert.ok(html.startsWith("<!doctype html>"));
  assert.ok(html.includes("Alma × Claude Code"));
  assert.ok(html.includes("Integration Team"));

  const wrappedNoAuth = await fetch(`${base}/v1/wrapped/2026-07-20`);
  assert.equal(wrappedNoAuth.status, 401, "wrapped needs the admin token");

  console.log(`OK — ingest stored ${ingBody.stored}, export ${lines.length} rows, wrapped rendered`);
  cleanup();
  process.exit(0);
} catch (e) {
  console.error("FAIL:", e.message);
  cleanup();
  process.exit(1);
}
