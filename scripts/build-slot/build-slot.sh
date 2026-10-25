#!/usr/bin/env bash
# build-slot-version: 7
# build-slot: run one heavy build or test command while holding the host-wide
# build slot, so only one such command runs on this machine at a time.
# Spec: docs/specs/SPEC_BUILD_SLOT_ENFORCEMENT_2026_10_20.md. AgentMux installs this file to
# ~/.agentmux/shared/locks/build-slot.sh at startup when its version is newer than the installed one.
#
#   build-slot [--reason "text"] -- <command> [args...]   run <command> when the slot is free
#   build-slot status                                     show the holder and the queue
#   build-slot history                                    waits and holds of past runs, by reason
#
# Works in Git Bash (Windows), Linux and macOS: it needs only bash, mkdir, date, sleep.
# One run at a time by default; BUILD_SLOT_MAX, or a number in <locks>/build/limit, allows more
# (a machine with the memory for two srv compiles). A slot is a directory created with mkdir (atomic). The holder refreshes a heartbeat
# file every 20 s; a slot whose heartbeat is older than STALE_SECS belongs to a dead
# process and is taken over. Waiters line up by ticket, oldest first.
# The whole script is one function, called on the last line. Bash reads a script a piece
# at a time, so a copy of this file replaced while a wait is still running it would break
# that wait mid-line; a function is parsed in full before anything in it runs.
main() {
set -u

ROOT="${AGENTMUX_BUILD_SLOT_DIR:-$HOME/.agentmux/shared/locks/build}"
SLOT="$ROOT/slot"   # the slot this run holds (slot, slot-2, ...) once it has one
QUEUE="$ROOT/queue"
HISTORY="$ROOT/history.log"   # one line per run: epoch, agent, reason, waited, held, exit (tab-separated)
STALE_SECS="${BUILD_SLOT_STALE_SECS:-90}"
MAX_WAIT="${BUILD_SLOT_MAX_WAIT:-3600}"
LIMIT_FILE="$ROOT/limit"   # the number of runs allowed at once on this machine, if not 1
POLL="${BUILD_SLOT_POLL:-3}"
ME="${AGENTMUX_AGENT_ID:-${USER:-unknown}}"
PARENT="$PPID"
# Whether we can watch our parent at all. Git Bash reports pid 1 (and cannot signal it) when
# its parent is a native Windows program (cmd.exe, PowerShell, task.exe), which is how the
# `cargo` shim usually reaches us; a wait must not take that for "the parent is gone".
PARENT_VISIBLE=0
if [ "$PARENT" -gt 1 ] 2>/dev/null && kill -0 "$PARENT" 2>/dev/null; then PARENT_VISIBLE=1; fi

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
# How many runs may hold a slot at once: $BUILD_SLOT_MAX, else the first number in the limit
# file, else 1. At most 16.
slot_limit() {
  local n="${BUILD_SLOT_MAX:-}"
  [ -n "$n" ] || n=$(head -1 "$LIMIT_FILE" 2>/dev/null | tr -dc '0-9')
  case "$n" in ''|*[!0-9]*) n=1 ;; esac
  n=$((10#$n))
  [ "$n" -ge 1 ] || n=1
  [ "$n" -le 16 ] || n=16
  echo "$n"
}
slot_dir() { if [ "$1" -le 1 ]; then echo "$ROOT/slot"; else echo "$ROOT/slot-$1"; fi; }
dfield() { sed -n "s/^$2=//p" "$1/owner" 2>/dev/null | head -1; } # slot dir, key
field() { dfield "$SLOT" "$1"; }
# A field as a number: an owner file still being written, or a slot released a
# moment ago, reads as empty, which must not break the arithmetic around it.
num_field() { local v; v=$(dfield "$SLOT" "$1"); case "$v" in ''|*[!0-9]*) v=$(now);; esac; echo "$v"; }

slot_is_stale() { # the slot (the dir in $SLOT) exists but nobody is keeping it alive
  local hb age
  hb=$(mtime "$SLOT/heartbeat")
  [ "$hb" -eq 0 ] && hb=$(mtime "$SLOT")   # owner not written yet: use the dir's age
  age=$(( $(now) - hb ))
  [ "$age" -gt "$STALE_SECS" ]
}

status() {
  # Every slot there is room for: "slot" for the first, "slot-2", ... for the others.
  local lim i d free=0 saved="$SLOT"
  lim=$(slot_limit)
  for d in "$ROOT"/slot-*; do # a slot left from a higher limit that is still held shows too
    [ -d "$d" ] || continue
    i="${d##*/slot-}"; case "$i" in ''|*[!0-9]*) continue ;; esac
    [ "$i" -gt "$lim" ] && lim="$i"
  done
  for i in $(seq 1 "$lim"); do
    d=$(slot_dir "$i"); SLOT="$d"
    local label="slot"; [ "$i" -gt 1 ] && label="slot-$i"
    if [ -d "$d" ]; then
      if slot_is_stale; then free=$((free+1)); [ "$lim" -eq 1 ] && echo "$label: STALE (previous holder died), free to take over"
      else
        echo "$label: HELD by $(field agent) since $(( ($(now) - $(num_field start)) / 60 )) min: $(field cmd)"
        echo "      in $(field cwd)${BUILD_SLOT_VERBOSE:+ (pid $(field pid))}"
      fi
    else free=$((free+1)); [ "$lim" -eq 1 ] && echo "$label: free"; fi
  done
  SLOT="$saved"
  [ "$lim" -gt 1 ] && echo "slots: $((lim - free)) of $lim in use"
  local n=0 t
  for t in "$QUEUE"/*; do
    [ -e "$t" ] || continue
    [ $(( $(now) - $(mtime "$t") )) -gt 30 ] && continue
    n=$((n+1)); echo "  queued #$n: $(cat "$t")"
  done
  [ "$n" -eq 0 ] && echo "  queue: empty"
  return 0
}

# One line per finished or given-up run, to tune the list of heavy commands and to see whether
# a second slot would help. Short lines appended with >> do not interleave.
log_history() { # waited-secs held-secs exit-code
  local reason="${REASON:-${CMDLINE:-}}"
  reason=$(printf '%s' "$reason" | tr '\t\n' '  ' | cut -c1-120)
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$(now)" "$ME" "$reason" "$1" "$2" "$3" >> "$HISTORY" 2>/dev/null || true
}
# Two generations: past 2000 lines the log is renamed to history.log.1 (replacing the older one)
# and the next append starts a new file. A rename rewrites nothing, so a line a waiter appends
# at that moment lands in one file or the other and is never dropped. Only the holder calls
# it, so two rotations never run together.
rotate_history() {
  [ "$(wc -l < "$HISTORY" 2>/dev/null || echo 0)" -gt 2000 ] || return 0
  mv -f "$HISTORY" "$HISTORY.1" 2>/dev/null
  return 0
}

history() {
  [ -s "$HISTORY" ] || [ -s "$HISTORY.1" ] || { echo "history: no runs recorded yet ($HISTORY)"; return 0; }
  local files=() f
  for f in "$HISTORY.1" "$HISTORY"; do [ -s "$f" ] && files+=("$f"); done
  awk -F'\t' '
    { n++; w[$3]+=$4; h[$3]+=$5; c[$3]++; if ($4>wm[$3]) wm[$3]=$4; tw+=$4; th+=$5; if ($6==75) gu++; if ($4>0) q++ }
    END {
      printf "runs: %d, waited at all: %d, gave up: %d, total wait %d min, total held %d min\n", n, q, gu, tw/60, th/60
      printf "%-40s %5s %9s %9s %10s\n", "reason", "runs", "avg wait", "max wait", "held (min)"
      for (r in c) printf "%-40s %5d %8ds %8ds %10d\n", substr(r,1,40), c[r], w[r]/c[r], wm[r], h[r]/60
    }' "${files[@]}"
}

if [ "${1:-}" = "status" ]; then status; exit 0; fi
if [ "${1:-}" = "history" ]; then history; exit 0; fi

REASON=""
if [ "${1:-}" = "--reason" ]; then REASON="${2:-}"; shift 2; fi
[ "${1:-}" = "--" ] && shift
[ $# -ge 1 ] || { echo "usage: build-slot [--reason text] -- <command> [args...]" >&2; exit 64; }

# A command started inside a held slot (task -> cargo) must not wait for itself.
if [ "${BUILD_SLOT_HELD:-}" = "1" ]; then exec "$@"; fi

# A lock folder that can't be made or written must never stop the build: say so and run the
# command without the slot.
if ! { mkdir -p "$QUEUE" 2>/dev/null && [ -w "$ROOT" ] && [ -w "$QUEUE" ]; }; then
  echo "build-slot: can't use the lock folder $ROOT; running without the build slot" >&2
  exec "$@"
fi
QUEUED_AT=$(now)
TICKET="$QUEUE/$QUEUED_AT-$$"
CMDLINE="$*"
TOKEN="$$-$(now)-${RANDOM:-0}"   # tells our slot from one a later holder created
printf '%s: %s\n' "$ME" "$CMDLINE" > "$TICKET"
HOLDING=0
LOGGED=0
ACQUIRED_AT=0
HB_PID=""
release() {
  rm -f "$TICKET"
  [ -n "$HB_PID" ] && kill "$HB_PID" 2>/dev/null
  # Remove the slot only if it is still ours: if our heartbeat died and a waiter
  # took the slot over, deleting it now would free it under the new holder.
  if [ "$HOLDING" -eq 1 ]; then
    # A run cut short by a signal never reaches the logging after its command; record it here
    # (130 is the exit code the signal trap gives).
    if [ "$LOGGED" -eq 0 ]; then
      LOGGED=1
      log_history "$(( ACQUIRED_AT - QUEUED_AT ))" "$(( $(now) - ACQUIRED_AT ))" "${rc:-130}"
    fi
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

# Take the first free slot, up to the limit. Only the run at the head of the queue tries.
# Returns 0 with SLOT set when it got one, 1 when all are held, 2 when the lock folder turned
# out to be unusable.
try_slots() {
  local lim i d
  lim=$(slot_limit)
  for i in $(seq 1 "$lim"); do
    d=$(slot_dir "$i")
    SLOT="$d"
    if [ -d "$d" ] && slot_is_stale; then rm -rf "$d"; fi
    if mkdir "$d" 2>/dev/null; then return 0; fi
    # Neither made nor already there: the folder went read-only or away since the check above.
    [ -d "$d" ] || return 2
  done
  SLOT="$ROOT/slot"
  return 1
}

waited=0; said=0
while :; do
  touch "$TICKET" 2>/dev/null
  if first_in_line; then
    try_slots; got=$?
    if [ "$got" -eq 0 ]; then HOLDING=1; ACQUIRED_AT=$(now); break; fi
    if [ "$got" -eq 2 ]; then
      rm -f "$TICKET"
      echo "build-slot: can't create a slot under $ROOT; running without the build slot" >&2
      exec "$@"
    fi
  fi
  if [ "$waited" -ge "$MAX_WAIT" ]; then
    log_history "$(( $(now) - QUEUED_AT ))" 0 75
    echo "build-slot: gave up after ${MAX_WAIT}s waiting. $(status | head -2 | tr '\n' ' ')" >&2; exit 75
  fi
  if [ "$said" -eq 0 ] || [ $((waited % 60)) -lt "$POLL" ]; then
    wait_note "build-slot: waiting for the build slot ($ME has waited $(( $(now) - QUEUED_AT ))s, gives up after ${MAX_WAIT}s) — $(status | head -2 | tr '\n' ' ')"; said=1
  fi
  # A wait whose parent is gone (the tool or shell that started it was stopped) must not
  # keep a ticket in the queue: leave. BUILD_SLOT_DETACHED=1 turns this off for a wait
  # started on purpose with nohup or setsid.
  if [ "$PARENT_VISIBLE" = 1 ] && [ -z "${BUILD_SLOT_DETACHED:-}" ] && ! kill -0 "$PARENT" 2>/dev/null; then
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
echo "build-slot: acquired by $ME after $(( ACQUIRED_AT - QUEUED_AT ))s: $CMDLINE" >&2

BUILD_SLOT_HELD=1 "$@"
rc=$?
# Still holding the slot here, so a rotation cannot overlap another holder's.
LOGGED=1
log_history "$(( ACQUIRED_AT - QUEUED_AT ))" "$(( $(now) - ACQUIRED_AT ))" "$rc"
rotate_history
echo "build-slot: released (exit $rc)" >&2
exit "$rc"
}
main "$@"; exit $?
