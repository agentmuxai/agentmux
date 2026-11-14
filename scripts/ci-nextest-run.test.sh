#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# ci-nextest-run.test.sh — tests for ci-nextest-run.sh: the retry of a stalled attempt, no retry of a failure, the
# memory-pressure rule, the readings the stalled run of 2026-11-14 produced, the sampler, and the cargo job limit. The
# cargo commands are replaced by CI_ATTEMPT_CMD (or a stub cargo) and the monitor is off.
#
# Usage: bash scripts/ci-nextest-run.test.sh   (exit 0 = all pass)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/ci-nextest-run.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
pass=0
fail=0
ok() { pass=$((pass + 1)); printf '  PASS  %s\n' "$1"; }
bad() { fail=$((fail + 1)); printf '  FAIL  %s\n     -> %s\n' "$1" "${2:-}"; }

# run <name> <expected exit> <expected attempts> <attempt command> [limit seconds]; leaves $out.
run() {
    local name=$1 want=$2 attempts=$3 cmd=$4 limit=${5:-30}
    local dir="$TMP/$name"
    mkdir -p "$dir"
    out=$(STUB="$dir" CI_MONITOR=0 CI_TRACE_FILE="$dir/trace" CI_BUILD_LOG="$dir/build" \
        CI_ATTEMPT1_SECONDS=$limit CI_ATTEMPT2_SECONDS=$limit CI_ATTEMPT_CMD="$cmd" bash "$SCRIPT" 2>&1)
    local got=$? n
    n=$(cat "$dir/n" 2>/dev/null || echo 0)
    if [ "$got" = "$want" ] && [ "$n" = "$attempts" ]; then ok "$name"; else bad "$name" "exit $got (want $want), $n attempts (want $attempts): $out"; fi
}

COUNT='n=$(cat "$STUB/n" 2>/dev/null || echo 0); echo $((n + 1)) > "$STUB/n"'

echo "attempts"
run passes 0 1 "$COUNT"
run "a test failure is not retried" 100 1 "$COUNT; exit 100"
run "a compile error is not retried" 101 1 "$COUNT; exit 101"
run "a stalled attempt is retried once and can pass" 0 2 "$COUNT; if [ \$n -eq 0 ]; then sleep 20; fi" 1
run "a stall twice fails with the timeout status" 124 2 "$COUNT; sleep 20" 1
run "a signal (the runner's shutdown) is retried" 0 2 "$COUNT; if [ \$n -eq 0 ]; then exit 143; fi"
echo "$out" | grep -q "stalled" && ok "the retry is announced" || bad "the retry is announced" "$out"

echo "the stall message names where the trace is"
run "a second stall says so" 124 2 "$COUNT; sleep 20" 1
echo "$out" | grep -q "stalled twice" && ok "second stall is named" || bad "second stall is named" "$out"

echo "memory pressure"
# shellcheck source=ci-nextest-run.sh
source "$SCRIPT"
pressure_high 100 100 && ok "little memory and little swap" || bad "little memory and little swap"
pressure_high 400 8000 && bad "plenty of swap is not pressure above 300 MB" || ok "plenty of swap is not pressure above 300 MB"
pressure_high 299 8000 && ok "under 300 MB is pressure whatever the swap" || bad "under 300 MB is pressure whatever the swap"
pressure_high 300 8000 && bad "300 MB with swap to spare is not pressure" || ok "300 MB with swap to spare is not pressure"
pressure_high 437 12000 && bad "the lowest healthy run (437 MB) is not pressure" || ok "the lowest healthy run (437 MB) is not pressure"
pressure_high 4000 100 && bad "plenty of memory is not pressure" || ok "plenty of memory is not pressure"
pressure_high 511 1023 && ok "just under both limits" || bad "just under both limits"
pressure_high 512 1023 && bad "at the memory limit is not pressure" || ok "at the memory limit is not pressure"

echo "stalled status"
FIRED="$TMP/fired"
for s in 124 137 143; do stalled "$s" && ok "status $s is a stall" || bad "status $s is a stall"; done
for s in 1 100 101; do stalled "$s" && bad "status $s is not a stall" || ok "status $s is not a stall"; done
: > "$FIRED"
stalled 101 && ok "any status after the watchdog fired is a stall" || bad "any status after the watchdog fired is a stall"

