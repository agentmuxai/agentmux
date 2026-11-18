#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# build-slot.test.sh — tests for build-slot.sh: one holder at a time, whatever `mkdir` says.
#
# The slot was a directory made with mkdir, taken to be exclusive. uutils coreutils' mkdir (Ubuntu
# 26.04) can report success to a caller that lost the race, so the claim is now the shell's exclusive
# create of a `lock` file. A `mkdir` that always says yes stands in for it, so these fail on GNU hosts too.
#
# Usage: bash scripts/build-slot/build-slot.test.sh   (exit 0 = all pass)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SUT="$HERE/build-slot.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
PASS=0; FAIL=0
ok() { PASS=$((PASS + 1)); }
bad() { FAIL=$((FAIL + 1)); echo "FAIL: $1"; }

export AGENTMUX_AGENT_ID=test BUILD_SLOT_POLL=1 BUILD_SLOT_MAX_WAIT=120
unset BUILD_SLOT_HELD BUILD_SLOT_MAX
newroot() { export AGENTMUX_BUILD_SLOT_DIR="$TMP/locks-$1"; rm -rf "$AGENTMUX_BUILD_SLOT_DIR"; mkdir -p "$AGENTMUX_BUILD_SLOT_DIR"; }

# A mkdir that always succeeds, like a caller that lost the race on uutils.
FAKE="$TMP/fake"; mkdir -p "$FAKE"
printf '#!/bin/sh\nfor last; do :; done\n[ -d "$last" ] || /bin/mkdir -p "$last"\nexit 0\n' >"$FAKE/mkdir"
chmod +x "$FAKE/mkdir"

# The command each run executes: take a marker exclusively, so a second run inside the slot at the
# same time is seen, hold it for a moment, give it back.
cat >"$TMP/job.sh" <<'JOB'
#!/usr/bin/env bash
( set -C; : >"$BUSY" ) 2>/dev/null || echo overlap >>"$OVERLAPS"
sleep "${HOLD:-0.4}"
rm -f "$BUSY"
echo done >>"$DONE"
JOB

run_many() { # <n> [PATH prefix]  -> runs n jobs at once; sets OVERLAPS and DONE counts
  local n="$1" pathpre="${2:-}" i
  export BUSY="$TMP/busy" OVERLAPS="$TMP/overlaps" DONE="$TMP/done"
  rm -f "$BUSY" "$OVERLAPS" "$DONE"; : >"$OVERLAPS"; : >"$DONE"
  for i in $(seq 1 "$n"); do
    PATH="${pathpre:+$pathpre:}$PATH" bash "$SUT" --reason "run $i" -- bash "$TMP/job.sh" >/dev/null 2>&1 &
  done
  wait
}

# 1. Runs started together hold the slot one at a time, with a normal mkdir and with one that always says yes.
#    (The queue lets one waiter at a time try the slot, so these pass on the old claim too; test 2 is the
#    one that needs the exclusive create.)
newroot a; run_many 8
[ "$(wc -l <"$OVERLAPS")" = 0 ] && ok || bad "8 simultaneous runs overlapped $(wc -l <"$OVERLAPS") times"
[ "$(wc -l <"$DONE")" = 8 ] && ok || bad "8 simultaneous runs: only $(wc -l <"$DONE") finished"
newroot b; run_many 8 "$FAKE"
[ "$(wc -l <"$OVERLAPS")" = 0 ] && ok || bad "with a mkdir that always succeeds, 8 simultaneous runs overlapped $(wc -l <"$OVERLAPS") times"
[ "$(wc -l <"$DONE")" = 8 ] && ok || bad "with a mkdir that always succeeds, only $(wc -l <"$DONE") of 8 finished"

# 2. A held slot is refused to a second run, which gives up (exit 75) instead of sharing it.
newroot c
BUSY="$TMP/b2" OVERLAPS="$TMP/o2" DONE="$TMP/d2" HOLD=6 bash "$SUT" -- bash "$TMP/job.sh" >/dev/null 2>&1 &
HOLDER=$!
for _ in $(seq 1 50); do [ -f "$AGENTMUX_BUILD_SLOT_DIR/slot/owner" ] && break; sleep 0.1; done
PATH="$FAKE:$PATH" BUILD_SLOT_MAX_WAIT=3 bash "$SUT" -- true >/dev/null 2>&1; st=$?
[ "$st" = 75 ] && ok || bad "a second run on a held slot exited $st, wanted 75 (gave up)"
wait "$HOLDER"
[ ! -d "$AGENTMUX_BUILD_SLOT_DIR/slot" ] && ok || bad "the slot was not removed when its holder finished"

