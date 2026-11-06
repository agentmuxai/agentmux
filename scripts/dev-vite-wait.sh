#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# dev-vite-wait.sh — wait until THIS run's Vite answers on a port.
#
# Usage: dev-vite-wait.sh <port> <token> [seconds]   (seconds default 30)
#
# "Something answers" is not enough: a Vite that lost the --strictPort bind to
# another run exits quietly, and the winner's server answers in its place. This waits
# for the token this run gave its Vite (see scripts/dev-instance-plugin.mjs).
#   exit 0  our Vite answered with our token
#   exit 1  a server answered with another token or none (it is not ours: stop), or
#           nothing answered in time
# Our own Vite cannot answer with the wrong token, and before it binds nothing answers,
# so any answer without our token means the port belongs to someone else.
# docs/retro/RETRO_DEV_VITE_PORT_RACE_COUPLED_INSTANCES_2026_11_06.md

set -uo pipefail

port="${1:-}"
token="${2:-}"
secs="${3:-30}"
if [ -z "$port" ] || [ -z "$token" ]; then
  echo "usage: dev-vite-wait.sh <port> <token> [seconds]" >&2
  exit 2
fi

url="http://localhost:$port/__agentmux_dev_instance"
body="$(mktemp 2>/dev/null || echo "${TMPDIR:-/tmp}/dev-vite-wait.$$")"
trap 'rm -f "$body"' EXIT

for _ in $(seq 1 "$secs"); do
  code="$(curl -s --max-time 2 -o "$body" -w '%{http_code}' "$url" 2>/dev/null || true)"
  if [ -n "$code" ] && [ "$code" != "000" ]; then
    if grep -qF "\"token\":\"$token\"" "$body" 2>/dev/null; then
      exit 0
    fi
    echo "❌ Port $port answered, but not with this run's Vite (HTTP $code): another run's dev server holds it." >&2
    if grep -q '"owner"' "$body" 2>/dev/null; then
      echo "   It says: $(tr -d '\r\n' <"$body" | cut -c1-300)" >&2
    fi
    echo "   Nothing was started against it. Take another port: AGENTMUX_VITE_PORT=\$(bash scripts/vite-port.sh pick) task dev" >&2
    exit 1
  fi
  sleep 1
done
echo "❌ Vite did not answer on port $port within ${secs}s." >&2
exit 1
