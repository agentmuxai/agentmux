#!/usr/bin/env bash
# build-slot-version: 3
# build-slot: run one heavy build or test command while holding the host-wide
# build slot, so only one such command runs on this machine at a time.
# Written up in two places: the standard, docs/standards/DEV_HOST_BUILD_SLOT.md in
# a5af/shared-infrastructure, and AgentMux's spec, docs/specs/SPEC_BUILD_SLOT_ENFORCEMENT_2026_10_20.md
# in agentmuxai/agentmux. AgentMux installs this file to ~/.agentmux/shared/locks/build-slot.sh
# at startup when its version is newer than the installed one.
#
#   build-slot [--reason "text"] -- <command> [args...]   run <command> when the slot is free
#   build-slot status                                     show the holder and the queue
#
# Works in Git Bash (Windows), Linux and macOS: it needs only bash, mkdir, date, sleep.
# The slot is a directory created with mkdir (atomic). The holder refreshes a heartbeat
# file every 20 s; a slot whose heartbeat is older than STALE_SECS belongs to a dead
# process and is taken over. Waiters line up by ticket, oldest first.
set -u

ROOT="${AGENTMUX_BUILD_SLOT_DIR:-$HOME/.agentmux/shared/locks/build}"
SLOT="$ROOT/slot"
QUEUE="$ROOT/queue"
STALE_SECS="${BUILD_SLOT_STALE_SECS:-90}"
MAX_WAIT="${BUILD_SLOT_MAX_WAIT:-3600}"
POLL="${BUILD_SLOT_POLL:-3}"
ME="${AGENTMUX_AGENT_ID:-${USER:-unknown}}"
PARENT="$PPID"

now() { date +%s; }
# The "waiting" line goes to the terminal when there is one, even if stdout and stderr are
# redirected to a file or a pipe: an agent's Bash wrapper stops a command that prints nothing
# for 10 minutes, and a wait behind a long build prints nothing else. With no terminal
# (cron, CI) it goes to stderr.
wait_note() { { echo "$*" > /dev/tty; } 2>/dev/null || echo "$*" >&2; }
mtime() { # file -> epoch seconds of last change, 0 if missing
  local f="$1" t
  [ -e "$f" ] || { echo 0; return; }
  t=$(stat -c %Y "$f" 2>/dev/null) || t=$(stat -f %m "$f" 2>/dev/null) || t=0
  echo "$t"
}
field() { sed -n "s/^$1=//p" "$SLOT/owner" 2>/dev/null | head -1; }
# A field as a number: an owner file still being written, or a slot released a
# moment ago, reads as empty, which must not break the arithmetic around it.
num_field() { local v; v=$(field "$1"); case "$v" in ''|*[!0-9]*) v=$(now);; esac; echo "$v"; }

slot_is_stale() { # the slot exists but nobody is keeping it alive
  local hb age
  hb=$(mtime "$SLOT/heartbeat")
  [ "$hb" -eq 0 ] && hb=$(mtime "$SLOT")   # owner not written yet: use the dir's age
  age=$(( $(now) - hb ))
  [ "$age" -gt "$STALE_SECS" ]
}

