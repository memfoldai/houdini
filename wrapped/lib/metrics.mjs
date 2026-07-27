// All day math is UTC because the export's `day` field is a UTC calendar day
// (Rust ymd_utc). Comparing UTC-day strings keeps the week boundary identical
// on every device regardless of local timezone.

const DAY_MS = 86_400_000;
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

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
  const left = `${MONTHS[s.getUTCMonth()]} ${s.getUTCDate()}`;
  const right = `${MONTHS[e.getUTCMonth()]} ${e.getUTCDate()}`;
  return `${left} – ${right}, ${e.getUTCFullYear()}`;
}

// `now` and `weekArg` are injected so callers stay deterministic in tests. With
// a weekArg the window is the Mon..Sun that contains it; without one it is the
// most recent week that has fully ended (so a Monday run reports the week that
// just closed, not the empty one in progress).
export function weekWindow(now, weekArg) {
  const anchor = weekArg ? mondayOf(weekArg) : addDays(mondayOf(ymd(now)), -7);
  const start = anchor;
  const end = addDays(start, 6);
  return { start, end, label: pretty(start, end) };
}

export function filterToWeek(rows, window) {
  return rows.filter((r) => r.day >= window.start && r.day <= window.end);
}

// Row days are UTC, but a person deciding "generate last week" means their own
// Monday. So the default anchor is built from local calendar parts: seven days
// back names a day inside the week that just ended for the operator, which
// weekWindow then expands to that whole Mon..Sun. The at-most-a-day edge skew
// against UTC row-days is immaterial at weekly resolution.
export function lastWeekArg(now = new Date()) {
  const t = new Date(now.getTime() - 7 * DAY_MS);
  const m = String(t.getMonth() + 1).padStart(2, "0");
  const d = String(t.getDate()).padStart(2, "0");
  return `${t.getFullYear()}-${m}-${d}`;
}

function inc(map, key, by) {
  map.set(key, (map.get(key) ?? 0) + by);
}

function ensurePerson(people, name) {
  let p = people.get(name);
  if (!p) {
    p = {
      person: name,
      minutes: 0,
      longest: 0,
      turns: 0,
      askingTurns: 0,
      doingTurns: 0,
      delegateTurns: 0,
      lateTurns: 0,
      earlyTurns: 0,
      chars: 0,
      tools: new Set(),
      almaClaudeCode: 0,
      almaBuild: 0,
      almaResearch: 0,
    };
    people.set(name, p);
  }
  return p;
}

// A turn counts as a "primary weight" for hour/shape histograms; sessions is the
// fallback so a labeled cell that predates turn counting still registers.
function weight(cell) {
  return cell.turns > 0 ? cell.turns : Math.max(cell.sessions, 1);
}

const LATE_HOURS = new Set([22, 23, 0, 1, 2, 3, 4, 5]);
const EARLY_HOURS = new Set([5, 6, 7, 8, 9]);

function hourLabel(h) {
  const period = h < 12 ? "AM" : "PM";
  const twelve = h % 12 === 0 ? 12 : h % 12;
  return `${twelve} ${period}`;
}

