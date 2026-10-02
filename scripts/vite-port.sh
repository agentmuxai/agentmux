#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# vite-port.sh — checks and picks the port `task dev` serves Vite on.
#
# WHY: the dev window is a Chromium window, and Chromium refuses to load a page
# from certain ports (net/base/port_util.cc, "restricted ports") no matter what
# is listening there. Port 6000 (X11) is one of them. A dev instance pointed at
# a blocked port gets ERR_UNSAFE_PORT on every page it loads and the window just
# flickers. See docs/specs/SPEC_DEV_VITE_UNSAFE_PORT_GUARD_2026_10_02.md.
#
# Usage:
#   vite-port.sh blocked <port>     exit 0 if Chromium refuses the port
#   vite-port.sh check <port>       exit 0 if usable; else one error, exit 1
#   vite-port.sh listening <port>   exit 0 if something is listening on it
#   vite-port.sh pick [start]       print the first usable, free port >= start
#                                   (default 5300)

set -uo pipefail

# Chromium's restricted ports from 1024 up (lower ports fail the floor in
# `check` anyway). A snapshot: Chromium has added entries over time (4190, 6679
# and 6697 are recent). The host's load-error page is the backstop for drift.
BLOCKED_PORTS="1719 1720 1723 2049 3659 4045 4190 5060 5061 6000 6566 6665 6666 6667 6668 6669 6679 6697 10080"

is_number() { case "$1" in '' | *[!0-9]*) return 1 ;; *) return 0 ;; esac; }

# Compare as numbers: Vite and Chromium both read 06000 as 6000. More than five
# digits is never a port (and would overflow the arithmetic below).
is_blocked() {
  local want="$1" p
  is_number "$want" || return 1
  want="${want#"${want%%[!0]*}"}" # strip leading zeros
  [ -n "$want" ] && [ "${#want}" -le 5 ] || return 1
  for p in $BLOCKED_PORTS; do
    [ "$p" = "$want" ] && return 0
  done
  return 1
}

is_windows() {
  case "$(uname -s 2>/dev/null)" in MINGW* | MSYS* | CYGWIN*) return 0 ;; *) return 1 ;; esac
}

# `listening` and `pick` are only as good as the tool behind them. A missing
# tool must stop them loudly: swallowing its "not found" would report every
# port as free, and `pick` could hand back one that is in use.
require_probe() {
  if is_windows; then
    command -v netstat >/dev/null 2>&1 && return 0
    echo "vite-port.sh: netstat not found, cannot tell which ports are in use" >&2
  else
    command -v lsof >/dev/null 2>&1 && return 0
    command -v ss >/dev/null 2>&1 && return 0
    echo "vite-port.sh: neither lsof nor ss found, cannot tell which ports are in use" >&2
  fi
  return 1
}

is_listening() {
  if is_windows; then
    netstat -ano 2>/dev/null | grep -i LISTENING | grep -q ":$1 "
  elif command -v lsof >/dev/null 2>&1; then
    [ -n "$(lsof -ti ":$1" -sTCP:LISTEN 2>/dev/null | head -1)" ]
  else
    ss -ltn 2>/dev/null | grep -q ":$1 "
  fi
}

cmd_check() {
  local port="${1:-}"
  # A plain number, no leading zero (06000 is 6000 to Vite but would slip past a
  # string comparison) and at most five digits (longer would overflow -gt).
  if ! is_number "$port" || [ "${#port}" -gt 5 ] || [ "${port#0}" != "$port" ] ||
    [ "$port" -lt 1024 ] || [ "$port" -gt 65535 ]; then
    echo "❌ AGENTMUX_VITE_PORT='$port' is not a plain port number between 1024 and 65535." >&2
    return 1
  fi
  if is_blocked "$port"; then
    echo "❌ Port $port is on Chromium's blocked-ports list, so the dev window would refuse to load" >&2
    echo "   anything from it (ERR_UNSAFE_PORT) and just flicker. Pick another, for example:" >&2
    echo "     AGENTMUX_VITE_PORT=\$(bash scripts/vite-port.sh pick) task dev" >&2
    return 1
  fi
  return 0
}

cmd_pick() {
  local port="${1:-5300}"
  require_probe || return 1
  is_number "$port" || { echo "start port '$port' is not a number" >&2; return 1; }
  while [ "$port" -le 65535 ]; do
    if ! is_blocked "$port" && ! is_listening "$port"; then
      echo "$port"
      return 0
    fi
    port=$((port + 1))
  done
  echo "no free port found" >&2
  return 1
}

case "${1:-}" in
  blocked) is_blocked "${2:-}" ;;
  check) cmd_check "${2:-}" ;;
  listening) require_probe && is_listening "${2:-}" ;;
  pick) cmd_pick "${2:-}" ;;
  *)
    echo "usage: vite-port.sh {blocked|check|listening} <port> | pick [start]" >&2
    exit 2
    ;;
esac
