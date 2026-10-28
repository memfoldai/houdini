// Day math runs on the `day` string as written by the device, which since
// Houdini 0.9.12 is that device's OWN local calendar day. Comparing the strings
// keeps a week boundary stable per device; the Date objects below are only ever
// used to name a weekday, never to shift a timestamp.

const DAY_MS = 86_400_000;
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DOW = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

function ymd(d) {
  return d.toISOString().slice(0, 10);
}

function atUtc(ymdStr) {
  return new Date(`${ymdStr}T00:00:00.000Z`);
}

function addDays(ymdStr, n) {
  return ymd(new Date(atUtc(ymdStr).getTime() + n * DAY_MS));
}

// Days since the most recent Monday. getUTCDay() is 0=Sun..6=Sat; the +6 %7
// rotation makes Monday the zero so a week runs Mon..Sun.
function mondayOf(ymdStr) {
  const dow = atUtc(ymdStr).getUTCDay();
  return addDays(ymdStr, -((dow + 6) % 7));
}

function pretty(startYmd, endYmd) {
  const s = atUtc(startYmd);
  const e = atUtc(endYmd);
  return `${MONTHS[s.getUTCMonth()]} ${s.getUTCDate()} – ${MONTHS[e.getUTCMonth()]} ${e.getUTCDate()}, ${e.getUTCFullYear()}`;
}

// `now` and `weekArg` are injected so callers stay deterministic in tests. With
// a weekArg the window is the Mon..Sun that contains it; without one it is the
// most recent week that has fully ended (so a Monday run reports the week that
// just closed, not the empty one in progress).
export function weekWindow(now, weekArg) {
  const start = weekArg ? mondayOf(weekArg) : addDays(mondayOf(ymd(now)), -7);
  const end = addDays(start, 6);
  return { start, end, label: pretty(start, end) };
}

export function filterToWeek(rows, window) {
  return rows.filter((r) => r.day >= window.start && r.day <= window.end);
}

// Row days are the device's own, but a person asking for "last week" means
// their own Monday. Seven days back names a day inside the week that just ended
// for the operator, which weekWindow expands to that whole Mon..Sun.
export function lastWeekArg(now = new Date()) {
  const t = new Date(now.getTime() - 7 * DAY_MS);
  const m = String(t.getMonth() + 1).padStart(2, "0");
  const d = String(t.getDate()).padStart(2, "0");
  return `${t.getFullYear()}-${m}-${d}`;
}

function inc(map, key, by) {
  map.set(key, (map.get(key) ?? 0) + by);
}

function bump(map, key) {
  let inner = map.get(key);
  if (!inner) {
    inner = new Map();
    map.set(key, inner);
  }
  return inner;
}

function ensurePerson(people, name) {
  let p = people.get(name);
  if (!p) {
    p = {
      person: name,
      turns: 0,
      turnsByTool: new Map(),
      intents: new Map(),
      domains: new Map(),
      hours: new Array(24).fill(0),
      // Prompts sent between midnight and 4am on the person's own clock.
      nightTurns: 0,
      // Sessions and minutes EXCLUDING Alma: an Alma voice command registers as
      // a session of ~zero length, so mixing it in would drag any per-session
      // average toward zero and make the two shapes of work incomparable.
      agentSessions: 0,
      agentMinutes: 0,
      almaSessions: 0,
      shortcutRuns: 0,
      shortcutsByName: new Map(),
      connectorRuns: 0,
      connectorsByApp: new Map(),
      lastDay: "",
    };
    people.set(name, p);
  }
  return p;
}

// A turn is the primary weight; sessions is the fallback so a labeled cell that
// predates turn counting still registers.
function weight(cell) {
  return cell.turns > 0 ? cell.turns : Math.max(cell.sessions, 1);
}

function hourLabel(h) {
  const period = h < 12 ? "AM" : "PM";
  const twelve = h % 12 === 0 ? 12 : h % 12;
  return `${twelve} ${period}`;
}

