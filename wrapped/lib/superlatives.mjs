// The Duolingo insight the research surfaced: an archetype card lifts sharing
// for *everyone*, not just the leader, because nobody is ranked last — each
// person is handed a flattering identity instead. So every axis here is
// "more is a personality", never a deficiency, and the assignment guarantees a
// distinct badge per person wherever the roster is small enough to allow it.

const int = (n) => Math.round(n).toLocaleString("en-US");
const pct = (x) => Math.round(x * 100);

// Each axis scores a person on one behaviour. `value` is compared across the
// team; the leader on an axis scores 1.0, which is what lets pass 1 hand each
// axis to the person who most embodies it.
const avgChars = (p) => p.chars / Math.max(p.turns, 1);

const AXES = [
  {
    badge: "The Yapper",
    value: (p) => p.askingTurns / Math.max(p.turns, 1),
    gate: (p) => p.askingTurns > 0,
    line: (p) => `asked more than they told. ${pct(p.askingTurns / Math.max(p.turns, 1))}% of their prompts were questions.`,
  },
  {
    badge: "The Shipper",
    value: (p) => p.doingTurns / Math.max(p.turns, 1),
    gate: (p) => p.doingTurns > 0,
    line: (p) => `${pct(p.doingTurns / Math.max(p.turns, 1))}% of their prompts were just "build this". zero small talk.`,
  },
  {
    badge: "The Puppet Master",
    value: (p) => p.delegateTurns / Math.max(p.turns, 1),
    gate: (p) => p.delegateTurns > 0,
    line: (p) => `had one AI run another ${int(p.delegateTurns)} times. never lifted a finger.`,
  },
  {
    badge: "The Night Owl",
    value: (p) => p.lateTurns / Math.max(p.turns, 1),
    gate: (p) => p.lateTurns > 0,
    line: () => `the last one online. every single night.`,
  },
  {
    badge: "The Early Bird",
    value: (p) => p.earlyTurns / Math.max(p.turns, 1),
    gate: (p) => p.earlyTurns > 0,
    line: () => `first one online. beat the coffee, every day.`,
  },
  {
    badge: "The Gremlin",
    value: (p) => p.longest,
    gate: (p) => p.longest > 0,
    line: (p) => `${int(p.longest)} minutes in one sitting. someone bring snacks.`,
  },
  {
    badge: "The Commitment-phobe",
    value: (p) => p.tools.size,
    gate: (p) => p.tools.size > 1,
    line: (p) => `used ${int(p.tools.size)} different AIs. can't pick a favorite child.`,
  },
  {
    badge: "The Overexplainer",
    value: avgChars,
    gate: (p) => p.turns > 2,
    line: (p) => `${int(avgChars(p))} characters a prompt. wrote a paragraph to say "fix this".`,
  },
  {
    badge: "The Sniper",
    value: (p) => 1 / (avgChars(p) + 1),
    gate: (p) => p.turns > 2,
    line: (p) => `gets it done in ${int(avgChars(p))} characters. not a word wasted.`,
  },
  {
    badge: "The Machine",
    value: (p) => p.turns,
    gate: (p) => p.turns > 0,
    line: (p) => `${int(p.turns)} prompts this week. genuinely, are they okay?`,
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
    for (const ax of AXES) {
      if (!ax.gate(p)) continue;
      const v = ax.value(p);
      if (v <= 0) continue;
      out.push({ person: p.person, badge: ax.badge, score: v / maxOf.get(ax.badge), line: ax.line(p) });
    }
  }
  // Strongest claim first; name breaks ties so the assignment is deterministic.
  out.sort((a, b) => b.score - a.score || a.person.localeCompare(b.person) || a.badge.localeCompare(b.badge));
  return out;
}

// Greedy in three passes: (1) give each axis to the person who leads it, one
// badge each; (2) let anyone still without a badge take their strongest axis
// even if that badge repeats (teams bigger than the axis list); (3) a person
// with no qualifying axis at all still gets named.
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
    if (c) return { person: p.person, badge: c.badge, line: c.line };
    return { person: p.person, badge: "The Wildcard", line: "impossible to label this week. we genuinely tried." };
  });
}

export const _internal = { AXES, candidates };
