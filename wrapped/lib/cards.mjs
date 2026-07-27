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
    sub: "buckle up. here's what the team did to the AIs this week.",
  });

  if (m.peopleCount === 0) {
    cards.push({
      kind: "stat",
      kicker: "nothing yet",
      heroNumber: 0,
      heroUnit: "prompts",
      sub: "no data landed for this week yet. once the team's devices check in, this fills itself in.",
    });
    cards.push({ kind: "summary", kicker: opts.weekLabel ?? "", heroText: team, grid: summaryGrid(m), sub: "check back once everyone's been cooking." });
    return cards;
  }

  if (m.totalMinutes > 0) {
    cards.push({
      kind: "stat",
      kicker: "time spent with AI",
      ...hours(m.totalMinutes),
      sub: "that's how long y'all spent yapping at robots. touch grass maybe.",
    });
  }

  if (m.totalTurns > 0) {
    cards.push({
      kind: "stat",
      kicker: "prompts sent",
      heroNumber: m.totalTurns,
      heroUnit: plural(m.totalTurns, "prompt", "prompts"),
      sub: "the AIs are tired. they won't say it but they are.",
    });
  }

  if (m.topTool) {
    cards.push({
      kind: "stat",
      kicker: "the team's ride or die",
      heroText: m.topTool.name,
      sub: "used more than anything else. it knows too much about y'all now.",
    });
  }

  if (m.toolCount > 1) {
    cards.push({
      kind: "stat",
      kicker: "AI tools touched",
      heroNumber: m.toolCount,
      heroUnit: plural(m.toolCount, "tool", "tools"),
      sub: "commitment issues? nah. we call it keeping options open.",
    });
  }

  if (m.topDomain) {
    cards.push({
      kind: "stat",
      kicker: "what y'all were locked in on",
      heroText: m.topDomain.label,
      sub: "the team was deep in it this week. no notes.",
    });
  }

  if (m.shape.asking + m.shape.doing > 0) {
    cards.push({
      kind: "split",
      kicker: "research vs. doing",
      blocks: [
        { label: "Research", pct: m.researchPct, tone: "a" },
        { label: "Doing", pct: m.doingPct, tone: "b" },
      ],
      sub:
        m.researchPct >= m.doingPct
          ? "more reading than shipping. the team really said lemme double-check."
          : "less reading, more shipping. reckless behavior. we stan.",
    });
  }

  cards.push({
    kind: "stat",
    kicker: "the team's rush hour",
    heroText: m.peakHourLabel,
    sub: "peak grind time. sleep is a scam apparently.",
  });

  if (m.busyDay) {
    cards.push({
      kind: "stat",
      kicker: "busiest day",
      heroText: m.busyDay.label,
      sub: "the day the whole team went feral. iykyk.",
    });
  }

  if (m.longestSession && m.longestSession.minutes > 0) {
    cards.push({
      kind: "stat",
      kicker: "longest single session",
      heroNumber: m.longestSession.minutes,
      heroUnit: plural(m.longestSession.minutes, "minute", "minutes"),
      sub: `${firstName(m.longestSession.person)} locked in this long straight. that's not a chat, that's a situationship.`,
    });
  }

  if (m.delegateTurns > 0) {
    cards.push({
      kind: "stat",
      kicker: "AI bossing AI",
      heroNumber: m.delegateTurns,
      heroUnit: plural(m.delegateTurns, "handoff", "handoffs"),
      sub: "times y'all made an AI go manage another AI. delegation king behavior.",
    });
  }

  if (m.topPerson && m.topPerson.minutes > 0) {
    cards.push({
      kind: "reveal",
      kicker: "the one who cooked the most",
      heroText: firstName(m.topPerson.person),
      sub: "carried the whole team on their back. log off. please. we're begging.",
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
      sub: "everyone else you're valid too. it's better down here anyway.",
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
      (v) => `ran Claude Code through Alma ${int(v)} ${plural(v, "time", "times")}. absolute machine.`,
      "nobody ran Claude Code through Alma this week. the trophy stays in the case.",
    ),
  );
  cards.push(
    trophy(
      "🔬",
      "The Alma × Research Award",
      m.almaResearch,
      (v) => `out-researched the whole team on Alma — ${int(v)} ${plural(v, "prompt", "prompts")} deep. the footnotes are immaculate.`,
      "not one research prompt through Alma this week. unclaimed.",
    ),
  );

  cards.push({
    kind: "summary",
    kicker: opts.weekLabel ?? "",
    heroText: team,
    grid: summaryGrid(m),
    sub: "your week, one card. screenshot it. post it. gaslight your manager.",
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
    runnersUp: ranking.slice(1, 4).map((r) => ({ name: firstName(r.person), value: r.value })),
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
