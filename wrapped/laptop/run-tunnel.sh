#!/bin/bash
# Exposes the local collector through a Cloudflare quick tunnel (no domain, no
# login) and publishes the assigned URL to the discovery gist every time it
# changes. Devices read that gist, so a rotated tunnel URL never strands them.
# When cloudflared exits, this script exits and launchd restarts it.
set -euo pipefail

DIR="$HOME/.houdini-collector"
set -a
# shellcheck disable=SC1091
source "$DIR/tokens.env"
set +a

PORT="${PORT:-8787}"
CFLOG="$DIR/cloudflared.log"
: > "$CFLOG"

# 127.0.0.1 (not localhost) so cloudflared never races IPv6/IPv4 resolution of
# the origin. --no-autoupdate keeps the pinned binary under launchd.
/opt/homebrew/bin/cloudflared tunnel --url "http://127.0.0.1:$PORT" --no-autoupdate >>"$CFLOG" 2>&1 &
CF=$!
trap 'kill "$CF" 2>/dev/null || true' EXIT INT TERM

published=""
while kill -0 "$CF" 2>/dev/null; do
  url="$(grep -oE 'https://[a-z0-9.-]+\.trycloudflare\.com' "$CFLOG" | tail -1 || true)"
  if [ -n "$url" ] && [ "$url" != "$published" ]; then
    printf '{"files":{"houdini-collector.txt":{"content":"%s"}}}' "$url" > "$DIR/gist-payload.json"
    if /opt/homebrew/bin/gh api -X PATCH "/gists/$GIST_ID" --input "$DIR/gist-payload.json" >/dev/null 2>&1; then
      published="$url"
      echo "$(date '+%F %T') published $url to gist $GIST_ID"
    else
      echo "$(date '+%F %T') gist update failed (will retry) for $url"
    fi
  fi
  sleep 5
done

echo "$(date '+%F %T') cloudflared exited; launchd will restart this service"
