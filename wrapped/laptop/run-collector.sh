#!/bin/bash
# Runs the team collector as a login-persistent service. launchd keeps it alive;
# storage and logs live under ~/.houdini-collector so the running service is
# independent of any repo checkout.
set -euo pipefail

DIR="$HOME/.houdini-collector"
set -a
# shellcheck disable=SC1091
source "$DIR/tokens.env"
set +a

export PORT="${PORT:-8787}"
export DB_PATH="$DIR/collector.sqlite"
export WRAPPED_DIR="$DIR/wrapped"
# Roll the weekly archive at the operator's local Monday, not UTC midnight.
export TZ="${TZ:-$(readlink /etc/localtime | sed 's#.*/zoneinfo/##')}"

exec /opt/homebrew/bin/node "$DIR/app/collector/server.mjs"
