# Usage analytics

Houdini records *that* you used an AI and *which* one. Analytics answers the
next question: **what for**. It classifies each request you made against a
fixed taxonomy and stores the result locally, so team-wide usage can be
understood without anyone reading anyone's messages.

## What it does

A background job wakes hourly, takes a small batch of user turns that have not
been labeled yet, and asks the organization's LiteLLM proxy to classify each one
against a closed set of labels. Four facets are recorded per turn:

| Facet | Values | Question it answers |
|---|---|---|
| `tool` | `Alma`, `Claude Code`, `Codex`, `ChatGPT`, `Claude`, `Gemini` | Which app the request went through |
| `intent` | 20 ids, `src/taxonomy.rs` | What was the AI asked to do |
| `domain` | 17 ids | What subject the request belongs to |
| `depth` | 1 to 4 | A single lookup, an iterative dig, a synthesis across sources, or autonomous multi-step work |
| `delegation` | `none`, `tool_call`, `agent_run` | Whether the person drove this AI directly, had it call a tool, or had it drive **another** AI |

`delegation: agent_run` is how nested usage becomes data. Asking Claude Code to
have Codex do the work is one request with one label, and it is counted as such
rather than inferred later from two unrelated transcripts.

## Why a taxonomy and not clustering

Analytics from many laptops has to add up. If every machine clustered its own
data, one would produce "debugging code" and another "fixing bugs", and no
honest merge exists afterwards. So the label space is **pinned by construction**:

- The taxonomy is a versioned constant compiled into the app, and the request
  uses strict JSON-schema structured output, so the model **cannot** return a
  label outside it.
- Every stored row pins `taxonomy_version`, `prompt_version` and `model`. Rows
  from two machines are comparable when those three match, and a future taxonomy
  revision coexists with old labels instead of overwriting them.
- Anything that genuinely does not fit gets `other`, plus a **proposal** in
  `label_candidates` with an observation count. Proposals are reviewed and
  promoted into the next taxonomy version by a human. Nothing is auto-promoted,
  and no device ever invents an id at runtime.

The share of turns landing in `other` is the coverage signal. A rising rate
means the taxonomy has drifted from what people actually do, and it is the cue
to run a promotion pass.

## What leaves the machine

The redacted text of your own requests goes to the configured proxy so it can be
classified. Nothing else does: assistant replies are never labeled, and no
content is written to the analytics tables. Redaction removes credential and
identifier shapes (keys, emails, cards) before storage and therefore before
labeling, but it deliberately keeps ordinary prose, which includes the names of
people, companies and places you mention. Build with `--features ner` to strip
personal identifiers as well.

## The export is a dashboard fact table

`data/analytics.jsonl` emits one `analytics_cell` row per unique combination of
dimensions, with a turn count as the measure. The grain is:

**Identity**: `person` (groups one human's machines), `device_name` (the
machine's friendly name), `device` (a stable install id, the join key).
**Dimensions**: `day`, `tool` / `tool_name`, `provider`, `surface`, `model`,
`intent`, `domain`, `depth`, `delegation`, plus `taxonomy_version` and
`prompt_version`.
**Measures**: `turns`, `sessions` (distinct), `chars` (redacted volume).

That is a star-schema fact row, so a dashboard slices it directly. A leaderboard
of who used a given tool most is a single grouping:

```sql
SELECT person, tool_name, SUM(turns) AS turns, SUM(sessions) AS sessions
FROM analytics_cells
WHERE day >= '2026-07-01'
GROUP BY person, tool_name
ORDER BY turns DESC;
```

Three measures rather than one because they answer different questions and
disagree in useful ways: `turns` counts requests, `sessions` counts distinct
pieces of work, and `chars` weighs how substantial they were. Ranking on turns
alone rewards whoever types the most one-liners.

`tool` is the stable stored id and `tool_name` is the product name to display,
so renaming a product never invalidates historical rows.

No text, no rationales, no session content leaves the device.

## Configuration

`~/Library/Application Support/ai.memfold.houdini/config.json`:

| Key | Default | Meaning |
|---|---|---|
| `analytics_enabled` | `true` | Turn the job off entirely |
| `analytics_base_url` | `https://litellm.memfold.ai` | OpenAI-compatible endpoint |
| `analytics_model` | `gpt-5.5` | Model id |
| `analytics_interval_ms` | `3600000` | How often the job wakes |
| `analytics_batch_limit` | `25` | Turns per batch |

## Getting the key onto a machine

The key lives in the login Keychain, never in the config file and never in the
app bundle. **It is deliberately not compiled into the binary**: releases are
public, so a baked key would be extractable from the DMG by anyone who
downloads it. Three ways in, pick per machine:

**One command, send it to the team.** The key is read from stdin, so it never
reaches the process list or shell history:

```sh
printf %s 'PASTE_KEY_HERE' | /Applications/Houdini.app/Contents/MacOS/houdini --set-analytics-key
```

Labeling starts on the next hourly tick. Run it again at any time to rotate the
key. The same line works unchanged as an MDM script.

**Run it through the installed app, not a dev build.** macOS ties a Keychain
item to the signature of whatever created it, so an item written by a different
binary makes the app prompt on every read. Both the key and the database key are
read exactly **once per launch** and held for the process lifetime, so a correct
setup prompts at most once, ever, and never per analytics job. (This is the same
single-reader discipline that fixed the per-message password prompts in 0.4.10.)

**Backfill on demand.** Label a batch immediately instead of waiting for the
hourly tick, which is how you work through a history the first time:

```sh
houdini --analyze-once 200
```

Without a key the job stays off and says so once in the log. Pausing recording
pauses labeling too.

## Why the key is on each machine rather than the data on one machine

The alternative is routing every laptop's text to one operator machine that
holds the key. That is worse on three counts: coworkers' work conversations
would come to rest on a colleague's laptop rather than only reaching the
organization's own proxy; the labeling job would depend on that laptop being
awake and reachable, so it would stall silently whenever it was not; and it
would need an internet-facing ingest endpoint with its own authentication,
which is real attack surface for no analytical gain. Keeping the key next to
the data means each machine talks only to the proxy the team already uses.

Bound the exposure at the proxy instead: a dedicated LiteLLM virtual key with
`models` restricted to the labeling model, a `max_budget`, and rate limits.
Rotation is `/key/regenerate` with a grace period, then re-provision.

## Cost and pacing

Labeling is per user turn, and user turns are a small share of captured
traffic. A full backfill of a mature database (roughly a thousand user turns)
costs a few dollars once; steady state is a few cents a day. Batches are small
and hourly on purpose: the job is never allowed to become the reason a laptop is
busy, and a failed batch leaves its turns queued rather than dropping them.

## Determinism

The proxy does not accept `temperature: 0` for this model, and identical
sampling does not guarantee identical output on hosted inference anyway. The
guarantee here is the **closed output space**: strict schema plus enum means the
set of legal answers is finite and known, so variation can only be *which* legal
label, never an invented one. A label that somehow arrives outside the taxonomy
is refused rather than stored, and the turn stays queued.
