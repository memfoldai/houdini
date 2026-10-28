const int = (n) => Math.round(n).toLocaleString("en-US");
const plural = (n, one, many) => (n === 1 ? one : many);

// Every persona is ONE measured fact, said in plain English. The rule still
// gates on the count — nothing here fires unless the data holds — but the card
// describes the habit instead of reciting the metric, because a number the
// reader has already seen four slides ago is not a character. A rule
// claiming a superlative resolves its single true holder across the whole
// roster and is skipped outright when that person already has a card — handing
// "the most shortcuts" to the runner-up would print a lie. Ties are skipped for
// the same reason. Whoever the rules don't reach gets their own largest number
// instead of an invented trait.

// Floors that keep a card from being technically true but pointless: one call
// to an app nobody else opened is trivia, not a persona.
const MIN_SHARED_RUNS = 10;
const MIN_SOLE_APP_CALLS = 3;

function strictTop(people, valueOf, min = 1) {
  const ranked = people
    .map((p) => ({ p, v: valueOf(p) }))
    .filter((x) => Number.isFinite(x.v) && x.v >= min)
    .sort((a, b) => b.v - a.v);
  if (!ranked.length) return null;
  if (ranked.length > 1 && ranked[0].v === ranked[1].v) return null;
  return { ...ranked[0], next: ranked[1]?.v ?? null };
}

function strictBottom(people, valueOf, eligible) {
  const ranked = people
    .filter(eligible)
    .map((p) => ({ p, v: valueOf(p) }))
    .filter((x) => Number.isFinite(x.v))
    .sort((a, b) => a.v - b.v);
  if (!ranked.length) return null;
  if (ranked.length > 1 && ranked[0].v === ranked[1].v) return null;
  return ranked[0];
}

function share(p) {
  return p.turns > 0 && p.topTool ? Math.round((p.topTool.value / p.turns) * 100) : 0;
}

function hourText(h) {
  if (h === 0) return "midnight";
  if (h === 12) return "noon";
  return h < 12 ? `${h}am` : `${h - 12}pm`;
}

// Intent ids come from the taxonomy; only the ones distinctive enough to name a
// week get a phrase, everything else falls back to the id itself.
const INTENT_BADGES = {
  edit_or_rewrite: "rewriting, not debugging",
  review_or_critique: "here to review",
  configure_or_setup: "setting things up",
  create_media: "making things",
  facts_or_lookup: "looking things up",
  plan_or_organize: "planning it out",
  automate_or_script: "scripting it away",
  write_prose: "writing it out",
  draft_communication: "drafting messages",
  troubleshooting_or_diagnosis: "chasing failures",
  modify_or_debug_code: "changing or fixing code",
  codebase_or_system_understanding: "reading the codebase",
  summarize_or_extract: "summarising",
  multi_source_synthesis: "joining the dots",
};

// The badge names the habit; the line needs a verb phrase that reads as an
// action so a card can contrast one person's week against the team's without
// saying the same words twice.
const INTENT_VERBS = {
  edit_or_rewrite: "rewriting what was already written",
  review_or_critique: "reading everyone else's work",
  configure_or_setup: "wiring things together",
  create_media: "making things",
  facts_or_lookup: "looking things up",
  plan_or_organize: "planning it out",
  automate_or_script: "scripting the boring parts away",
  write_prose: "writing it out longhand",
  draft_communication: "drafting messages",
  troubleshooting_or_diagnosis: "chasing down failures",
  modify_or_debug_code: "fixing code",
  codebase_or_system_understanding: "reading the codebase",
  summarize_or_extract: "boiling things down",
  multi_source_synthesis: "joining the dots",
  learning_or_explanation: "getting things explained",
};

function intentVerb(key) {
  return INTENT_VERBS[key] ?? key.replace(/_/g, " ");
}

function intentPhrase(key) {
  return INTENT_BADGES[key] ?? key.replace(/_/g, " ");
}

// A second, unrelated fact about the same person, only added when it is big
// enough to be interesting. Every branch reads a counted field, so the colour
// in a line is never invented — it is just a number the reader has not seen yet.
function flavour(p) {
  if (p.nightTurns >= 20) return `${int(p.nightTurns)} of them between midnight and 4am`;
  if (p.topIntent && p.topIntent.value >= 20) return `${int(p.topIntent.value)} of them ${intentPhrase(p.topIntent.key)}`;
  if (p.shortcutRuns >= 10 && p.topShortcut) return `plus ${p.topShortcut.value} runs of ${p.topShortcut.key}`;
  return null;
}