// `pushes` maps person → the local day of that device's most recent upload.
// It is the only honest basis for "stopped reporting": a device can upload on
// Sunday night and simply have had nothing to say since Thursday.
export function compute(cells, spans, usage = [], window = null, pushes = null) {
  const people = new Map();
  const toolTurns = new Map();
  const toolNames = new Map();
  const domains = new Map();
  const hours = new Array(24).fill(0);

  let totalMinutes = 0;
  let totalTurns = 0;

  for (const s of spans) {
    const p = ensurePerson(people, s.person);
    if (s.day > p.lastDay) p.lastDay = s.day;
    if (s.tool === "openclaw") p.almaSessions += s.sessions;
    else {
      p.agentSessions += s.sessions;
      p.agentMinutes += s.total_minutes;
    }
    totalMinutes += s.total_minutes;
    if (!toolNames.has(s.tool)) toolNames.set(s.tool, s.tool_name);
  }

  for (const c of cells) {
    const p = ensurePerson(people, c.person);
    const w = weight(c);
    if (c.day > p.lastDay) p.lastDay = c.day;
    p.turns += w;
    inc(p.turnsByTool, c.tool, w);
    inc(p.intents, c.intent, w);
    if (c.domain !== "other") inc(p.domains, c.domain, w);
    p.hours[c.hour] += w;
    if (c.hour <= 4) p.nightTurns += w;

    totalTurns += w;
    inc(toolTurns, c.tool, w);
    if (!toolNames.has(c.tool)) toolNames.set(c.tool, c.tool_name);
    if (c.domain !== "other") inc(domains, c.domain, w);
    hours[c.hour] += w;
  }

  for (const u of usage) {
    const p = ensurePerson(people, u.person);
    if (u.day > p.lastDay) p.lastDay = u.day;
    if (u.kind === SHORTCUT) {
      p.shortcutRuns += u.runs;
      inc(p.shortcutsByName, u.name, u.runs);
    } else {
      p.connectorRuns += u.runs;
      inc(p.connectorsByApp, u.name, u.runs);
    }
  }

  const roster = [...people.values()];
  for (const p of roster) {
    p.topTool = topEntry(p.turnsByTool);
    p.topToolName = p.topTool ? (toolNames.get(p.topTool.key) ?? p.topTool.key) : null;
    p.topIntent = topEntry(p.intents);
    p.topDomain = topEntry(p.domains);
    p.topShortcut = topEntry(p.shortcutsByName);
    p.peakHour = p.turns > 0 ? p.hours.indexOf(Math.max(...p.hours)) : null;
    p.almaActions = p.shortcutRuns + p.connectorRuns;
  }

  const alma = almaSection(usage, spans, window, { turns: toolTurns.get("openclaw") ?? 0, totalTurns });

  const toolShare = rank(toolTurns, toolNames);
  // Asking Alma for something out loud never produces a prompt, so ranking the
  // tools by prompt alone scores most of Alma's week as nothing at all. A
  // REQUEST is the unit the two shapes share: one typed prompt, or one shortcut
  // a person set off. Connector calls are deliberately excluded — they are what
  // a request went on to do, not a second thing anyone asked for.
  const requests = new Map(toolTurns);
  if (alma.shortcutRuns > 0) inc(requests, "openclaw", alma.shortcutRuns);
  const requestShare = rank(requests, toolNames);

  const domainTotal = [...domains.values()].reduce((a, b) => a + b, 0);
  const domainRank = [...domains.entries()]
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .map(([key, n]) => ({ key, label: domainTitle(key), pct: domainTotal > 0 ? Math.round((n / domainTotal) * 100) : 0 }));

  let peakHour = 0;
  for (let h = 1; h < 24; h++) if (hours[h] > hours[peakHour]) peakHour = h;

  // A device that checks in with one empty session is a device, not a person
  // in the week's story; headline counts follow the people who did something.
  const active = roster.filter((p) => p.turns > 0 || p.almaActions > 0);

  return {
    people: roster,
    peopleCount: active.length,
    totalMinutes,
    totalHours: Math.round(totalMinutes / 60),
    totalTurns,
    tools: toolShare,
    toolShare,
    requestShare,
    totalRequests: [...requests.values()].reduce((a, b) => a + b, 0),
    topTool: requestShare[0] ?? null,
    domainRank,
    peakHour,
    peakHourLabel: hourLabel(peakHour),
    alma,
    coverage: coverageOf(roster, window, pushes),
  };
}

function rank(counts, names) {
  const total = [...counts.values()].reduce((a, b) => a + b, 0);
  return [...counts.entries()]
    .map(([tool, turns]) => ({
      tool,
      name: names.get(tool) ?? tool,
      turns,
      pct: total > 0 ? Math.round((turns / total) * 100) : 0,
    }))
    .sort((a, b) => b.turns - a.turns || a.tool.localeCompare(b.tool));
}

// Without push times no claim is made at all: rows alone cannot separate a
// quiet weekend from a sleeping laptop, and guessing either way misleads.
function coverageOf(roster, window, pushes) {
  if (!window || !pushes) return { total: roster.length, quiet: [] };
  const quiet = roster
    .filter((p) => (pushes.get(p.person) ?? "") < window.end)
    .map((p) => ({ person: p.person, lastDay: pushes.get(p.person) ?? "" }))
    .sort((a, b) => a.lastDay.localeCompare(b.lastDay) || a.person.localeCompare(b.person));
  return { total: roster.length, quiet };
}

function topEntry(map) {
  let best = null;
  for (const [key, value] of map) {
    if (!best || value > best.value || (value === best.value && key < best.key)) best = { key, value };
  }
  return best;
}

const SHORTCUT = "shortcut";
const CONNECTOR = "connector";

// One capability arrives under several spellings — `mcp.file-ops.annotate` and
// `file-ops.annotate`, `recap.day` and `recap_day` — because different ingest
// sources (app-runtime args, hook events, narrator log) name the same call
// differently. Bucketing on the leading segment, after dropping the `mcp`
// transport prefix, is the only normalization that survives all three.
const CAPABILITY_BUCKETS = {
  people: "people lookup",
  document: "docs & decks",
  docs: "docs & decks",
  presentation: "docs & decks",
  maps: "maps & trips",
  "trip-plan": "maps & trips",
  email: "email",
  gmail: "email",
  calendar: "calendar",
  schedule: "calendar",
  visualise: "visualise",
  recap: "day recap",
  compare: "compare",
  "file-ops": "annotate files",
  obsidian: "notes",
  agent: "coding agents",
  slack: "slack",
  terminal: "terminal",
};

