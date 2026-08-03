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
    badge: "the grindmaxxer",
    value: (p) => p.minutes,
    gate: (p) => p.minutes >= 600,
    line: (p) => `${h(p.minutes)} hours with AI in seven days. grass has filed a missing person report.`,
    chips: (p) => [`${h(p.minutes)}h total`, `${int(p.sessions)} sessions`, `${activeDays(p)}/7 days`],
  },
  {
    badge: "the essayist",
    value: avgChars,
    gate: (p) => p.turns > 10 && avgChars(p) > 600,
    line: (p) => `average prompt: ${int(avgChars(p))} characters. that's not a prompt, that's a newsletter. subscribe.`,
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
    badge: "the situationship",
    value: (p) => p.casualTurns,
    gate: (p) => p.casualTurns > 20,
    line: (p) => `${int(p.casualTurns)} casual convos with the AI this week. that's not a tool anymore. exclusive? unclear.`,
    chips: (p) => [`${int(p.casualTurns)} just chatting`, `${int(p.sessions)} sessions`],
  },
  {
    badge: "the hit and run",
    value: (p) => p.almaSessions / (p.almaMinutes + 1),
    gate: (p) => p.almaSessions >= 60 && almaSecs(p) < 120,
    line: (p) => `${int(p.almaSessions)} Alma sessions, ${almaSecs(p)} seconds each on average. gets in, gets out, forms no attachments.`,
    chips: (p) => [`${int(p.almaSessions)} sessions`, `~${almaSecs(p)}s each`],
  },
  {
    badge: "the alma ride-or-die",
    value: (p) => p.almaMinutes / Math.max(p.minutes, 1),
    gate: (p) => p.almaMinutes > 120 && p.almaMinutes / Math.max(p.minutes, 1) > 0.5,
    line: (p) => `${pct(p.almaMinutes / p.minutes)}% of their AI time was Alma. the only one who understood the assignment.`,
    chips: (p) => [`${pct(p.almaMinutes / p.minutes)}% Alma`, `${h(p.almaMinutes)}h with Alma`],
  },
  {
    badge: "the after-midnight",
    value: (p) => p.lateTurns,
    gate: (p) => p.lateTurns > 10,
    line: (p) => `${int(p.lateTurns)} prompts after midnight. the 3 am thoughts needed somewhere to go.`,
    chips: (p) => [`${int(p.lateTurns)} midnight prompts`, `${int(p.turns)} total`],
  },
  {
    badge: "the ghostwriter",
    value: (p) => p.draftTurns,
    gate: (p) => p.draftTurns >= 8,
    line: (p) => `had AI write ${int(p.draftTurns)} messages for them. hasn't faced a blank text box alone in months.`,
    chips: (p) => [`${int(p.draftTurns)} drafted msgs`, `${int(p.turns)} prompts`],
  },
  {
    badge: "the middle manager",
    value: (p) => p.drives,
    gate: (p) => p.drives >= 10,
    line: (p) => `had one AI boss another AI around ${int(p.drives)} times. hasn't done a task personally in weeks. delegation era.`,
    chips: (p) => [`${int(p.drives)} handoffs`, `${int(p.sessions)} sessions`],
  },
  {
    badge: "the monk mode",
    // Engaged Claude Code session length — long, silent, locked-in blocks.
    value: (p) => p.ccMinutes / Math.max(p.ccSessions, 1),
    gate: (p) => p.ccSessions >= 5 && p.ccMinutes / Math.max(p.ccSessions, 1) >= 60,
    line: (p) => `${int(p.ccSessions)} Claude Code sessions averaging ${Math.round(p.ccMinutes / p.ccSessions)} minutes each. locked in. do not perceive them.`,
    chips: (p) => [`~${Math.round(p.ccMinutes / p.ccSessions)}m per session`, `${h(p.ccMinutes)}h in Claude Code`],
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
    line: (p) => `${int(p.sessions)} visits, about ${Math.max(1, Math.round(p.minutes / p.sessions))} minute${Math.round(p.minutes / p.sessions) === 1 ? "" : "s"} each. gets what they came for and leaves.`,
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
