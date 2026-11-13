#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# dev-vite.sh — starts the Vite dev server for `task dev` and makes sure it ends with the run.
#
# WHY: `task dev` started Vite with `npx vite &` and never stopped it. When the launcher ended
# (a normal quit, a failed start such as exit 78 for a blocked sandbox, a signal) the Vite stayed
# up and held its port until someone found it and killed it by PID (charlie #611). Under go-task's
# shell `$!` is not an operating-system PID, so the PID is recorded here, by a real bash.
#
# Usage:
#   dev-vite.sh run <port> <pidfile>   Start Vite on <port> (--strictPort) as this script's child,
#                                      write "<pid> <port>" to <pidfile>, and keep running as its
#                                      supervisor: when Vite ends this script ends with its exit
#                                      code; when the process that started this script ends (task
#                                      dev, however it ends, even by SIGKILL) or this script gets
#                                      TERM, INT or HUP, Vite is ended. Run it in the background.
#   dev-vite.sh stop <pidfile>         End the Vite recorded in <pidfile>, and only that one: the PID
#                                      must still be a Vite on the recorded port. Always exits 0.
#
# Only the PID this script started is ever signalled; nothing is found by name or command line.
# The command can be replaced with AGENTMUX_DEV_VITE_CMD (the tests use it); it gets --port <port>
# --strictPort appended, like Vite does.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STOP_WAIT_SECONDS="${AGENTMUX_DEV_VITE_STOP_WAIT:-5}"
PARENT_POLL_SECONDS="${AGENTMUX_DEV_VITE_PARENT_POLL:-1}"

# The command line of a PID, or nothing when it is gone. /proc where there is one (Linux, and
# Git Bash on Windows); ps elsewhere (macOS).
cmdline_of() {
  local pid="$1"
  if [ -r "/proc/$pid/cmdline" ]; then
    tr '\0' ' ' <"/proc/$pid/cmdline" 2>/dev/null
  else
    ps -p "$pid" -o command= 2>/dev/null
  fi
}

alive() { kill -0 "$1" 2>/dev/null; }

# End <pid> and wait for it: TERM, then KILL when it has not gone within STOP_WAIT_SECONDS.
end_process() {
  local pid="$1" waited=0
  alive "$pid" || return 0
  kill "$pid" 2>/dev/null
  while alive "$pid" && [ "$waited" -lt $((STOP_WAIT_SECONDS * 10)) ]; do
    sleep 0.1
    waited=$((waited + 1))
  done
  alive "$pid" && kill -9 "$pid" 2>/dev/null
  return 0
}

run() {
  local port="$1" pidfile="$2" parent="$PPID"
  export AGENTMUX_VITE_PORT="$port"
  if [ -n "${AGENTMUX_DEV_VITE_CMD:-}" ]; then
    # shellcheck disable=SC2086 # the test command is a word list on purpose
    $AGENTMUX_DEV_VITE_CMD --port "$port" --strictPort &
  else
    (cd "$ROOT" && exec node node_modules/vite/bin/vite.js --config vite.config.ts --port "$port" --strictPort) &
  fi
  local vite=$!
  echo "$vite $port" >"$pidfile"
  # Watch the parent only when it can be seen: a parent that is not a process this shell can signal
  # (PID 1 or none, as under a native Windows parent) would read as already gone and end Vite at once.
  # Then only `stop` ends it.
  local watch_parent=1
  case "$parent" in '' | 0 | 1 | *[!0-9]*) watch_parent=0 ;; esac
  [ "$watch_parent" -eq 1 ] && ! alive "$parent" && watch_parent=0
  # TERM, INT or HUP to this supervisor ends its Vite: a terminal's Ctrl+C reaches both anyway,
  # but a plain `kill` of the supervisor reaches only this script.
  trap 'end_process "$vite"; rm -f "$pidfile"; exit 143' TERM INT HUP
  while alive "$vite"; do
    if [ "$watch_parent" -eq 1 ] && ! alive "$parent"; then
      echo "dev-vite: the process that started Vite on port $port has ended; ending Vite (pid $vite)." >&2
      end_process "$vite"
      break
    fi
    sleep "$PARENT_POLL_SECONDS"
  done
  wait "$vite" 2>/dev/null
  local rc=$?
  rm -f "$pidfile"
  exit "$rc"
}

stop() {
  local pidfile="$1" pid port
  [ -f "$pidfile" ] || exit 0
  read -r pid port <"$pidfile" || true
  case "$pid" in '' | *[!0-9]*) rm -f "$pidfile"; exit 0 ;; esac
  if alive "$pid"; then
    # Still the Vite recorded? A number reused by another process must not be touched.
    local cmd
    cmd="$(cmdline_of "$pid")"
    case "$cmd" in
      *vite*"--port $port"* | *"--port $port"*vite*) end_process "$pid" ;;
      *) echo "dev-vite: pid $pid is not the Vite on port $port any more; left alone." >&2 ;;
    esac
  fi
  rm -f "$pidfile"
  exit 0
}

case "${1:-}" in
  run)
    [ $# -eq 3 ] || { echo "usage: dev-vite.sh run <port> <pidfile>" >&2; exit 2; }
    run "$2" "$3"
    ;;
  stop)
    [ $# -eq 2 ] || { echo "usage: dev-vite.sh stop <pidfile>" >&2; exit 2; }
    stop "$2"
    ;;
  *)
    echo "usage: dev-vite.sh run <port> <pidfile> | stop <pidfile>" >&2
    exit 2
    ;;
esac
