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
    trophy(
      "🏆",
      "The Alma × Claude Code Award",
      m.almaClaudeCode,
      (v) => `ran Claude Code through Alma ${int(v)} ${plural(v, "time", "times")}. never did it the hard way.`,
      "nobody ran Claude Code through Alma this week. it's right there, people.",
    ),
  );
  cards.push(
    trophy(
      "🔬",
      "The Alma × Research Award",
      m.almaResearch,
      (v) => `leaned on Alma to dig into things ${int(v)} ${plural(v, "prompt", "prompts")} deep. the team's resident researcher.`,
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
