#!/bin/bash
# Exposes the local collector through a Cloudflare quick tunnel (no domain, no
# login) and publishes the assigned URL to the discovery gist whenever it
# changes. Devices read that gist, so a rotated tunnel URL never strands them.
#
# Self-healing: a network change (new wifi, wake from sleep) can wedge
# cloudflared — it keeps its URL but its edge connection dies and it retries
# forever without exiting, so launchd's KeepAlive never fires. So this script
# actively health-checks the PUBLIC url; if we're online but the tunnel won't
# answer, it kills cloudflared and exits, and launchd restarts it clean (new
# url, republished). When offline (lid shut), it waits and does nothing.
set -euo pipefail

DIR="$HOME/.houdini-collector"
set -a
# shellcheck disable=SC1091
source "$DIR/tokens.env"
set +a

PORT="${PORT:-8787}"
CFLOG="$DIR/cloudflared.log"
: > "$CFLOG"
CURL=/usr/bin/curl
GH=/opt/homebrew/bin/gh
# The discovery gist belongs to the rahul-memfold account; the machine's
# ACTIVE gh account may be a different login, and a wrong-account PATCH 404s.
# Pin the gist token explicitly, falling back to the default account.
GIST_TOKEN="$($GH auth token --user rahul-memfold 2>/dev/null || true)"
publish_gist() {
  if [ -n "$GIST_TOKEN" ]; then
    GH_TOKEN="$GIST_TOKEN" $GH api -X PATCH "/gists/$GIST_ID" --input "$DIR/gist-payload.json" >/dev/null 2>&1
  else
    $GH api -X PATCH "/gists/$GIST_ID" --input "$DIR/gist-payload.json" >/dev/null 2>&1
  fi
}

/opt/homebrew/bin/cloudflared tunnel --url "http://127.0.0.1:$PORT" --no-autoupdate >>"$CFLOG" 2>&1 &
CF=$!
trap 'kill "$CF" 2>/dev/null || true' EXIT INT TERM

online() { $CURL -sf --max-time 6 -o /dev/null https://api.github.com/zen; }

published=""
fails=0
last_publish_try=0
while kill -0 "$CF" 2>/dev/null; do
  url="$(grep -oE 'https://[a-z0-9.-]+\.trycloudflare\.com' "$CFLOG" | tail -1 || true)"
  if [ -n "$url" ] && [ "$url" != "$published" ]; then
    # Publish retries are spaced 5 minutes apart: a 20s retry loop once tripped
    # GitHub's rate limit and then kept itself limited for a day and a half.
    now=$(date +%s)
    if [ $((now - last_publish_try)) -ge 300 ]; then
      last_publish_try=$now
      printf '{"files":{"houdini-collector.txt":{"content":"%s"}}}' "$url" > "$DIR/gist-payload.json"
      if publish_gist; then
        published="$url"
        fails=0
        echo "$(date '+%F %T') published $url to gist $GIST_ID"
      else
        echo "$(date '+%F %T') gist update failed (retry in 5m) for $url"
      fi
    fi
  fi

  # Health-check the tunnel's OWN url, published or not: an unpublished wedged
  # tunnel must still be detected and restarted, or the publish loop above
  # retries a dead hostname forever.
  if [ -n "$url" ]; then
    if $CURL -s --max-time 8 "$url/health" | grep -q ok; then
      fails=0
    elif online; then
      fails=$((fails + 1))
      echo "$(date '+%F %T') tunnel unreachable while online ($fails/3)"
      if [ "$fails" -ge 3 ]; then
        echo "$(date '+%F %T') cloudflared is wedged; restarting for a fresh tunnel"
        kill "$CF" 2>/dev/null || true
        break
      fi
    fi
  fi

  sleep 20
done

echo "$(date '+%F %T') cloudflared stopped; launchd will restart this service"