function appLabel(name) {
  return String(name)
    .replace(/^almanac[-.]/, "")
    .replace(/[-._]+/g, " ")
    .trim()
    .replace(/\b\w/g, (c) => c.toUpperCase());
}

const RULES = [
  // Someone with Alma activity and no labeled prompt anywhere is the clearest
  // signal in the set: their whole AI week happened through shortcuts.
  {
    id: "no-prompts",
    find: (people) => {
      const best = people.filter((p) => p.turns === 0 && p.almaActions > 0).sort((a, b) => b.almaActions - a.almaActions)[0];
      if (!best) return null;
      return {
        person: best,
        badge: "never typed a prompt",
        line: "Didn't send a single prompt to any AI all week. Just talked to Alma and got on with it.",
      };
    },
  },
  {
    id: "most-prompts",
    find: (people) => {
      const top = strictTop(people, (p) => p.turns, 10);
      if (!top) return null;
      return {
        person: top.p,
        badge: "out-prompted everyone",
        line: top.p.nightTurns >= 20
          ? "Asked more of the machines than anyone else here, and a fair share of it arrived while the rest of the team was asleep."
          : "Asked more of the machines than anyone else here, and it wasn't close.",
      };
    },
  },
  {
    id: "most-shortcuts",
    find: (people) => {
      const top = strictTop(people, (p) => p.shortcutRuns, 5);
      if (!top) return null;
      return {
        person: top.p,
        badge: "the shortcut habit",
        line: `Set off more shortcuts than anyone here, mostly ${top.p.topShortcut.key}${share(top.p) === 100 ? `, and never opened a second AI all week` : ""}.`,
      };
    },
  },
  {
    id: "most-connectors",
    find: (people) => {
      const top = strictTop(people, (p) => p.connectorRuns, 5);
      if (!top) return null;
      return {
        person: top.p,
        badge: "wired into everything",
        line: "Put Alma to work in more apps than anyone else bothered to, by a distance.",
      };
    },
  },
  {
    id: "widest-repertoire",
    find: (people) => {
      const top = strictTop(people, (p) => p.shortcutsByName.size, 4);
      if (!top) return null;
      return {
        person: top.p,
        badge: "orders off the whole menu",
        line: "Reached for more different shortcuts than anyone else — not just the two everybody knows.",
      };
    },
  },
  {
    id: "long-sessions",
    find: (people) => {
      const top = strictTop(people, (p) => (p.agentSessions >= 5 ? Math.round(p.agentMinutes / p.agentSessions) : NaN), 30);
      if (!top) return null;
      return {
        person: top.p,
        badge: "goes in and stays in",
        line: "Doesn't do quick questions. Opens a session and vanishes into it for most of the afternoon.",
      };
    },
  },
  {
    id: "earliest-peak",
    find: (people) => {
      const top = strictBottom(people, (p) => p.peakHour, (p) => p.turns >= 20 && p.peakHour !== null);
      if (!top) return null;
      return {
        person: top.p,
        badge: `already going at ${hourText(top.v)}`,
        line: `Hits their stride at ${hourText(top.v)}, well before anyone else here gets moving.`,
      };
    },
  },
  // A shortcut the team shares but one person clearly owns. The threshold is a
  // plurality of the team's runs, and the copy quotes both raw counts so the
  // claim can be checked without trusting a percentage.
  {
    id: "owns-a-shortcut",
    find: (people, ctx) => {
      const owners = [];
      for (const [name, team] of ctx.shortcutTotals) {
        // Shared by enough people and run enough times to be worth a card, and
        // one person must hold a clear plurality of it.
        if (team.people < 3 || team.runs < MIN_SHARED_RUNS) continue;
        const top = strictTop(people, (p) => p.shortcutsByName.get(name) ?? NaN, 3);
        if (!top || top.v / team.runs < 0.4) continue;
        owners.push({ top, name, team, frac: top.v / team.runs });
      }
      return owners
        .sort((a, b) => b.frac - a.frac)
        .map(({ top, name, team }) => ({
          person: top.p,
          badge: `the ${name} one`,
          line: `The whole team can run ${name}. Nobody leans on it like this.`,
        }));
    },
  },
  // Their top intent is one no one else's week was built on. Multiple people can
  // qualify; each badge is named from their own intent, so they stay distinct.
  {
    id: "unique-top-intent",
    find: (people, ctx) =>
      people
        .filter((p) => p.topIntent && p.topIntent.value >= 5 && ctx.topIntentCounts.get(p.topIntent.key) === 1)
        .sort((a, b) => b.topIntent.value - a.topIntent.value)
        .map((p) => ({
          person: p,
          badge: intentPhrase(p.topIntent.key),
          line: `While the rest of the team was ${intentVerb(ctx.teamTopIntent)}, they spent the week ${intentVerb(p.topIntent.key)}.${p.shortcutRuns > p.turns ? " And they spoke to Alma more often than they typed to anything." : ""}`,
        })),
  },
  // Being the only person to touch an app is a property, not a ranking, so
  // every sole owner can claim it truthfully and each badge names its own app.
  {
    id: "sole-app",
    find: (people, ctx) =>
      people
        .map((p) => {
          const owned = [...p.connectorsByApp.entries()]
            .filter(([app, runs]) => ctx.appUsers.get(app) === 1 && runs >= MIN_SOLE_APP_CALLS)
            .sort((a, b) => b[1] - a[1])[0];
          return owned ? { p, app: owned[0], runs: owned[1] } : null;
        })
        .filter(Boolean)
        .sort((a, b) => b.runs - a.runs)
        .map(({ p, app, runs }) => ({
          person: p,
          badge: `the only one in ${appLabel(app)}`,
          line: `The one person here who pointed Alma at ${appLabel(app)} — everyone else left it alone.`,
        })),
  },
];

