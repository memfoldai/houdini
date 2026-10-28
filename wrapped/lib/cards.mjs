import { assignPersonas } from "./personas.mjs";

const int = (n) => Math.round(n).toLocaleString("en-US");
const plural = (n, one, many) => (n === 1 ? one : many);

function firstName(person) {
  return person.split(/[\s._-]+/)[0] || person;
}

// Below this share the cross-tool chart is left out entirely (see its use).
const ALMA_SHARE_FLOOR = 15;

const WEEKDAYS = { Mon: "Monday", Tue: "Tuesday", Wed: "Wednesday", Thu: "Thursday", Fri: "Friday", Sat: "Saturday", Sun: "Sunday" };

// The arc: the whole team's AI week first (volume, which AI, what for, when),
// then Alma in detail because that is what the team steers by, then one card
// per person. Every card states a number the rows can defend, and a card whose
// evidence is missing drops out rather than hedge.
export function buildCards(m, opts = {}) {
  const team = opts.team ?? "Houdini";
  const history = opts.history ?? null;
  const a = m.alma;
  const cards = [];

  cards.push({
    kind: "title",
    kicker: opts.weekLabel ?? "",
    heroText: team,
    sub: "your team's week with AI. every number below is counted, not guessed.",
  });

  if (m.peopleCount === 0) {
    cards.push({
      kind: "stat",
      kicker: "nothing yet",
      heroNumber: 0,
      heroUnit: "prompts",
      sub: "no devices reported this week.",
    });
    cards.push({ kind: "summary", kicker: opts.weekLabel ?? "", heroText: team, grid: summaryGrid(m), sub: "check back once the devices check in." });
    return cards;
  }

  if (m.totalTurns > 0) {
    cards.push({
      kind: "stat",
      kicker: "prompts sent",
      heroNumber: m.totalTurns,
      heroUnit: plural(m.totalTurns, "prompt", "prompts"),
      sub: `across ${m.peopleCount} ${plural(m.peopleCount, "person", "people")} and every AI on their machines.`,
    });
  }

  if (m.totalMinutes > 0) {
    cards.push({
      kind: "stat",
      kicker: "time inside the tools",
      heroNumber: m.totalHours,
      heroUnit: plural(m.totalHours, "hour", "hours"),
      sub: "measured from session transcripts, idle gaps removed.",
    });
  }

  // Alma's own requests are still under-counted upstream, so this ranking only
  // earns a slide once Alma is unambiguously a big share of the week. Below that
  // the chart would argue about a number the rows cannot yet settle.
  const almaRequests = m.requestShare.find((t) => t.tool === "openclaw");
  if (m.requestShare.length > 1 && (almaRequests?.pct ?? 0) >= ALMA_SHARE_FLOOR) {
    cards.push({
      kind: "ranked",
      kicker: "which AI got used the most",
      rows: m.requestShare.slice(0, 5).map((t) => ({ label: t.name, pct: t.pct })),
      sub: `${int(m.totalRequests)} requests across the team — every prompt typed, and every shortcut said out loud.`,
    });
  }

  if (m.domainRank.length) {
    cards.push({
      kind: "ranked",
      kicker: "what the team used AI for",
      rows: m.domainRank.slice(0, 5).map((d) => ({ label: d.label, pct: d.pct })),
      sub: "by prompt, across every tool.",
    });
  }

  if (m.totalTurns > 0) {
    cards.push({
      kind: "stat",
      kicker: "the team's busiest hour",
      heroText: m.peakHourLabel,
      sub: "more prompts land in this hour than any other, on everyone's own clock.",
    });
  }

  if (a.actions === 0) {
    cards.push({ kind: "summary", kicker: opts.weekLabel ?? "", heroText: team, grid: summaryGrid(m), sub: "the week, itemized." });
    return cards;
  }

  cards.push({
    kind: "stat",
    kicker: "shortcuts run through Alma",
    heroNumber: a.shortcutRuns,
    heroUnit: plural(a.shortcutRuns, "run", "runs"),
    sub: `${a.distinctShortcuts} different shortcuts, ${a.peopleUsing} ${plural(a.peopleUsing, "person", "people")}, seven days.`,
  });

  if (a.byDay.some((d) => d.shortcuts > 0)) {
    const peak = a.byDay.reduce((x, d) => (d.shortcuts > x.shortcuts ? d : x));
    cards.push({
      kind: "bars",
      kicker: "shortcut runs, by day",
      bars: a.byDay.map((d) => ({ label: d.label, value: d.shortcuts })),
      sub: `${WEEKDAYS[peak.label] ?? peak.label} was the peak with ${peak.shortcuts}.${coverageNote(m.coverage, a.byDay.at(-1))}`,
    });
  }

  if (a.byName.length) {
    cards.push({
      kind: "counts",
      kicker: "most-run shortcuts",
      rows: a.byName.slice(0, 8).map((s) => ({ label: s.key, value: s.runs, meta: `${s.people} ${plural(s.people, "person", "people")}` })),
      sub: "counted by runs. the small number is how many people ran it.",
    });

    const broad = a.byReach[0];
    cards.push({
      kind: "counts",
      kicker: "widest reach",
      rows: a.byReach.slice(0, 8).map((s) => ({ label: s.key, value: s.people, meta: `${s.runs} ${plural(s.runs, "run", "runs")}` })),
      sub: `${broad.key} reached ${broad.people} of ${a.peopleUsing}. reach and volume are not the same list.`,
    });
  }

  if (a.sessions > 0) {
    cards.push({
      kind: "stat",
      kicker: "Alma sessions",
      heroNumber: a.sessions,
      heroUnit: plural(a.sessions, "session", "sessions"),
      sub: `${a.minutes} minutes of clock time between them. for ${a.oneShotPeople} of ${a.peopleWithSessions} people the longest one all week was two minutes or less.`,
    });
  }

  if (a.capabilities.length) {
    cards.push({
      kind: "counts",
      kicker: "what Alma reached into",
      rows: a.capabilities.slice(0, 8).map((c) => ({ label: c.key, value: c.runs, meta: `${c.people} ${plural(c.people, "person", "people")}` })),
      sub: "connector calls by capability. one person can run up a whole family alone, so watch the small number.",
    });
  }

  if (history?.debuts?.length) {
    const n = history.debuts.length;
    cards.push({
      kind: "counts",
      kicker: "new shortcuts this week",
      rows: history.debuts.slice(0, 8).map((d) => ({ label: d.key, value: d.runs, meta: `${d.people} ${plural(d.people, "person", "people")}` })),
      sub: `${n} ${plural(n, "shortcut", "shortcuts")} ran for the first time ever${n > 8 ? ", showing the eight most-run" : ""}.`,
    });
  }

  // Dictation is a separate product (push-to-talk speech to text). Its weekly
  // aggregates ride in as a sidecar so the wrapped can show where the team is
  // actually talking to its machines, not just typing at them.
  const d = opts.dictation;
  if (d && d.words > 0) {
    cards.push({
      kind: "stat",
      kicker: "words spoken to the machine",
      heroNumber: d.words,
      heroUnit: "words dictated",
      sub: `about ${d.speechMinutes} minutes of talking, at${d.wpm ? ` roughly ${d.wpm} words a minute` : " a steady clip"}. the team would rather say it than type it.`,
    });
    if (d.apps?.length) {
      cards.push({
        kind: "counts",
        kicker: "where the team dictates",
        rows: d.apps.slice(0, 8).map((x) => ({ label: x.label, value: x.value, meta: "words" })),
        sub: "which apps the spoken words landed in. notes and Claude do most of the listening.",
      });
    }
    if (d.lengths?.length) {
      cards.push({
        kind: "bars",
        kicker: "how much they say per go",
        bars: d.lengths.map((x) => ({ label: x.label, value: x.value })),
        sub: "one dictation, sized from a quick quip to a full rant. most land around a single line.",
      });
    }
    if (d.declinedNoField > 0) {
      cards.push({
        kind: "stat",
        kicker: "talking to the void",
        heroNumber: d.declinedNoField,
        heroUnit: "times",
        sub: "someone hit dictate with no text box in focus, so the words went nowhere. the void is a bad listener.",
      });
    }
  }

  // A week can ship hand-written personas (someone read the data and wrote the
  // cards); they replace the rule-driven set wholesale rather than mixing.
  const personas = opts.personas?.length ? opts.personas : assignPersonas(m.people);
  for (const p of personas) {
    cards.push({ kind: "person", personName: firstName(p.person), badge: p.badge, line: p.line });
  }

  cards.push({
    kind: "summary",
    kicker: opts.weekLabel ?? "",
    heroText: team,
    grid: summaryGrid(m),
    sub: "the week, itemized.",
  });

  return cards;
}

// A device uploads its whole export in one push, so a person with no rows on
// the closing days either did nothing or stopped checking in. The rows cannot
// tell those apart, so the chart says so instead of implying a quiet weekend.
function coverageNote(coverage, lastDay) {
  if (!coverage?.quiet?.length) return "";
  const n = coverage.quiet.length;
  const day = WEEKDAYS[lastDay?.label] ?? "the last day";
  return ` ${n} of ${coverage.total} ${plural(coverage.total, "device", "devices")} last checked in before ${day}, so the closing days are a floor, not a total.`;
}

function summaryGrid(m) {
  const a = m.alma;
  const grid = [
    { label: "Prompts", value: int(m.totalTurns) },
    { label: "AI time", value: `${int(m.totalHours)}h` },
    { label: "People", value: int(m.peopleCount) },
  ];
  if (m.topTool) grid.push({ label: "Top AI", value: m.topTool.name });
  if (a.shortcutRuns > 0) grid.push({ label: "Shortcut runs", value: int(a.shortcutRuns) });
  if (a.connectorRuns > 0) grid.push({ label: "Connector calls", value: int(a.connectorRuns) });
  return grid;
}
