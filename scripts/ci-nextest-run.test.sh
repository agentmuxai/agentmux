#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# ci-nextest-run.test.sh — tests for ci-nextest-run.sh: the retry of a stalled attempt, no retry of a failure, and
# the memory-pressure rule. The cargo commands are replaced by CI_ATTEMPT_CMD and the monitor is off.
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
pressure_high 100 8000 && bad "plenty of swap is not pressure" || ok "plenty of swap is not pressure"
pressure_high 4000 100 && bad "plenty of memory is not pressure" || ok "plenty of memory is not pressure"
pressure_high 511 1023 && ok "just under both limits" || bad "just under both limits"
pressure_high 512 1023 && bad "at the memory limit is not pressure" || ok "at the memory limit is not pressure"

echo "stalled status"
FIRED="$TMP/fired"
for s in 124 137 143; do stalled "$s" && ok "status $s is a stall" || bad "status $s is a stall"; done
for s in 1 100 101; do stalled "$s" && bad "status $s is not a stall" || ok "status $s is not a stall"; done
: > "$FIRED"
stalled 101 && ok "any status after the watchdog fired is a stall" || bad "any status after the watchdog fired is a stall"

echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
