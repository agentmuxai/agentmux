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
#   vite-port.sh pick [start]       claim and print the first usable, free port >= start
#                                   (default 5300)
#   vite-port.sh claim <port>       take the claim on a port for this clone; exit 1 if
#                                   another clone holds a live one
#   vite-port.sh release <port>     drop this clone's claim on a port
#
# CLAIMS: a free port is not a reserved port. `task dev` can wait most of an hour
# for the shared build slot between choosing its port and binding it, and runs that
# wait together all start Vite in the same second. A claim is a directory made with
# mkdir (atomic), under AGENTMUX_DEV_PORT_CLAIMS (default ~/.agentmux/dev-port-claims),
# holding the clone that took it. One that nobody has renewed for
# AGENTMUX_DEV_PORT_CLAIM_TTL seconds (default 7200) is taken over.
# docs/retro/RETRO_DEV_VITE_PORT_RACE_COUPLED_INSTANCES_2026_11_06.md

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

CLAIMS_DIR="${AGENTMUX_DEV_PORT_CLAIMS:-${HOME:-/tmp}/.agentmux/dev-port-claims}"
CLAIM_TTL="${AGENTMUX_DEV_PORT_CLAIM_TTL:-7200}"

# Who holds a claim: this clone, named from where the script lives so `pick` run from
# any directory and `task dev` (run from the clone root) agree. A test sets
# AGENTMUX_DEV_PORT_OWNER to be several clones at once.
claim_owner() {
  printf '%s' "${AGENTMUX_DEV_PORT_OWNER:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)}"
}

claim_field() { sed -n "s/^$2=//p" "$1/claim" 2>/dev/null | head -1; }

# Written whole and renamed into place, so a reader never sees half a claim.
write_claim() { # <dir> <since>
  local tmp="$1/claim.tmp.$$"
  {
    echo "owner=$(claim_owner)"
    echo "by=${AGENTMUX_AGENT_ID:-${USER:-unknown}}"
    echo "since=$2"
    echo "seen=$(date +%s)"
  } >"$tmp" && mv -f "$tmp" "$1/claim"
}

cmd_claim() {
  local port="${1:-}" d now owner seen since i
  is_number "$port" || { echo "claim: '$port' is not a port number" >&2; return 2; }
  mkdir -p "$CLAIMS_DIR" 2>/dev/null || { echo "vite-port.sh: cannot create $CLAIMS_DIR" >&2; return 2; }
  d="$CLAIMS_DIR/$port"
  now="$(date +%s)"
  if mkdir "$d" 2>/dev/null; then
    write_claim "$d" "$now"
    return 0
  fi
  # Someone made it first; they may not have written their name yet.
  for i in 1 2 3 4 5 6 7 8 9 10; do
    [ -f "$d/claim" ] && break
    sleep 0.1
  done
  owner="$(claim_field "$d" owner)"
  seen="$(claim_field "$d" seen)"
  since="$(claim_field "$d" since)"
  if [ -n "$owner" ] && [ "$owner" = "$(claim_owner)" ]; then
    write_claim "$d" "${since:-$now}"
    return 0
  fi
  [ -n "$seen" ] || seen="$(stat -c %Y "$d" 2>/dev/null || stat -f %m "$d" 2>/dev/null || echo "$now")"
  if [ $((now - seen)) -gt "$CLAIM_TTL" ]; then
    # Nobody renewed it: its run is gone. Take it over under a lock, so two takers
    # cannot each delete the other's fresh claim. The lock is dropped on every way out,
    # and one a killed taker left (older than a minute) is cleared, or the port would
    # stay refused for good.
    [ -n "$(find "$d.lock" -maxdepth 0 -mmin +1 2>/dev/null)" ] && rmdir "$d.lock" 2>/dev/null
    if mkdir "$d.lock" 2>/dev/null; then
      taken=1
      seen="$(claim_field "$d" seen)"
      if [ -z "$seen" ] || [ $((now - seen)) -gt "$CLAIM_TTL" ]; then
        rm -rf "$d"
        if mkdir "$d" 2>/dev/null && write_claim "$d" "$now"; then taken=0; fi
      fi
      rmdir "$d.lock" 2>/dev/null
      [ "$taken" = 0 ] && return 0
    fi
  fi
  echo "❌ Port $port is claimed by another dev run: ${owner:-unknown} (agent $(claim_field "$d" by), since $(date -d "@${since:-$now}" '+%H:%M' 2>/dev/null || echo "${since:-?}"))." >&2
  echo "   Take another port: AGENTMUX_VITE_PORT=\$(bash scripts/vite-port.sh pick) task dev" >&2
  return 1
}

cmd_release() {
  local d="$CLAIMS_DIR/${1:-}"
  [ -n "${1:-}" ] && [ -f "$d/claim" ] || return 0
  [ "$(claim_field "$d" owner)" = "$(claim_owner)" ] && rm -rf "$d"
  return 0
}

cmd_pick() {
  local port="${1:-5300}"
  require_probe || return 1
  is_number "$port" || { echo "start port '$port' is not a number" >&2; return 1; }
  while [ "$port" -le 65535 ]; do
    # The claim is the reservation: it is taken in the same step as the pick, so two
    # picks at once cannot both get this port.
    if ! is_blocked "$port" && ! is_listening "$port" && cmd_claim "$port" 2>/dev/null; then
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
  claim) cmd_claim "${2:-}" ;;
  release) cmd_release "${2:-}" ;;
  *)
    echo "usage: vite-port.sh {blocked|check|listening|claim|release} <port> | pick [start]" >&2
    exit 2
    ;;
esac
