#!/bin/bash
# Installs the laptop channel: copies the collector runtime into
# ~/.houdini-collector/app, installs two LaunchAgents (collector + tunnel), and
# starts them. Idempotent — safe to re-run after a code change. Requires
# ~/.houdini-collector/tokens.env (INGEST_TOKEN, ADMIN_TOKEN, GIST_ID) to exist.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
WRAPPED="$(cd "$HERE/.." && pwd)"
DIR="$HOME/.houdini-collector"
AGENTS="$HOME/Library/LaunchAgents"

[ -f "$DIR/tokens.env" ] || { echo "error: $DIR/tokens.env missing (run setup first)"; exit 1; }
command -v /opt/homebrew/bin/cloudflared >/dev/null || { echo "error: cloudflared not installed (brew install cloudflared)"; exit 1; }

echo "==> Staging runtime into $DIR/app"
mkdir -p "$DIR/app" "$DIR/wrapped" "$AGENTS"
rm -rf "$DIR/app/lib" "$DIR/app/collector"
cp -R "$WRAPPED/lib" "$DIR/app/lib"
cp -R "$WRAPPED/collector" "$DIR/app/collector"
cp "$HERE/run-collector.sh" "$HERE/run-tunnel.sh" "$DIR/"
chmod +x "$DIR/run-collector.sh" "$DIR/run-tunnel.sh"

write_agent() {
  local label="$1" script="$2"
  cat > "$AGENTS/$label.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$label</string>
  <key>ProgramArguments</key>
  <array><string>/bin/bash</string><string>$DIR/$script</string></array>
  <key>EnvironmentVariables</key>
  <dict><key>PATH</key><string>/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin</string></dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>$DIR/$label.out.log</string>
  <key>StandardErrorPath</key><string>$DIR/$label.err.log</string>
</dict>
</plist>
PLIST
}

echo "==> Writing LaunchAgents"
write_agent "ai.memfold.houdini.collector" "run-collector.sh"
write_agent "ai.memfold.houdini.tunnel" "run-tunnel.sh"

echo "==> (Re)loading services"
for label in ai.memfold.houdini.collector ai.memfold.houdini.tunnel; do
  launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
  launchctl bootstrap "gui/$(id -u)" "$AGENTS/$label.plist"
  launchctl kickstart -k "gui/$(id -u)/$label" 2>/dev/null || true
done

echo "==> Done. Tail logs with:"
echo "    tail -f $DIR/ai.memfold.houdini.*.log $DIR/cloudflared.log"
