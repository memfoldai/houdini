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
    sub: "your team's week with AI. the receipts are in.",
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
    const jobs = Math.round(m.workWeeks);
    cards.push({
      kind: "stat",
      kicker: "time spent with AI",
      ...hours(m.totalMinutes),
      sub: jobs >= 2 ? `as a team. that's ${int(jobs)} entire work weeks in seven days. be so fr.` : "as a team. respectable. suspicious, but respectable.",
    });
  }

  if (m.totalTurns > 0) {
    cards.push({
      kind: "stat",
      kicker: "prompts sent",
      heroNumber: m.totalTurns,
      heroUnit: plural(m.totalTurns, "prompt", "prompts"),
      sub: "all fired at the clankers. the group chat could never.",
    });
  }

  if (m.topTool) {
    cards.push({
      kind: "stat",
      kicker: "the team's ride or die",
      heroText: m.topTool.name,
      sub: "opened more than the fridge. and this team opens the fridge a lot.",
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
    // When the peak genuinely lands in the six o'clock hour, the 6-7 meme
    // writes itself; any other hour gets the plain line.
    sub: m.peakHourLabel.startsWith("6:30")
      ? "the six-seven window. we don't make the rules."
      : "the collective lock-in hour. calendars fear it.",
  });

  if (m.busyDay) {
    cards.push({
      kind: "stat",
      kicker: "busiest day",
      heroText: m.busyDay.label,
      sub: "carried the week like a group project with one working member.",
    });
  }

  const board = m.byMinutes.filter((p) => p.minutes > 0);
  if (board.length > 1) {
    cards.push({
      kind: "podium",
      kicker: "the whole team, by AI hours",
      rows: board.map((p, i) => ({
        rank: i + 1,
        name: firstName(p.person),
        value: Math.round(p.minutes / 60) || 0,
        unit: "h",
      })),
      sub: "the full leaderboard. no hiding. we all saw it.",
    });
  }

  for (const s of assignSuperlatives(m.people)) {
    cards.push({
      kind: "person",
      personName: firstName(s.person),
      badge: s.badge,
      line: s.line,
      chips: s.chips,
    });
  }

  cards.push({
    kind: "summary",
    kicker: opts.weekLabel ?? "",
    heroText: team,
    grid: summaryGrid(m),
    sub: "the week, itemized. screenshot it before anyone starts denying things.",
  });

  return cards;
}

function summaryGrid(m) {
  const grid = [
    { label: "AI time", value: `${m.totalHours}h` },
    { label: "Prompts", value: int(m.totalTurns) },
    { label: "People", value: int(m.peopleCount) },
  ];
  if (m.topTool) grid.push({ label: "Top tool", value: m.topTool.name });
  grid.push({ label: "Peak", value: m.peakHourLabel });
  return grid;
}
