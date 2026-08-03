// Persona cards: every person gets exactly one archetype and the joke is
// always THEIR verified number. Axes only use skew-free facts — totals,
// per-tool splits, engaged session lengths, labeled intents, prompt length,
// hour histograms, deterministic drives. Day-of-week facts are banned here:
// span minutes land on a session's START day, so weekend/one-day claims lie
// for anyone with long sessions (a Friday-night session swallows Saturday).
// Array order is the tie-break priority when one person leads several axes,
// so the funniest claim wins, not the alphabet.

const int = (n) => Math.round(n).toLocaleString("en-US");
const h = (min) => Math.round(min / 60);
const pct = (x) => Math.round(x * 100);
const avgChars = (p) => p.chars / Math.max(p.turns, 1);
const activeDays = (p) => Object.keys(p.dayMinutes ?? {}).length;
const almaSecs = (p) => Math.round((p.almaMinutes * 60) / Math.max(p.almaSessions, 1));

const AXES = [
  {
    badge: "cooked",
    value: (p) => p.minutes,
    gate: (p) => p.minutes >= 600,
    line: (p) => `${h(p.minutes)} hours with AI in seven days. that's ${Math.round(p.minutes / 60 / 40)} full-time jobs. the sleep schedule is chopped.`,
    chips: (p) => [`${h(p.minutes)}h total`, `${int(p.sessions)} sessions`, `${activeDays(p)}/7 days`],
  },
  {
    badge: "the lore dropper",
    value: avgChars,
    gate: (p) => p.turns > 10 && avgChars(p) > 600,
    line: (p) => `${int(avgChars(p))} characters per prompt. that's not a prompt, that's lore. the AI needs a recap episode.`,
    chips: (p) => [`${int(avgChars(p))} chars/prompt`, `${int(p.turns)} prompts`],
  },
  {
    badge: "the crashout arc",
    // troubleshootTurns is the labeler's count of diagnostic prompts — say
    // exactly that; never claim a literal quote count (people fact-check).
    value: (p) => p.troubleshootTurns,
    gate: (p) => p.troubleshootTurns > 30,
    line: (p) => `${int(p.troubleshootTurns)} of their prompts were pure debugging. the bugs are living in there rent free.`,
    chips: (p) => [`${int(p.troubleshootTurns)} debugging prompts`, `${int(p.turns)} total analyzed`],
  },
  {
    badge: "the situationship",
    value: (p) => p.casualTurns,
    gate: (p) => p.casualTurns > 20,
    line: (p) => `${int(p.casualTurns)} casual convos with the AI this week. that's not a tool anymore. hard launch when?`,
    chips: (p) => [`${int(p.casualTurns)} just chatting`, `${int(p.sessions)} sessions`],
  },
  {
    badge: "the hit and run",
    value: (p) => p.almaSessions / (p.almaMinutes + 1),
    gate: (p) => p.almaSessions >= 60 && almaSecs(p) < 120,
    line: (p) => `${int(p.almaSessions)} Alma sessions, ${almaSecs(p)} seconds each on average. in and out like it owes them money.`,
    chips: (p) => [`${int(p.almaSessions)} sessions`, `~${almaSecs(p)}s each`],
  },
  {
    badge: "alma's day one",
    value: (p) => p.almaMinutes / Math.max(p.minutes, 1),
    gate: (p) => p.almaMinutes > 120 && p.almaMinutes / Math.max(p.minutes, 1) > 0.5,
    line: (p) => `${pct(p.almaMinutes / p.minutes)}% of their AI time was Alma. no side quests. main quest only.`,
    chips: (p) => [`${pct(p.almaMinutes / p.minutes)}% Alma`, `${h(p.almaMinutes)}h with Alma`],
  },
  {
    badge: "3am coded",
    value: (p) => p.lateTurns,
    gate: (p) => p.lateTurns > 10,
    line: (p) => `${int(p.lateTurns)} prompts past midnight. sleep schedule in shambles. the thoughts win every night.`,
    chips: (p) => [`${int(p.lateTurns)} midnight prompts`, `${int(p.turns)} total`],
  },
  {
    badge: "the ghostwriter",
    value: (p) => p.draftTurns,
    gate: (p) => p.draftTurns >= 8,
    line: (p) => `${int(p.draftTurns)} messages ghostwritten by AI. hasn't typed "hope this finds you well" by hand since 2024.`,
    chips: (p) => [`${int(p.draftTurns)} drafted msgs`, `${int(p.turns)} prompts`],
  },
  {
    badge: "in the trenches",
    // Engaged Claude Code session length — long, silent, focused blocks.
    value: (p) => p.ccMinutes / Math.max(p.ccSessions, 1),
    gate: (p) => p.ccSessions >= 5 && p.ccMinutes / Math.max(p.ccSessions, 1) >= 60,
    line: (p) => `${int(p.ccSessions)} Claude Code sessions, ${Math.round(p.ccMinutes / p.ccSessions)} minutes each. permanently in the trenches. send rations.`,
    chips: (p) => [`~${Math.round(p.ccMinutes / p.ccSessions)}m per session`, `${h(p.ccMinutes)}h in Claude Code`],
  },
  {
    badge: "the middle manager",
    // Gate high enough that only serious delegators qualify; drives are
    // deterministic transcript tool-calls, so the count itself is unarguable.
    value: (p) => p.drives,
    gate: (p) => p.drives >= 20,
    line: (p) => `had one AI boss another AI around ${int(p.drives)} times this week. delegation era.`,
    chips: (p) => [`${int(p.drives)} handoffs`, `${int(p.sessions)} sessions`],
  },
  {
    badge: "the setup arc",
    value: (p) => p.configTurns,
    gate: (p) => p.configTurns >= 20,
    line: (p) => `${int(p.configTurns)} prompts just configuring things. still not configured. the journey continues.`,
    chips: (p) => [`${int(p.configTurns)} setup prompts`, `${p.tools.size} AIs`],
  },
  {
    badge: "the polycule",
    value: (p) => p.tools.size,
    gate: (p) => p.tools.size >= 4,
    line: (p) => `${p.tools.size} different AIs in one week. loyalty? never heard of her.`,
    chips: (p) => [`${p.tools.size} AIs`, `${h(p.minutes)}h total`],
  },
  {
    badge: "the drive-by",
    value: (p) => p.sessions / (p.minutes + 1),
    gate: (p) => p.minutes > 0 && p.minutes < 180 && p.sessions >= 10,
    line: (p) => `${int(p.sessions)} visits, about ${Math.max(1, Math.round(p.minutes / p.sessions))} minute${Math.round(p.minutes / p.sessions) === 1 ? "" : "s"} each. gets the answer and dips.`,
    chips: (p) => [`${int(p.sessions)} visits`, `~${Math.max(1, Math.round(p.minutes / p.sessions))}m each`],
  },
  {
    badge: "the ghost",
    value: (p) => 1 / (p.minutes + 1),
    gate: (p) => p.minutes < 150,
    line: (p) => `${int(p.minutes)} minutes all week. left the entire AI industry on read.`,
    chips: (p) => [`${int(p.minutes)} min total`, `${activeDays(p)} day${activeDays(p) === 1 ? "" : "s"} seen`],
  },
];

