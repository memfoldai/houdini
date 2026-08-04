import { readFileSync } from "node:fs";

// The analytics export (aum/3) interleaves three row kinds in one NDJSON
// stream. The wrapped only needs the two aggregate kinds; label_candidate rows
// are taxonomy-tuning telemetry and are dropped here.
export const CELL = "analytics_cell";
export const SPAN = "session_span";
export const DELEGATION = "delegation";
export const CONNECTOR = "connector";
export const SHORTCUT = "shortcut";

// A cell's identity is its full dimension tuple. Re-uploading a device's export
// as more of its history gets labeled produces the same tuple with larger
// counts, so keeping the last occurrence per key makes a merge idempotent and
// matches the collector's UPSERT semantics.
function cellKey(r) {
  return [
    r.device,
    r.person,
    r.day,
    r.hour,
    r.tool,
    r.provider,
    r.surface,
    r.model ?? "",
    r.intent,
    r.domain,
    r.depth,
    r.delegation,
    r.delegate_tool,
  ].join("");
}

// A span is one device's rollup for a tool on a day; the same key re-appears
// with a fuller total as sessions accrue, so last-wins is again correct.
function spanKey(r) {
  return [r.device, r.person, r.day, r.tool].join("");
}

// The collector stores one row per dimension key so a re-upload overwrites
// rather than duplicates — the same idempotency the in-memory merge relies on.
// Returns null for kinds the wrapped ignores (label_candidate), which the
// collector drops on ingest.
export function rowKey(r) {
  if (!r || typeof r !== "object") return null;
  if (r.kind === CELL) return `${CELL}:${cellKey(r)}`;
  if (r.kind === SPAN) return `${SPAN}:${spanKey(r)}`;
  if (r.kind === DELEGATION) return `${DELEGATION}:${delegKey(r)}`;
  if (r.kind === CONNECTOR || r.kind === SHORTCUT) return `${r.kind}:${usageKey(r)}`;
  return null;
}

// A delegation: how many times a driving tool (Alma) drove a driven tool
// (Claude Code) on a day. Detected from the transcript, so re-upload replaces.
function delegKey(r) {
  return [r.device, r.person, r.day, r.tool, r.driven_tool].join("");
}

function usageKey(r) {
  return [r.device, r.person, r.day, r.name, r.detail].join("");
}

function coerceCell(r) {
  return {
    device: String(r.device ?? ""),
    person: String(r.person ?? "unknown"),
    device_name: String(r.device_name ?? ""),
    day: String(r.day ?? ""),
    hour: Number.isFinite(r.hour) ? r.hour : 0,
    tool: String(r.tool ?? "other"),
    tool_name: String(r.tool_name ?? r.tool ?? "other"),
    provider: String(r.provider ?? ""),
    surface: String(r.surface ?? ""),
    model: r.model == null ? null : String(r.model),
    intent: String(r.intent ?? "other"),
    shape: String(r.shape ?? "other"),
    domain: String(r.domain ?? "other"),
    depth: Number.isFinite(r.depth) ? r.depth : 1,
    delegation: String(r.delegation ?? "none"),
    delegate_tool: String(r.delegate_tool ?? "none"),
    turns: Number.isFinite(r.turns) ? r.turns : 0,
    sessions: Number.isFinite(r.sessions) ? r.sessions : 0,
    chars: Number.isFinite(r.chars) ? r.chars : 0,
  };
}

function coerceSpan(r) {
  return {
    device: String(r.device ?? ""),
    person: String(r.person ?? "unknown"),
    device_name: String(r.device_name ?? ""),
    day: String(r.day ?? ""),
    tool: String(r.tool ?? "other"),
    tool_name: String(r.tool_name ?? r.tool ?? "other"),
    sessions: Number.isFinite(r.sessions) ? r.sessions : 0,
    total_minutes: Number.isFinite(r.total_minutes) ? r.total_minutes : 0,
    longest_minutes: Number.isFinite(r.longest_minutes) ? r.longest_minutes : 0,
  };
}

// Accepts already-parsed row objects (the collector path) or raw NDJSON text
// (the file path). Malformed lines are skipped, never fatal: one device with a
// truncated upload must not sink a whole team's wrapped.
export function collect(rows) {
  const cells = new Map();
  const spans = new Map();
  const delegations = new Map();
  const usage = new Map();
  let skipped = 0;
  for (const r of rows) {
    if (!r || typeof r !== "object") {
      skipped++;
      continue;
    }
    if (r.kind === CELL) cells.set(cellKey(r), coerceCell(r));
    else if (r.kind === SPAN) spans.set(spanKey(r), coerceSpan(r));
    else if (r.kind === DELEGATION) delegations.set(delegKey(r), coerceDelegation(r));
    else if (r.kind === CONNECTOR || r.kind === SHORTCUT) usage.set(`${r.kind}:${usageKey(r)}`, coerceUsage(r));
  }
  return {
    cells: [...cells.values()],
    spans: [...spans.values()],
    delegations: [...delegations.values()],
    usage: [...usage.values()],
    skipped,
  };
}

function coerceUsage(r) {
  return {
    kind: String(r.kind),
    device: String(r.device ?? ""),
    person: String(r.person ?? ""),
    day: String(r.day ?? ""),
    name: String(r.name ?? ""),
    detail: String(r.detail ?? ""),
    runs: Number(r.runs ?? 0),
  };
}

function coerceDelegation(r) {
  return {
    person: String(r.person ?? "unknown"),
    day: String(r.day ?? ""),
    tool: String(r.tool ?? "other"),
    driven_tool: String(r.driven_tool ?? "other"),
    turns: Number.isFinite(r.turns) ? r.turns : 0,
  };
}

export function parseNdjson(text) {
  const rows = [];
  for (const line of text.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    try {
      rows.push(JSON.parse(trimmed));
    } catch {
      rows.push(null);
    }
  }
  return rows;
}

export function readFiles(paths) {
  const rows = [];
  for (const p of paths) {
    rows.push(...parseNdjson(readFileSync(p, "utf8")));
  }
  return collect(rows);
}