// Nobody is left without a card, and the fallback still cites a real number:
// whichever of their own counts is largest.
function fallback(p) {
  const options = [
    { v: p.turns, badge: `mostly ${p.topToolName}`, line: `Spent the week in ${p.topToolName} and didn't feel the need to shop around.` },
    { v: p.shortcutRuns, badge: "quietly shortcutting", line: "Kept Alma busy with shortcuts without making a thing of it." },
    { v: p.connectorRuns, badge: "the connector user", line: "Let Alma reach into other apps rather than doing it by hand." },
  ].filter((o) => o.v > 0);
  const pick = options.sort((a, b) => b.v - a.v)[0];
  if (!pick) return null;
  return { person: p, badge: pick.badge, line: pick.line };
}

export function assignPersonas(people) {
  const roster = people.filter((p) => p.turns > 0 || p.almaActions > 0);
  if (!roster.length) return [];

  const shortcutTotals = new Map();
  const appUsers = new Map();
  const topIntentCounts = new Map();
  for (const p of roster) {
    for (const [name, runs] of p.shortcutsByName) {
      const t = shortcutTotals.get(name) ?? { runs: 0, people: 0 };
      shortcutTotals.set(name, { runs: t.runs + runs, people: t.people + 1 });
    }
    for (const app of p.connectorsByApp.keys()) appUsers.set(app, (appUsers.get(app) ?? 0) + 1);
    if (p.topIntent) topIntentCounts.set(p.topIntent.key, (topIntentCounts.get(p.topIntent.key) ?? 0) + 1);
  }
  // The team's dominant tool, so a person leading with a different one can be
  // called out as the exception it is.
  const teamTools = new Map();
  for (const p of roster) for (const [tool, n] of p.turnsByTool) teamTools.set(tool, (teamTools.get(tool) ?? 0) + n);
  const teamTopTool = [...teamTools.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? null;
  // How many people lead with each tool: "the only one on X" is a lie the
  // moment two people share it, which a differs-from-the-team check misses.
  const topToolCounts = new Map();
  for (const p of roster) if (p.topTool) topToolCounts.set(p.topTool.key, (topToolCounts.get(p.topTool.key) ?? 0) + 1);
  const teamIntents = new Map();
  for (const p of roster) for (const [k, n] of p.intents) teamIntents.set(k, (teamIntents.get(k) ?? 0) + n);
  const teamTopIntent = [...teamIntents.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? null;
  const ctx = { shortcutTotals, appUsers, topIntentCounts, teamTopTool, topToolCounts, teamTopIntent };

  const taken = new Set();
  const badges = new Set();
  const out = [];
  const claim = (card) => {
    if (!card || taken.has(card.person.person) || badges.has(card.badge)) return;
    taken.add(card.person.person);
    badges.add(card.badge);
    out.push({ person: card.person.person, badge: card.badge, line: card.line });
  };

  for (const rule of RULES) {
    const found = rule.find(roster, ctx);
    if (!found) continue;
    for (const card of Array.isArray(found) ? found : [found]) claim(card);
  }

  for (const p of roster) {
    if (!taken.has(p.person)) claim(fallback(p));
  }

  const order = new Map(roster.map((p, i) => [p.person, i]));
  const rank = (name) => {
    const p = roster[order.get(name)];
    return p.turns + p.almaActions;
  };
  return out.sort((a, b) => rank(b.person) - rank(a.person) || a.person.localeCompare(b.person));
}
