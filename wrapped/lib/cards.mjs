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
    sub: "one week with the robots. here's the damage.",
  });

  if (m.peopleCount === 0) {
    cards.push({
      kind: "stat",
      kicker: "nothing yet",
      heroNumber: 0,
      heroUnit: "prompts",
      sub: "quiet week so far. once devices check in, this fills itself in.",
    });
    cards.push({ kind: "summary", kicker: opts.weekLabel ?? "", heroText: team, grid: summaryGrid(m), sub: "check back once everyone's been cooking." });
    return cards;
  }

  if (m.totalMinutes > 0) {
    cards.push({
      kind: "stat",
      kicker: "time spent with AI",
      ...hours(m.totalMinutes),
      sub: "that's how long you spent talking to robots. they've forgotten already.",
    });
  }

  if (m.totalTurns > 0) {
    cards.push({
      kind: "stat",
      kicker: "prompts sent",
      heroNumber: m.totalTurns,
      heroUnit: plural(m.totalTurns, "prompt", "prompts"),
      sub: "the AIs are tired. they'd never say it, but they are.",
    });
  }

  if (m.topTool) {
    cards.push({
      kind: "stat",
      kicker: "the team's ride or die",
      heroText: m.topTool.name,
      sub: "the team's most-used AI. it knows too much about you now.",
    });
  }

  if (m.toolShare.length > 1) {
    cards.push({
      kind: "ranked",
      kicker: "which AI got used the most",
      rows: m.toolShare.slice(0, 5).map((t) => ({ label: t.name, pct: t.pct })),
      sub: "the whole AI roster, by share of the team's prompts.",
    });
  }

  for (const t of m.toolCategories) {
    cards.push({
      kind: "ranked",
      kicker: `what they ran ${t.name} for`,
      rows: t.categories.map((c) => ({ label: c.label, pct: c.pct })),
      sub: `what the team actually used ${t.name} for this week.`,
    });
  }

  if (m.domainRank.length) {
    cards.push({
      kind: "ranked",
      kicker: "what the team used AI for",
      rows: m.domainRank.slice(0, 5).map((d) => ({ label: d.label, pct: d.pct })),
      sub: "the team's whole personality this week, ranked. it tracks.",
    });
  }

  cards.push({
    kind: "stat",
    kicker: "the team's rush hour",
    heroText: m.peakHourLabel,
    sub: "the hour the team went hardest. sleep lost.",
  });

  if (m.busyDay) {
    cards.push({
      kind: "stat",
      kicker: "busiest day",
      heroText: m.busyDay.label,
      sub: "the day the whole team went feral. the servers felt it.",
    });
  }

  if (m.longestSession && m.longestSession.minutes > 0) {
    cards.push({
      kind: "stat",
      kicker: "longest single session",
      heroNumber: m.longestSession.minutes,
      heroUnit: plural(m.longestSession.minutes, "minute", "minutes"),
      sub: `${firstName(m.longestSession.person)} stayed locked in this long. not a session, a situationship.`,
    });
  }

  if (m.delegateTurns > 0) {
    cards.push({
      kind: "stat",
      kicker: "AI bossing AI",
      heroNumber: m.delegateTurns,
      heroUnit: plural(m.delegateTurns, "handoff", "handoffs"),
      sub: "times you made an AI manage another AI. middle management, automated.",
    });
  }

  if (m.topPerson && m.topPerson.minutes > 0) {
    cards.push({
      kind: "reveal",
      kicker: "the one who cooked the most",
      heroText: firstName(m.topPerson.person),
      sub: "carried the whole team on their back. someone check on them.",
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
      sub: "everyone else. honestly, the view's better down here.",
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
    trophy(
      "🏆",
      "The Alma × Claude Code Award",
      m.almaClaudeCode,
      (v) => `ran Claude Code through Alma ${int(v)} ${plural(v, "time", "times")}. the fans never stopped.`,
      "nobody ran Claude Code through Alma this week. trophy stays in the case.",
    ),
  );
  cards.push(
    trophy(
      "🔬",
      "The Alma × Research Award",
      m.almaResearch,
      (v) => `out-researched the whole team on Alma, ${int(v)} ${plural(v, "prompt", "prompts")} deep. footnotes immaculate.`,
      "zero research prompts through Alma this week. this one's unclaimed.",
    ),
  );

  cards.push({
    kind: "summary",
    kicker: opts.weekLabel ?? "",
    heroText: team,
    grid: summaryGrid(m),
    sub: "your week in one card. screenshot it. gaslight your manager.",
  });

  return cards;
}

function trophy(emoji, title, ranking, earned, emptyLine) {
  const winner = ranking[0];
  if (!winner) {
    return { kind: "trophy", emoji, title, winner: null, stat: emptyLine };
  }
  return {
    kind: "trophy",
    emoji,
    title,
    winner: firstName(winner.person),
    stat: earned(winner.value),
    runnersUp: ranking.slice(1, 3).map((r) => ({ name: firstName(r.person), value: int(r.value) })),
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
