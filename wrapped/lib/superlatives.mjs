// Persona cards: every person gets exactly one archetype, and the joke is
// always THEIR number — no generic flattery, no roast without receipts. The
// Duolingo insight still holds (nobody ranks last, everyone gets an identity),
// so each axis frames "more of this" as a personality, and the greedy pass
// keeps badges unique wherever the roster is small enough to allow it.

const int = (n) => Math.round(n).toLocaleString("en-US");
const h = (min) => Math.round(min / 60);
const pct = (x) => Math.round(x * 100);
const avgChars = (p) => p.chars / Math.max(p.turns, 1);
const activeDays = (p) => Object.keys(p.dayMinutes ?? {}).length;
const avgSessionSecs = (p) => Math.round((p.almaMinutes * 60) / Math.max(p.almaSessions, 1));

// Each axis: value ranks the claim across the team (leader scores 1.0), gate
// keeps the claim honest, line/chips are built from that person's own data.
const AXES = [
  {
    badge: "the grindmaxxer",
    value: (p) => p.minutes,
    gate: (p) => p.minutes >= 600,
    line: (p) => `${h(p.minutes)} hours with AI in seven days. grass has filed a missing person report.`,
    chips: (p) => [`${h(p.minutes)}h total`, `${int(p.sessions)} sessions`, `${activeDays(p)}/7 days`],
  },
  {
    badge: "the time traveler",
    // More engaged minutes in one day than the day has hours = parallel
    // sessions stacking, which is the whole joke.
    value: (p) => p.topDayMinutes,
    gate: (p) => p.topDayMinutes >= 1080,
    line: (p) => `${h(p.topDayMinutes)} hours of AI in a single ${p.topDayName}. that's more hours than the day has. physics said ok.`,
    chips: (p) => [`${h(p.topDayMinutes)}h in one day`, `${int(p.sessions)} sessions`],
  },
  {
    badge: "the essayist",
    value: avgChars,
    gate: (p) => p.turns > 10 && avgChars(p) > 600,
    line: (p) => `average prompt: ${int(avgChars(p))} characters. that's not a prompt, that's a newsletter. subscribe.`,
    chips: (p) => [`${int(avgChars(p))} chars/prompt`, `${int(p.turns)} prompts`],
  },
  {
    badge: "the sniper",
    value: (p) => 1 / (avgChars(p) + 1),
    gate: (p) => p.turns > 10,
    line: (p) => `${int(avgChars(p))} characters per prompt. says less, gets more. honestly scary.`,
    chips: (p) => [`${int(avgChars(p))} chars/prompt`, `${int(p.turns)} prompts`],
  },
  {
    badge: "the why-is-it-broken",
    value: (p) => p.troubleshootTurns,
    gate: (p) => p.troubleshootTurns > 30,
    line: (p) => `asked "why is this broken" ${int(p.troubleshootTurns)} times this week. at some point it's not the code, bestie.`,
    chips: (p) => [`${int(p.troubleshootTurns)} debugging cries`, `${h(p.minutes)}h total`],
  },
  {
    badge: "the yap session",
    value: (p) => p.casualTurns,
    gate: (p) => p.casualTurns > 20,
    line: (p) => `${int(p.casualTurns)} casual convos with the AI. that's not a tool anymore, that's a situationship.`,
    chips: (p) => [`${int(p.casualTurns)} just chatting`, `${int(p.sessions)} sessions`],
  },
  {
    badge: "the hit and run",
    value: (p) => p.almaSessions / (p.almaMinutes + 1),
    gate: (p) => p.almaSessions >= 30 && avgSessionSecs(p) < 120,
    line: (p) => `${int(p.almaSessions)} Alma sessions, ${avgSessionSecs(p)} seconds each on average. gets in, gets out, forms no attachments.`,
    chips: (p) => [`${int(p.almaSessions)} sessions`, `~${avgSessionSecs(p)}s each`],
  },
  {
    badge: "the 9-to-5 icon",
    value: (p) => p.minutes,
    gate: (p) => p.minutes > 1000 && p.weekendMinutes === 0,
    line: (p) => `${h(p.minutes)} hours midweek, zero on the weekend. logs off friday and becomes unverifiable lore.`,
    chips: (p) => [`${h(p.minutes)}h mon–fri`, `0h weekend`],
  },
  {
    badge: "the weekend arc",
    value: (p) => p.weekendMinutes,
    gate: (p) => p.weekendMinutes >= 600,
    line: (p) => `${h(p.weekendMinutes)} weekend hours with AI. saturday plans? compiling.`,
    chips: (p) => [`${h(p.weekendMinutes)}h weekend`, `${h(p.minutes)}h total`],
  },
  {
    badge: "the one-day wonder",
    value: (p) => p.topDayMinutes / Math.max(p.minutes, 1),
    gate: (p) => p.minutes > 300 && p.topDayMinutes / Math.max(p.minutes, 1) > 0.55,
    line: (p) => `did ${pct(p.topDayMinutes / p.minutes)}% of their entire week on one ${p.topDayName}. peaked immediately. iconic, honestly.`,
    chips: (p) => [`${pct(p.topDayMinutes / p.minutes)}% on ${p.topDayName}`, `${h(p.minutes)}h total`],
  },
  {
    badge: "the polycule",
    value: (p) => p.tools.size,
    gate: (p) => p.tools.size >= 4,
    line: (p) => `${p.tools.size} different AIs in one week. loyalty? never heard of her.`,
    chips: (p) => [`${p.tools.size} AIs`, `${h(p.minutes)}h total`],
  },
  {
    badge: "the alma ride-or-die",
    value: (p) => p.almaMinutes / Math.max(p.minutes, 1),
    gate: (p) => p.almaMinutes > 120 && p.almaMinutes / Math.max(p.minutes, 1) > 0.5,
    line: (p) => `${pct(p.almaMinutes / p.minutes)}% of their AI time was Alma. the only one who understood the assignment.`,
    chips: (p) => [`${pct(p.almaMinutes / p.minutes)}% Alma`, `${h(p.almaMinutes)}h with Alma`],
  },
  {
    badge: "the middle manager",
    value: (p) => p.drives,
    gate: (p) => p.drives >= 10,
    line: (p) => `made one AI boss another AI around ${int(p.drives)} times. hasn't done a task personally in weeks. delegation era.`,
    chips: (p) => [`${int(p.drives)} handoffs`, `${int(p.sessions)} sessions`],
  },
  {
    badge: "the marathoner",
    value: (p) => p.longest,
    gate: (p) => p.longest >= 240,
    line: (p) => `one sitting lasted ${h(p.longest)} hours. the chair is pressing charges.`,
    chips: (p) => [`${h(p.longest)}h one sitting`, `${h(p.minutes)}h total`],
  },
  {
    badge: "the ghostwriter",
    value: (p) => p.draftTurns,
    gate: (p) => p.draftTurns >= 8,
    line: (p) => `had AI write ${int(p.draftTurns)} messages for them. hasn't faced a blank text box alone in months.`,
    chips: (p) => [`${int(p.draftTurns)} drafted msgs`, `${int(p.turns)} prompts`],
  },
  {
    badge: "the setup arc",
    value: (p) => p.configTurns,
    gate: (p) => p.configTurns >= 20,
    line: (p) => `${int(p.configTurns)} prompts just configuring things. still not configured. the journey continues.`,
    chips: (p) => [`${int(p.configTurns)} setup prompts`, `${p.tools.size} AIs`],
  },
  {
    badge: "the after-midnight",
    value: (p) => p.lateTurns,
    gate: (p) => p.lateTurns > 10,
    line: (p) => `${int(p.lateTurns)} prompts after midnight. the 3 am thoughts needed a compiler.`,
    chips: (p) => [`${int(p.lateTurns)} midnight prompts`],
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
    for (const ax of AXES) {
      if (!ax.gate(p)) continue;
      const v = ax.value(p);
      if (v <= 0) continue;
      out.push({ person: p.person, badge: ax.badge, score: v / maxOf.get(ax.badge), line: ax.line(p), chips: ax.chips(p) });
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
    if (c) return { person: p.person, badge: c.badge, line: c.line, chips: c.chips };
    return { person: p.person, badge: "the wildcard", line: "impossible to label this week. the data threw its hands up.", chips: [] };
  });
}

export const _internal = { AXES, candidates };