status() {
  if [ -d "$SLOT" ]; then
    if slot_is_stale; then echo "slot: STALE (previous holder died), free to take over"
    else
      echo "slot: HELD by $(field agent) since $(( ($(now) - $(num_field start)) / 60 )) min: $(field cmd)"
      echo "      in $(field cwd)${BUILD_SLOT_VERBOSE:+ (pid $(field pid))}"
    fi
  else echo "slot: free"; fi
  local n=0 t
  for t in "$QUEUE"/*; do
    [ -e "$t" ] || continue
    [ $(( $(now) - $(mtime "$t") )) -gt 30 ] && continue
    n=$((n+1)); echo "  queued #$n: $(cat "$t")"
  done
  [ "$n" -eq 0 ] && echo "  queue: empty"
  return 0
}

if [ "${1:-}" = "status" ]; then status; exit 0; fi

REASON=""
if [ "${1:-}" = "--reason" ]; then REASON="${2:-}"; shift 2; fi
[ "${1:-}" = "--" ] && shift
[ $# -ge 1 ] || { echo "usage: build-slot [--reason text] -- <command> [args...]" >&2; exit 64; }

# A command started inside a held slot (task -> cargo) must not wait for itself.
if [ "${BUILD_SLOT_HELD:-}" = "1" ]; then exec "$@"; fi

mkdir -p "$QUEUE" || exit 70
TICKET="$QUEUE/$(now)-$$"
CMDLINE="$*"
TOKEN="$$-$(now)-${RANDOM:-0}"   # tells our slot from one a later holder created
printf '%s: %s\n' "$ME" "$CMDLINE" > "$TICKET"
HOLDING=0
HB_PID=""
release() {
  rm -f "$TICKET"
  [ -n "$HB_PID" ] && kill "$HB_PID" 2>/dev/null
  # Remove the slot only if it is still ours: if our heartbeat died and a waiter
  # took the slot over, deleting it now would free it under the new holder.
  if [ "$HOLDING" -eq 1 ]; then
    [ "$(field token)" = "$TOKEN" ] && rm -rf "$SLOT"
    HOLDING=0
  fi
}
trap release EXIT
trap 'exit 130' INT TERM HUP

first_in_line() {
  local t oldest=""
  for t in "$QUEUE"/*; do
    [ -e "$t" ] || continue
    if [ $(( $(now) - $(mtime "$t") )) -gt 30 ] && [ "$t" != "$TICKET" ]; then rm -f "$t"; continue; fi
    oldest="$t"; break   # glob order is ticket order: epoch seconds, then pid
  done
  [ "$oldest" = "$TICKET" ]
}

waited=0; said=0
while :; do
  touch "$TICKET"
  if first_in_line; then
    if [ -d "$SLOT" ] && slot_is_stale; then rm -rf "$SLOT"; fi
    if mkdir "$SLOT" 2>/dev/null; then HOLDING=1; break; fi
  fi
  if [ "$waited" -ge "$MAX_WAIT" ]; then
    echo "build-slot: gave up after ${MAX_WAIT}s waiting. $(status | head -2 | tr '\n' ' ')" >&2; exit 75
  fi
  if [ "$said" -eq 0 ] || [ $((waited % 60)) -lt "$POLL" ]; then
    wait_note "build-slot: waiting ($ME) — $(status | head -2 | tr '\n' ' ')"; said=1
  fi
  # A wait whose parent is gone (the tool or shell that started it was stopped) must not
  # keep a ticket in the queue: leave. BUILD_SLOT_DETACHED=1 turns this off for a wait
  # started on purpose with nohup or setsid.
  if [ -z "${BUILD_SLOT_DETACHED:-}" ] && ! kill -0 "$PARENT" 2>/dev/null; then
    echo "build-slot: the process that started this wait ($PARENT) is gone; leaving the queue" >&2; exit 129
  fi
  # Sleep in the background and wait for it, so a signal is handled at once, not after the sleep.
  sleep "$POLL" & wait $! 2>/dev/null
  waited=$((waited + POLL))
done

{ echo "agent=$ME"; echo "pid=$$"; echo "token=$TOKEN"; echo "start=$(now)"; echo "cwd=$PWD"; echo "cmd=$CMDLINE";
  [ -n "$REASON" ] && echo "reason=$REASON"; } > "$SLOT/owner"
touch "$SLOT/heartbeat"
rm -f "$TICKET"
# The heartbeat keeps no descriptor of ours open (a caller that captures our
# output, `$(build-slot ...)` or `| tee`, would otherwise wait for its last sleep),
# and it stops when we are gone or the slot is no longer ours.
( while kill -0 $$ 2>/dev/null && [ "$(field token)" = "$TOKEN" ]; do
    touch "$SLOT/heartbeat" 2>/dev/null; sleep 20
  done ) >/dev/null 2>&1 </dev/null &
HB_PID=$!
echo "build-slot: acquired by $ME after ${waited}s: $CMDLINE" >&2

BUILD_SLOT_HELD=1 "$@"
rc=$?
echo "build-slot: released (exit $rc)" >&2
exit "$rc"