function capabilityBucket(detail) {
  const family = String(detail ?? "")
    .replace(/^mcp[._]/, "")
    .split(/[._]/)[0];
  if (!family) return "other";
  return CAPABILITY_BUCKETS[family] ?? family.replace(/[-_]/g, " ");
}

function tally(rows, keyOf) {
  const runs = new Map();
  const who = new Map();
  for (const r of rows) {
    const key = keyOf(r);
    inc(runs, key, r.runs);
    if (!who.has(key)) who.set(key, new Set());
    who.get(key).add(r.person);
  }
  return [...runs.entries()]
    .map(([key, n]) => ({ key, runs: n, people: who.get(key).size }))
    .sort((a, b) => b.runs - a.runs || a.key.localeCompare(b.key));
}

function almaSection(usage, spans, window, prompts) {
  const shortcuts = usage.filter((u) => u.kind === SHORTCUT);
  const connectors = usage.filter((u) => u.kind === CONNECTOR);
  const sum = (rows) => rows.reduce((a, r) => a + r.runs, 0);

  const shortcutRuns = sum(shortcuts);
  const byName = tally(shortcuts, (r) => r.name);
  const voiceRuns = sum(shortcuts.filter((r) => r.detail === "voice"));

  const dayRuns = new Map();
  for (const r of shortcuts) inc(dayRuns, r.day, r.runs);
  const connectorByDay = new Map();
  for (const r of connectors) inc(connectorByDay, r.day, r.runs);
  const days = window
    ? Array.from({ length: 7 }, (_, i) => addDays(window.start, i))
    : [...new Set(usage.map((u) => u.day))].sort();

  const almaSpans = spans.filter((s) => s.tool === "openclaw");
  const longestByPerson = new Map();
  for (const s of almaSpans) {
    longestByPerson.set(s.person, Math.max(longestByPerson.get(s.person) ?? 0, s.longest_minutes));
  }

  return {
    shortcutRuns,
    connectorRuns: sum(connectors),
    actions: shortcutRuns + sum(connectors),
    distinctShortcuts: byName.length,
    peopleUsing: new Set(usage.map((u) => u.person)).size,
    voiceRuns,
    voicePct: shortcutRuns > 0 ? Math.round((voiceRuns / shortcutRuns) * 100) : 0,
    byName,
    byReach: [...byName].sort((a, b) => b.people - a.people || b.runs - a.runs || a.key.localeCompare(b.key)),
    byDay: days.map((day) => ({
      day,
      label: DOW[atUtc(day).getUTCDay()],
      shortcuts: dayRuns.get(day) ?? 0,
      connectors: connectorByDay.get(day) ?? 0,
    })),
    capabilities: tally(connectors, (r) => capabilityBucket(r.detail)),
    sessions: almaSpans.reduce((a, s) => a + s.sessions, 0),
    minutes: Math.round(almaSpans.reduce((a, s) => a + s.total_minutes, 0)),
    oneShotPeople: [...longestByPerson.values()].filter((m) => m <= 2).length,
    peopleWithSessions: longestByPerson.size,
    turns: prompts.turns,
  };
}

// "Which shortcut is genuinely new?" is only answerable against history, so
// callers pass UNFILTERED rows; the window marks which runs count as the debut.
export function almaHistory(allUsage, window) {
  const firstSeen = new Map();
  for (const u of allUsage) {
    if (u.kind !== SHORTCUT) continue;
    const prev = firstSeen.get(u.name);
    if (!prev || u.day < prev) firstSeen.set(u.name, u.day);
  }
  const inWeek = allUsage.filter(
    (u) => u.kind === SHORTCUT && u.day >= window.start && u.day <= window.end && firstSeen.get(u.name) >= window.start,
  );
  return { debuts: tally(inWeek, (r) => r.name) };
}

// Domain ids come from the taxonomy; these are the human-facing category names.
const DOMAIN_TITLES = {
  software_engineering: "Software Engineering",
  data_and_analytics: "Data & Analytics",
  infrastructure_and_devops: "Infra & DevOps",
  security: "Security",
  product_and_design: "Product & Design",
  research_and_science: "Research",
  business_and_finance: "Business & Finance",
  marketing_and_sales: "Marketing & Sales",
  legal_and_compliance: "Legal & Compliance",
  people_and_hiring: "People & Hiring",
  education_and_learning: "Learning",
  health_and_medicine: "Health",
  personal_and_lifestyle: "Personal",
  creative_and_media: "Creative & Media",
  customer_support: "Customer Support",
  operations_and_admin: "Ops & Admin",
};
function domainTitle(key) {
  return DOMAIN_TITLES[key] ?? key.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}

export const _internal = { mondayOf, addDays, hourLabel, capabilityBucket };