# 3. A slot an older build-slot made (a folder with an owner and a fresh heartbeat, no lock file) is respected.
newroot d
mkdir -p "$AGENTMUX_BUILD_SLOT_DIR/slot"; echo "agent=old" >"$AGENTMUX_BUILD_SLOT_DIR/slot/owner"; touch "$AGENTMUX_BUILD_SLOT_DIR/slot/heartbeat"
BUILD_SLOT_MAX_WAIT=3 bash "$SUT" -- true >/dev/null 2>&1; st=$?
[ "$st" = 75 ] && ok || bad "a live slot without a lock file was taken (exit $st)"

# 4. A slot whose heartbeat stopped (a dead holder) is taken over, with or without a lock file.
newroot e
D="$AGENTMUX_BUILD_SLOT_DIR/slot"; mkdir -p "$D"; : >"$D/lock"; echo "agent=dead" >"$D/owner"; touch "$D/heartbeat"
touch -d '10 minutes ago' "$D/heartbeat" "$D/owner" "$D/lock" "$D" 2>/dev/null || touch -t "$(date -d '10 minutes ago' +%Y%m%d%H%M 2>/dev/null || date -v-10M +%Y%m%d%H%M)" "$D/heartbeat" "$D/owner" "$D/lock" "$D"
bash "$SUT" -- true >/dev/null 2>&1; st=$?
[ "$st" = 0 ] && ok || bad "a stale slot was not taken over (exit $st)"
[ ! -e "$D.takeover" ] && ok || bad "a takeover left its lock behind"

# 5. A takeover lock a killed taker left (older than a minute) does not wedge the slot.
newroot f
D="$AGENTMUX_BUILD_SLOT_DIR/slot"; mkdir -p "$D"; : >"$D/lock"; echo "agent=dead" >"$D/owner"; touch "$D/heartbeat"
: >"$D.takeover"
for f in "$D/heartbeat" "$D/owner" "$D/lock" "$D" "$D.takeover"; do
  touch -d '10 minutes ago' "$f" 2>/dev/null || touch -t "$(date -d '10 minutes ago' +%Y%m%d%H%M 2>/dev/null || date -v-10M +%Y%m%d%H%M)" "$f"
done
bash "$SUT" -- true >/dev/null 2>&1; st=$?
[ "$st" = 0 ] && ok || bad "an abandoned takeover lock blocked a stale slot (exit $st)"

# 6. With room for two (BUILD_SLOT_MAX=2) two runs hold a slot each, a third waits.
newroot g
export BUSY="$TMP/b6" OVERLAPS="$TMP/o6" DONE="$TMP/d6"; rm -f "$BUSY" "$OVERLAPS" "$DONE"
for i in 1 2; do HOLD=5 BUILD_SLOT_MAX=2 BUSY="$TMP/b6-$i" bash "$SUT" -- bash "$TMP/job.sh" >/dev/null 2>&1 & done
for _ in $(seq 1 80); do [ -f "$AGENTMUX_BUILD_SLOT_DIR/slot/owner" ] && [ -f "$AGENTMUX_BUILD_SLOT_DIR/slot-2/owner" ] && break; sleep 0.1; done
if [ -f "$AGENTMUX_BUILD_SLOT_DIR/slot/owner" ] && [ -f "$AGENTMUX_BUILD_SLOT_DIR/slot-2/owner" ]; then ok; else bad "two runs did not each get a slot with BUILD_SLOT_MAX=2"; fi
BUILD_SLOT_MAX=2 BUILD_SLOT_MAX_WAIT=2 bash "$SUT" -- true >/dev/null 2>&1; st=$?
[ "$st" = 75 ] && ok || bad "a third run with both slots held exited $st, wanted 75"
wait

echo "build-slot.test.sh: $PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]
