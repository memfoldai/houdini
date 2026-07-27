import { assignSuperlatives } from "./superlatives.mjs";

const int = (n) => Math.round(n).toLocaleString("en-US");
const plural = (n, one, many) => (n === 1 ? one : many);

function hours(minutes) {
  if (minutes >= 60) return { heroNumber: Math.round(minutes / 60), heroUnit: plural(Math.round(minutes / 60), "hour", "hours") };
  return { heroNumber: minutes, heroUnit: plural(minutes, "minute", "minutes") };
}

function firstName(person) {
  return person.split(/[\s._-]+/)[0] || person;
}

// Fixed narrative order (the research's canonical arc): a general, Spotify-style
// team recap first — volume, tools, vibes, the reveal, the podium, everyone's
// superlative — with the two Alma trophies saved as the special finale. Cards
// with no data drop out so a quiet week never renders an empty block. Copy is
// second-person, short, and casual on purpose; the number is the joke.
export function buildCards(m, opts = {}) {
  const team = opts.team ?? "Houdini";
  const cards = [];

  cards.push({
    kind: "title",
    kicker: opts.weekLabel ?? "",
    heroText: team,
    sub: "your team's week with AI. every last prompt of it.",
  });

  if (m.peopleCount === 0) {
    cards.push({
      kind: "stat",
      kicker: "nothing yet",
      heroNumber: 0,
      heroUnit: "prompts",
      sub: "quiet week so far. once everyone's devices check in, this fills up.",
    });
    cards.push({ kind: "summary", kicker: opts.weekLabel ?? "", heroText: team, grid: summaryGrid(m), sub: "check back once everyone's been cooking." });
    return cards;
  }

  if (m.totalMinutes > 0) {
    cards.push({
      kind: "stat",
      kicker: "time spent with AI",
      ...hours(m.totalMinutes),
      sub: "as a team. that's a part-time job nobody's getting paid for.",
    });
  }

  if (m.totalTurns > 0) {
    cards.push({
      kind: "stat",
      kicker: "prompts sent",
      heroNumber: m.totalTurns,
      heroUnit: plural(m.totalTurns, "prompt", "prompts"),
      sub: "quality was optional this week, apparently.",
    });
  }

  if (m.topTool) {
    cards.push({
      kind: "stat",
      kicker: "the team's ride or die",
      heroText: m.topTool.name,
      sub: "the team's go-to. opened more than the fridge.",
    });
  }

  if (m.toolShare.length > 1) {
    cards.push({
      kind: "ranked",
      kicker: "which AI got used the most",
      rows: m.toolShare.slice(0, 5).map((t) => ({ label: t.name, pct: t.pct })),
      sub: "how the team's prompts split across every AI.",
    });
  }

  if (m.alma.categories.length) {
    cards.push({
      kind: "ranked",
      kicker: "what the team ran Alma for",
      rows: m.alma.categories.map((c) => ({ label: c.label, pct: c.pct })),
      sub: `Alma got ${m.alma.share}% of the team's prompts. here's where they went.`,
    });
  } else {
    cards.push({
      kind: "stat",
      kicker: "the Alma report",
      heroNumber: m.alma.share,
      heroUnit: "% of prompts",
      sub: "Alma barely got a look this week. it's right there, people.",
    });
  }

  if (m.domainRank.length) {
    cards.push({
      kind: "ranked",
      kicker: "what the team used AI for",
      rows: m.domainRank.slice(0, 5).map((d) => ({ label: d.label, pct: d.pct })),
      sub: "what the team was actually working on. mostly.",
    });
  }

  cards.push({
    kind: "stat",
    kicker: "the team's rush hour",
    heroText: m.peakHourLabel,
    sub: "peak grind. focus blocks never stood a chance.",
  });

  if (m.busyDay) {
    cards.push({
      kind: "stat",
      kicker: "busiest day",
      heroText: m.busyDay.label,
      sub: "carried the whole week on its back.",
    });
  }

  if (m.longestSession && m.longestSession.minutes > 0) {
    cards.push({
      kind: "stat",
      kicker: "longest single session",
      heroNumber: m.longestSession.minutes,
      heroUnit: plural(m.longestSession.minutes, "minute", "minutes"),
      sub: `${firstName(m.longestSession.person)} went this long without stopping. that's a hostage situation.`,
    });
  }

  if (m.topPerson && m.topPerson.minutes > 0) {
    cards.push({
      kind: "reveal",
      kicker: "the one who cooked the most",
      heroText: firstName(m.topPerson.person),
      sub: "out-prompted the entire team. and it wasn't close.",
    });
  }

  if (m.top5.length > 1) {
    cards.push({
      kind: "podium",
      kicker: "top 5, by AI hours",
      rows: m.top5.map((p, i) => ({
        rank: i + 1,
        name: firstName(p.person),
        value: Math.round(p.minutes / 60) || 0,
        unit: "h",
      })),
      sub: "the top 5. everyone below can finally relax.",
    });
  }

  for (const s of assignSuperlatives(m.people)) {
    cards.push({
      kind: "person",
      kicker: "certified",
      personName: firstName(s.person),
      badge: s.badge,
      line: s.line,
    });
  }

  cards.push(
    pickTrophy(
      "🏆",
      "The Alma × Claude Code Award",
      m.people,
      // Alma driving Claude Code: Claude Code sessions by someone who also uses
      // Alma (so the CC is plausibly Alma-run), plus a boost for Alma sessions
      // that actually spawned an agent. Session-based, so a heavy user whose
      // cells aren't labeled yet still counts. Direct-only CC users (no Alma)
      // score zero — they're not driving it through Alma.
      (p) => (p.almaSessions > 0 ? p.ccSessions : 0) + p.almaAgentRun * 3 + p.almaClaudeCode * 3,
      (v) => `ran Claude Code through Alma ${int(v)} ${plural(v, "time", "times")}. no notes.`,
      "nobody drove Claude Code through Alma this week. the goal awaits.",
    ),
  );
  cards.push(
    pickTrophy(
      "🔬",
      "The Alma × Research Award",
      m.people,
      (p) => p.almaResearch,
      (v) => `leaned on Alma to dig into things ${int(v)} ${plural(v, "time", "times")}. the resident researcher.`,
      "nobody used Alma to research this week. the library's empty.",
    ),
  );

  cards.push({
    kind: "summary",
    kicker: opts.weekLabel ?? "",
    heroText: team,
    grid: summaryGrid(m),
    sub: "the whole week, one card. screenshot it before someone edits it.",
  });

  return cards;
}