function candidates(people) {
  const maxOf = new Map();
  for (const ax of AXES) {
    let max = 0;
    for (const p of people) if (ax.gate(p)) max = Math.max(max, ax.value(p));
    maxOf.set(ax.badge, max || 1);
  }
  const out = [];
  for (const p of people) {
    AXES.forEach((ax, idx) => {
      if (!ax.gate(p)) return;
      const v = ax.value(p);
      if (v <= 0) return;
      out.push({ person: p.person, badge: ax.badge, idx, score: v / maxOf.get(ax.badge), line: ax.line(p), chips: ax.chips(p) });
    });
  }
  // Strongest claim first; the axes' own order breaks ties so the funniest
  // claim (as curated above) wins over the alphabet; person name last for
  // full determinism.
  out.sort((a, b) => b.score - a.score || a.idx - b.idx || a.person.localeCompare(b.person));
  return out;
}

// Greedy in three passes: (1) hand each axis to its strongest unclaimed
// person, one badge each; (2) anyone still without a badge takes their
// strongest axis even if it repeats (teams bigger than the axis list);
// (3) a person with no qualifying axis at all still gets named.
export function assignSuperlatives(people) {
  const cands = candidates(people);
  const byPerson = new Map();
  const usedBadge = new Set();

  for (const c of cands) {
    if (byPerson.has(c.person) || usedBadge.has(c.badge)) continue;
    byPerson.set(c.person, c);
    usedBadge.add(c.badge);
  }
  for (const c of cands) {
    if (byPerson.has(c.person)) continue;
    byPerson.set(c.person, c);
  }
  return people.map((p) => {
    const c = byPerson.get(p.person);
    if (c) return { person: p.person, badge: c.badge, line: c.line, chips: c.chips };
    return { person: p.person, badge: "the cryptid", line: "impossible to label this week. the data threw up its hands.", chips: [] };
  });
}

export const _internal = { AXES, candidates };
