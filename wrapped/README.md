# Houdini Team Wrapped

A Spotify-Wrapped-style weekly recap of a team's AI usage, built from the
redacted analytics Houdini already exports. Two decoupled systems:

1. **The wrapped generator** (`lib/`, `generate.mjs`) — pure, zero-dependency
   Node. Analytics NDJSON in → one self-contained story-reel HTML out. Runs
   anywhere Node runs; no network.
2. **The collector** (`collector/`) — a small service that receives each
   device's analytics, stores them in SQLite, and serves (or archives) the
   rendered wrapped. This is the *channel* — how many laptops' data reaches one
   place.

Everything is zero-dependency: Node's own `http` server and `node:sqlite`
(available flag-free since Node 22.13). No framework, no build step.

---

## Week boundary

A week runs **Monday–Sunday**. "Last week" is the Mon–Sun that just ended
relative to the operator's local date, so running it on a Monday reports the
week that closed the day before. Row days are UTC (matching the export), so the
boundary is at most a timezone-offset off at the very edges — immaterial at
weekly resolution. Override with `--week=YYYY-MM-DD` (any date inside the target
week) or `?week=` on the collector. The laptop collector service sets `TZ` from
the Mac's own zone so its automatic archive rolls at your local Monday.

---

## Generate a wrapped locally

```sh
node generate.mjs --team="Houdini" --out=wrapped.html \
  ~/Library/Application\ Support/ai.memfold.houdini/data/analytics.jsonl
node generate.mjs --week=2026-07-22 a.jsonl b.jsonl   # a specific past week
```

Open the HTML in any browser. Tap (or → / space) to advance, tap the left third
(or ←) to go back, **hold to pause and read**. `#N` in the URL deep-links to a
card. The last card has a **save the card** button for a shareable PNG.

Refresh a device's own export first with `houdini --export-once`. Run the tests
with `npm test` (generator) and `npm run smoke` (spawns the collector and drives
ingest → export → wrapped end to end).

---

## The metrics

A general, Spotify-style recap first — team-first, one idea per card,
superlatives so nobody is "last": total engaged time, prompts sent, top tool,
tools used, what the team worked on, research-vs-doing, peak hour, busiest day,
longest session, nested-AI handoffs, the most-prolific reveal, a top-5 podium,
and a unique per-person badge for everyone. Then two team-specific trophies as
the finale:

- **🏆 The Alma × Claude Code Award** — most Claude Code run through Alma
  (`tool == openclaw && delegate_tool == claude_code`, by turns).
- **🔬 The Alma × Research Award** — most research through Alma
  (`tool == openclaw && shape == asking`, by turns).

Engaged time is gap-aware: an idle gap over 30 minutes (a step-away, a sleep, a
weekend) ends a work burst and does not count, so a CLI transcript resumed over
days reports minutes worked, not its calendar span.

---

## The channel

Endpoints (all JSON/NDJSON):

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `POST` | `/v1/ingest` | `Bearer INGEST_TOKEN` | a device uploads its analytics NDJSON (UPSERT by row identity) |
| `GET` | `/v1/export?week=YYYY-MM-DD` | `Bearer ADMIN_TOKEN` (or `?key=`) | pull a week's raw rows |
| `GET` | `/v1/wrapped/<monday>` | `Bearer ADMIN_TOKEN` (or `?key=`) | the rendered wrapped |
| `GET` | `/health` | none | liveness |

Env: `PORT` (8787), `DB_PATH`, `WRAPPED_DIR`, `TEAM_NAME` (Houdini), `TZ`,
`INGEST_TOKEN` (required — ingest is fail-closed until set), `ADMIN_TOKEN`
(required — export/wrapped fail-closed until set).

**No duplication, nothing missed.** The collector upserts by each row's full
dimension identity, so a device re-sending its whole export is idempotent.
Devices re-upload on startup, every 15 minutes, and immediately on wake, with
curl-level retries — so a laptop coming back online catches the collector up
with no gaps and no dupes.

### Run it — two ways

**A) On your own laptop, via a Cloudflare quick tunnel (no domain, no login).**
This is what `laptop/install.sh` sets up: the collector and a `cloudflared`
quick tunnel run as LaunchAgents; the tunnel's URL is published to a private
GitHub gist that every device reads before uploading, so a rotated URL never
strands anyone. See [`laptop/README.md`](laptop/README.md).

```sh
# one-time: put strong tokens + your gist id in ~/.houdini-collector/tokens.env
laptop/install.sh          # stages the runtime, installs + starts both services
laptop/uninstall.sh        # stop and remove the services (keeps data)
```

Devices are pointed at the gist (not a fixed URL) via two baked-at-release
values: `HOUDINI_UPLOAD_DISCOVERY` (the gist raw URL) and `HOUDINI_UPLOAD_TOKEN`
(the ingest token). Quick tunnels have no uptime guarantee and their URL changes
on restart; the gist + retries absorb that. For a rock-solid endpoint, use a
Cloudflare **named** tunnel (needs a domain) or set a static
`HOUDINI_UPLOAD_URL` — the app prefers a static URL when one is baked.

**B) Anywhere, in Docker.** The same collector containerized for an always-on
host (e.g. an Azure VM):

```sh
cd collector
printf 'INGEST_TOKEN=%s\nADMIN_TOKEN=%s\nTEAM_NAME=Houdini\n' \
  "$(openssl rand -hex 16)" "$(openssl rand -hex 16)" > .env
docker compose up -d --build
```

`node:24-slim`, non-root, persists to the `collector-data` volume, weekly
archive to `WRAPPED_DIR`.

---

## Security model

- **Ingest token vs admin token.** The token baked into the app is the *ingest*
  token — it only permits `POST`ing already-redacted analytics, never reading.
  The *admin* token (export + wrapped) lives only on the collector host and your
  laptop, never in the shipped binary. Because Houdini ships from a public repo,
  treat the baked ingest token and discovery URL as extractable: worst case an
  outsider can push junk redacted rows to a write-only endpoint. That is the
  accepted tradeoff for a small internal team; a named tunnel with per-device
  tokens is the upgrade path if the team grows.
- **Redaction is upstream.** Houdini redacts before export; the collector only
  ever holds aggregate cells and spans, never message text.
- **Tokens never touch argv.** The uploader passes the bearer token to `curl`
  through a config on stdin; the collector compares tokens in constant time.
- Keep `.env` and `~/.houdini-collector/tokens.env` out of version control, and
  never commit generated `*.html` — it carries your team's names and stats.