echo "the stall of PR 7232 (run 43538724481, 2026-11-14): the readings its trace holds"
# The last samples before the freeze (mem_avail MB, swap_free MB), then the one 20 minutes 49 seconds later. The old rule
# fired on none of them, and neither would any threshold that does not also fire on a healthy run (the lowest of 26
# healthy runs read 338 MB available): the freeze comes between two samples, and the sampler goes dark.
strikes=0
fired=no
for r in "1732 15359" "1289 15359" "1080 15359" "834 15359" "501 14358" "809 12841" "701 12789" "1560 12124" "1278 12139" "1571 12126" "724 11504"; do
    # shellcheck disable=SC2086
    set -- $r
    judge "$strikes" "$1" "$2" 5
    strikes=$STRIKES
    [ "$strikes" -ge 3 ] && fired=yes
done
[ "$fired" = no ] && ok "none of the samples before the freeze reaches the trigger" || bad "none of the samples before the freeze reaches the trigger"
judge 0 816 11199 1249
[ "$STRIKES" -ge 3 ] && ok "a sampler starved for 20 minutes on a machine with 816 MB free fires at once" || bad "a sampler starved for 20 minutes fires at once" "$STRIKES"
judge 0 625 15000 8
[ "$STRIKES" -ge 3 ] && bad "the longest gap of a healthy run (8 s, 625 MB) is not a stall" || ok "the longest gap of a healthy run (8 s, 625 MB) is not a stall"
judge 0 14000 15000 25
[ "$STRIKES" -ge 3 ] && bad "a late sample on an idle machine is not memory" || ok "a late sample on an idle machine is not memory"
judge 2 100 100 5
[ "$STRIKES" -ge 3 ] && ok "the third strike in a row fires" || bad "the third strike in a row fires" "$STRIKES"
judge 2 4000 15000 5
[ "$STRIKES" -eq 0 ] && ok "a good reading clears the strikes" || bad "a good reading clears the strikes" "$STRIKES"

echo "the sampler starts no process and reads real figures"
sample
echo "$SAMPLE" | grep -Eq '^\[resources\] [0-9]{2}:[0-9]{2}:[0-9]{2} mem_avail=[0-9]+MB swap_free=[0-9]+MB mem_used=[0-9]+MB swap_used=[0-9]+MB demand=[0-9]+MB top=[^ ]+:[0-9]+MB psi_full10=[0-9.na]+$' && ok "a sample line has every field" || bad "a sample line has every field" "$SAMPLE"
read_mem
[ "$MEM_AVAIL_MB" -gt 0 ] && [ "$MEM_USED_MB" -gt 0 ] && [ "$SWAP_USED_MB" -ge 0 ] && ok "memory figures are read" || bad "memory figures are read" "$MEM_AVAIL_MB $MEM_USED_MB $SWAP_USED_MB"
top_process
[ -n "$TOP_PID" ] && [ -d "/proc/$TOP_PID" ] && ok "the biggest process exists" || bad "the biggest process exists" "$TOP_PID"
top_process runner-safe
case "$TOP_COMM" in bash | ps | awk | Runner* | dotnet* | systemd* | sshd*) bad "the watchdog never picks a shell or the runner" "$TOP_COMM" ;; *) ok "the watchdog never picks a shell or the runner" ;; esac

echo "the cargo job limit"
STUB_BIN="$TMP/stubbin"
mkdir -p "$STUB_BIN"
printf '#!/usr/bin/env bash\necho "cargo $1: CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-unset}"\n' > "$STUB_BIN/cargo"
chmod +x "$STUB_BIN/cargo"
for mode in pr queue; do
    for jobs in "" 3; do
        log="$TMP/jobs-$mode-$jobs.log"
        in_queue=""
        [ "$mode" = queue ] && in_queue=true
        env PATH="$STUB_BIN:$PATH" IN_QUEUE=$in_queue CI_BUILD_LOG="$log" CI_TRACE_FILE="$TMP/jobs.trace" ${jobs:+CI_CARGO_JOBS=$jobs} bash "$SCRIPT" --attempt > /dev/null 2>&1
        want=${jobs:-2}
        if grep -q "CARGO_BUILD_JOBS=$want" "$log" && ! grep -q "CARGO_BUILD_JOBS=unset" "$log"; then ok "$mode lane builds with $want jobs"; else bad "$mode lane builds with $want jobs" "$(cat "$log")"; fi
    done
done

echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