export function compute(cells, spans) {
  const people = new Map();
  const toolTurns = new Map();
  const toolNames = new Map();
  const toolDomain = new Map();
  const domains = new Map();
  const days = new Map();
  const hours = new Array(24).fill(0);
  const shape = { asking: 0, doing: 0, expressing: 0, other: 0 };

  let totalMinutes = 0;
  let totalTurns = 0;
  let delegateTurns = 0;
  let delegateCells = 0;
  let longestSession = { minutes: 0, person: null };

  for (const s of spans) {
    const p = ensurePerson(people, s.person);
    p.minutes += s.total_minutes;
    p.longest = Math.max(p.longest, s.longest_minutes);
    p.tools.add(s.tool);
    totalMinutes += s.total_minutes;
    if (!toolNames.has(s.tool)) toolNames.set(s.tool, s.tool_name);
    if (s.longest_minutes > longestSession.minutes) {
      longestSession = { minutes: s.longest_minutes, person: s.person };
    }
  }

  for (const c of cells) {
    const p = ensurePerson(people, c.person);
    const w = weight(c);
    p.turns += w;
    p.chars += c.chars;
    p.tools.add(c.tool);
    totalTurns += w;
    inc(toolTurns, c.tool, w);
    if (!toolNames.has(c.tool)) toolNames.set(c.tool, c.tool_name);
    if (c.domain !== "other") {
      inc(domains, c.domain, w);
      let dm = toolDomain.get(c.tool);
      if (!dm) {
        dm = new Map();
        toolDomain.set(c.tool, dm);
      }
      inc(dm, c.domain, w);
    }
    inc(days, c.day, w);

    if (shape[c.shape] === undefined) shape.other += w;
    else shape[c.shape] += w;
    if (c.shape === "asking") p.askingTurns += w;
    else if (c.shape === "doing") p.doingTurns += w;

    hours[c.hour] += w;
    if (LATE_HOURS.has(c.hour)) p.lateTurns += w;
    if (EARLY_HOURS.has(c.hour)) p.earlyTurns += w;

    const named = c.delegate_tool && c.delegate_tool !== "none";
    if (named) {
      delegateTurns += w;
      delegateCells += 1;
      p.delegateTurns += w;
    }

    if (c.tool === "openclaw" && c.delegate_tool === "claude_code") p.almaClaudeCode += w;
    // Proxy for "ran Claude Code through Alma": the labeler rarely tags the
    // delegate tool, so real coding/delegation through Alma is the honest signal.
    if (c.tool === "openclaw" && (c.shape === "doing" || c.delegation !== "none")) p.almaBuild += w;
    if (c.tool === "openclaw" && c.shape === "asking") p.almaResearch += w;
  }

  const roster = [...people.values()];
  const byMinutes = [...roster].sort((a, b) => b.minutes - a.minutes || a.person.localeCompare(b.person));

  // Tools ranked by share of the team's prompts — the adoption measure the team
  // watches (how much of the week's AI use runs through Alma vs everything else).
  const toolShare = [...toolTurns.entries()]
    .map(([tool, turns]) => ({
      tool,
      name: toolNames.get(tool) ?? tool,
      turns,
      pct: totalTurns > 0 ? Math.round((turns / totalTurns) * 100) : 0,
    }))
    .sort((a, b) => b.turns - a.turns || a.tool.localeCompare(b.tool));

  // Alma's own breakdown, surfaced ALWAYS regardless of its rank — the team
  // steers by Alma adoption, so this card can't depend on Alma being top-2.
  // Includes the Claude Code that ran through Alma (those cells are tool=openclaw).
  const almaDm = toolDomain.get("openclaw") ?? new Map();
  const almaDomTotal = [...almaDm.values()].reduce((a, b) => a + b, 0);
  const alma = {
    share: totalTurns > 0 ? Math.round(((toolTurns.get("openclaw") ?? 0) / totalTurns) * 100) : 0,
    categories: [...almaDm.entries()]
      .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
      .slice(0, 5)
      .map(([key, n]) => ({ label: domainTitle(key), pct: almaDomTotal > 0 ? Math.round((n / almaDomTotal) * 100) : 0 })),
  };

  let peakHour = 0;
  for (let h = 1; h < 24; h++) if (hours[h] > hours[peakHour]) peakHour = h;

  const topDomain = topKey(domains);
  const busyDay = topKey(days);

  const domainTotal = [...domains.values()].reduce((a, b) => a + b, 0);
  const domainRank = [...domains.entries()]
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .map(([key, n]) => ({ key, label: domainTitle(key), pct: domainTotal > 0 ? Math.round((n / domainTotal) * 100) : 0 }));

  const askDo = shape.asking + shape.doing;

  const award = (key) =>
    [...roster]
      .filter((p) => p[key] > 0)
      .sort((a, b) => b[key] - a[key] || a.person.localeCompare(b.person))
      .map((p) => ({ person: p.person, value: p[key] }));

  return {
    people: roster,
    peopleCount: roster.length,
    totalMinutes,
    totalHours: Math.round(totalMinutes / 60),
    totalTurns,
    workWeeks: totalMinutes / 60 / 40,
    byMinutes,
    topPerson: byMinutes[0] ?? null,
    top5: byMinutes.slice(0, 5),
    tools: toolShare,
    toolShare,
    alma,
    topTool: toolShare[0] ?? null,
    toolCount: toolNames.size,
    shape,
    researchPct: askDo > 0 ? Math.round((shape.asking / askDo) * 100) : 0,
    doingPct: askDo > 0 ? Math.round((shape.doing / askDo) * 100) : 0,
    delegateTurns,
    delegateCells,
    hours,
    peakHour,
    peakHourLabel: hourLabel(peakHour),
    topDomain: topDomain ? { key: topDomain, label: domainLabel(topDomain) } : null,
    domainRank,
    busyDay: busyDay ? { day: busyDay, label: weekday(busyDay) } : null,
    longestSession: longestSession.person ? longestSession : null,
    almaClaudeCode: award("almaClaudeCode"),
    almaResearch: award("almaResearch"),
  };
}

function topKey(map) {
  let best = null;
  let max = -1;
  for (const [k, v] of map) if (v > max) ((max = v), (best = k));
  return best;
}

const WEEKDAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
function weekday(ymdStr) {
  return WEEKDAYS[atUtc(ymdStr).getUTCDay()];
}

// Domain ids come straight from the taxonomy; these are the human-facing,
// slightly-cheeky labels the cards show instead of snake_case.
const DOMAIN_LABELS = {
  software_engineering: "shipping code",
  data_and_analytics: "wrangling data",
  infrastructure_and_devops: "fighting infra",
  security: "locking things down",
  product_and_design: "designing stuff",
  research_and_science: "deep research",
  business_and_finance: "counting money",
  marketing_and_sales: "selling the dream",
  legal_and_compliance: "reading the fine print",
  people_and_hiring: "hiring people",
  education_and_learning: "learning things",
  health_and_medicine: "health stuff",
  personal_and_lifestyle: "living life",
  creative_and_media: "making things",
  customer_support: "helping users",
  operations_and_admin: "keeping the lights on",
};
function domainLabel(key) {
  return DOMAIN_LABELS[key] ?? key.replace(/_/g, " ");
}

// Proper category names for the ranked breakdown (the cheeky labels above are
// for the single hero card).
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

export const _internal = { mondayOf, addDays, hourLabel };
