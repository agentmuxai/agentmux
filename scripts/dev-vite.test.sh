#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# dev-vite.test.sh — tests for scripts/dev-vite.sh (the bookkeeping that ends the Vite `task dev`
# started when its run ends, charlie #611). A stand-in for Vite (a sleeping process whose command
# line carries "vite") is used through AGENTMUX_DEV_VITE_CMD, so no real Vite or port is needed.
#
# Usage: bash scripts/dev-vite.test.sh    (exit 0 = all pass)

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SUT="$HERE/dev-vite.sh"
WORK="$(mktemp -d)"
PASS=0
FAIL=0
PIDS_TO_CLEAN=()

ok() { PASS=$((PASS + 1)); }
bad() { FAIL=$((FAIL + 1)); echo "FAIL: $1"; }
cleanup() {
  for p in "${PIDS_TO_CLEAN[@]:-}"; do [ -n "$p" ] && kill -9 "$p" 2>/dev/null; done
  rm -rf "$WORK"
}
trap cleanup EXIT

# The stand-in: "fake-vite.sh" sleeps; its command line has "vite" and the --port it is given.
cat >"$WORK/fake-vite.sh" <<'EOF'
#!/usr/bin/env bash
exec -a "fake-vite $*" sleep 300
EOF
chmod +x "$WORK/fake-vite.sh"
export AGENTMUX_DEV_VITE_CMD="$WORK/fake-vite.sh"
export AGENTMUX_DEV_VITE_PARENT_POLL=0.2
export AGENTMUX_DEV_VITE_STOP_WAIT=2

alive() { kill -0 "$1" 2>/dev/null; }
wait_for() { # <seconds> <command...>: poll until the command succeeds
  local limit=$(($1 * 10)) n=0
  shift
  while [ "$n" -lt "$limit" ]; do
    "$@" && return 0
    sleep 0.1
    n=$((n + 1))
  done
  return 1
}
gone() { ! alive "$1"; }
has_pidfile() { [ -s "$1" ]; }
pid_of() { cut -d' ' -f1 <"$1"; }

# --- 1. run records the pid and port; stop ends exactly that Vite and removes the file
PF="$WORK/one.pid"
bash "$SUT" run 5301 "$PF" &
SUP=$!
PIDS_TO_CLEAN+=("$SUP")
if wait_for 5 has_pidfile "$PF"; then ok; else bad "run did not write the pidfile"; fi
VITE="$(pid_of "$PF")"
PIDS_TO_CLEAN+=("$VITE")
if alive "$VITE"; then ok; else bad "the recorded pid is not running"; fi
[ "$(cut -d' ' -f2 <"$PF")" = "5301" ] && ok || bad "the pidfile does not record the port"
bash "$SUT" stop "$PF"
if wait_for 5 gone "$VITE"; then ok; else bad "stop did not end the Vite"; fi
if wait_for 5 gone "$SUP"; then ok; else bad "the supervisor outlived its Vite"; fi
[ ! -e "$PF" ] && ok || bad "stop left the pidfile"

# --- 2. a process that is not the recorded Vite is never touched
PF="$WORK/other.pid"
sleep 300 &
OTHER=$!
PIDS_TO_CLEAN+=("$OTHER")
echo "$OTHER 5302" >"$PF"
bash "$SUT" stop "$PF" 2>/dev/null
if alive "$OTHER"; then ok; else bad "stop ended a process that is not the recorded Vite"; fi
[ ! -e "$PF" ] && ok || bad "stop left the pidfile of a process it did not own"
kill "$OTHER" 2>/dev/null

# --- 3. stop with no pidfile, or a garbled one, does nothing and succeeds
bash "$SUT" stop "$WORK/missing.pid" && ok || bad "stop failed without a pidfile"
echo "not-a-pid" >"$WORK/bad.pid"
bash "$SUT" stop "$WORK/bad.pid" && ok || bad "stop failed on a garbled pidfile"

# --- 4. the process that started the supervisor ending (even killed outright) ends the Vite
PF="$WORK/parent.pid"
bash -c "bash '$SUT' run 5303 '$PF' >/dev/null 2>&1 & echo \$! >'$WORK/parent.sup'; exec sleep 300" &
PARENT=$!
PIDS_TO_CLEAN+=("$PARENT")
if wait_for 5 has_pidfile "$PF"; then ok; else bad "run (under a parent) did not write the pidfile"; fi
VITE="$(pid_of "$PF")"
SUP="$(cat "$WORK/parent.sup")"
PIDS_TO_CLEAN+=("$VITE" "$SUP")
kill -9 "$PARENT" 2>/dev/null
if wait_for 8 gone "$VITE"; then ok; else bad "the Vite outlived the process that started it"; fi
if wait_for 5 gone "$SUP"; then ok; else bad "the supervisor outlived the process that started it"; fi
[ ! -e "$PF" ] && ok || bad "the supervisor left its pidfile after the parent ended"

# --- 5. TERM to the supervisor ends its Vite
PF="$WORK/term.pid"
bash "$SUT" run 5304 "$PF" &
SUP=$!
PIDS_TO_CLEAN+=("$SUP")
wait_for 5 has_pidfile "$PF" || bad "run did not write the pidfile (term case)"
VITE="$(pid_of "$PF")"
PIDS_TO_CLEAN+=("$VITE")
kill -TERM "$SUP"
if wait_for 8 gone "$VITE"; then ok; else bad "TERM to the supervisor left the Vite running"; fi
wait "$SUP" 2>/dev/null

# --- 6. the supervisor ends with Vite's own end
PF="$WORK/self.pid"
bash "$SUT" run 5305 "$PF" &
SUP=$!
PIDS_TO_CLEAN+=("$SUP")
wait_for 5 has_pidfile "$PF" || bad "run did not write the pidfile (self case)"
VITE="$(pid_of "$PF")"
PIDS_TO_CLEAN+=("$VITE")
kill -9 "$VITE"
if wait_for 5 gone "$SUP"; then ok; else bad "the supervisor kept running after its Vite ended"; fi

# --- 7. a wrong call is refused
bash "$SUT" run 5306 2>/dev/null && bad "run without a pidfile was accepted" || ok
bash "$SUT" nonsense 2>/dev/null && bad "an unknown command was accepted" || ok

echo "dev-vite.test.sh: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