// Picks the winner + top 3 by a jittered weighted sort: each person's real
// score is scaled by a random factor per render, so the most active tend to win
// but the pick genuinely shuffles every time the wrapped is regenerated. The
// numbers shown are the real scores. Everyone with any relevant activity is
// eligible, so the award is never left empty when the metric is populated.
function pickTrophy(emoji, title, people, scoreFn, earned, emptyLine) {
  const pool = people
    .map((p) => ({ name: firstName(p.person), score: scoreFn(p) }))
    .filter((x) => x.score > 0);
  if (!pool.length) {
    return { kind: "trophy", emoji, title, winner: null, stat: emptyLine };
  }
  const ranked = pool
    .map((x) => ({ ...x, jitter: Math.pow(x.score, 0.3) * (0.4 + Math.random()) }))
    .sort((a, b) => b.jitter - a.jitter)
    .slice(0, 3);
  const winner = ranked[0];
  // Runners-up are names only: the winner is a weighted-random pick, so a strict
  // 2nd/3rd by number would look wrong when a lighter user wins. Names keep it a
  // fun "podium" without implying an exact ranking.
  return {
    kind: "trophy",
    emoji,
    title,
    winner: winner.name,
    stat: earned(winner.score),
    runnersUp: ranked.slice(1).map((r) => ({ name: r.name })),
  };
}

function summaryGrid(m) {
  const grid = [
    { label: "AI time", value: `${m.totalHours}h` },
    { label: "Prompts", value: int(m.totalTurns) },
    { label: "People", value: int(m.peopleCount) },
  ];
  if (m.topTool) grid.push({ label: "Top tool", value: m.topTool.name });
  if (m.topPerson) grid.push({ label: "MVP", value: firstName(m.topPerson.person) });
  grid.push({ label: "Peak", value: m.peakHourLabel });
  return grid;
}
