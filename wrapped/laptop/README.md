# Laptop channel — collector + Cloudflare quick tunnel

Turns your Mac into the team's collector with no domain and no server. Two
LaunchAgents run at login and stay up:

- **`ai.memfold.houdini.collector`** — the Node collector on `127.0.0.1:8787`,
  storing to `~/.houdini-collector/collector.sqlite`.
- **`ai.memfold.houdini.tunnel`** — a `cloudflared` quick tunnel that exposes it
  publicly, and publishes the assigned `*.trycloudflare.com` URL to a private
  GitHub gist whenever it changes.

Every teammate's Houdini reads that gist (baked as `HOUDINI_UPLOAD_DISCOVERY`)
to find the current URL, then uploads with the baked ingest token. A quick
tunnel's URL rotates on restart; publishing it to the gist plus device-side
retries means a rotation is absorbed within a couple of minutes, never lost.

## Setup

Prereqs: `brew install cloudflared`, and `gh` logged in (`gh auth status`).

1. Create the tokens + discovery gist (once):

   ```sh
   mkdir -p ~/.houdini-collector && chmod 700 ~/.houdini-collector
   printf 'https://pending.trycloudflare.com\n' > /tmp/houdini-collector.txt
   GID=$(basename "$(gh gist create --desc 'Houdini collector pointer' /tmp/houdini-collector.txt)")
   cat > ~/.houdini-collector/tokens.env <<EOF
   INGEST_TOKEN=hd_ingest_$(openssl rand -hex 16)
   ADMIN_TOKEN=hd_admin_$(openssl rand -hex 16)
   TEAM_NAME=Houdini
   GIST_ID=$GID
   EOF
   chmod 600 ~/.houdini-collector/tokens.env
   ```

2. Install and start the services:

   ```sh
   ./install.sh
   ```

3. The discovery URL to bake into the OTA is the gist's raw URL:
   `https://gist.githubusercontent.com/<you>/<GIST_ID>/raw/houdini-collector.txt`

## Operate

```sh
tail -f ~/.houdini-collector/ai.memfold.houdini.*.log ~/.houdini-collector/cloudflared.log
launchctl list | grep houdini            # both services should be listed
curl -s localhost:8787/health            # -> ok
gh api gists/$GIST_ID --jq '.files["houdini-collector.txt"].content'   # current URL
./uninstall.sh                           # stop + remove (keeps the SQLite data)
```

Pull last week's wrapped once devices have checked in (ADMIN_TOKEN from
`tokens.env`):

```sh
open "$(gh api gists/$GIST_ID --jq '.files["houdini-collector.txt"].content')/v1/wrapped/$(date -v-mon -v-7d +%F)?key=$ADMIN_TOKEN"
```

## Caveats

- **Your laptop is the server.** While it's asleep the tunnel is down; teammates'
  devices keep retrying and land their data once you're back online.
- **Quick tunnels are best-effort.** Cloudflare gives no uptime guarantee and the
  URL changes on every `cloudflared` restart — the gist + retries handle it. For
  a permanent URL, use a Cloudflare **named** tunnel (needs a domain) and bake a
  static `HOUDINI_UPLOAD_URL` instead of the discovery pointer.
