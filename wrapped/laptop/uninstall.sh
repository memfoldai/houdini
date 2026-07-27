#!/bin/bash
# Stops and removes the laptop channel services. Leaves ~/.houdini-collector
# (the SQLite store and tokens) in place; delete it by hand to wipe data.
set -euo pipefail

AGENTS="$HOME/Library/LaunchAgents"
for label in ai.memfold.houdini.collector ai.memfold.houdini.tunnel; do
  launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
  rm -f "$AGENTS/$label.plist"
  echo "removed $label"
done
echo "Services stopped. Data kept at ~/.houdini-collector (remove manually to wipe)."
