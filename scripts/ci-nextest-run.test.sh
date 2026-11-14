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
    out=$(STUB="$dir" CI_MONITOR=0 CI_CEILING=0 CI_TRACE_FILE="$dir/trace" CI_BUILD_LOG="$dir/build" \
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

echo "another step under the same guard"
# CI_STEP_NAME puts the step's name in the messages, which say "nextest" when it is not set (the server package build uses it).
out=$(STUB="$TMP/named" CI_STEP_NAME="server package build" CI_MONITOR=0 CI_CEILING=0 CI_TRACE_FILE="$TMP/named.trace" CI_BUILD_LOG="$TMP/named.build" \
    CI_ATTEMPT1_SECONDS=1 CI_ATTEMPT2_SECONDS=1 CI_ATTEMPT_CMD="sleep 20" bash "$SCRIPT" 2>&1)
echo "$out" | grep -q "The server package build step stalled twice" && ok "the step's name is in the stall message" || bad "the step's name is in the stall message" "$out"
echo "$out" | grep -q "nextest" && bad "no stray nextest in the messages" "$out" || ok "no stray nextest in the messages"
run "the default name is nextest" 124 2 "$COUNT; sleep 20" 1
echo "$out" | grep -q "The nextest step stalled" && ok "the default name is nextest" || bad "the default name is nextest" "$out"

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

echo "the memory ceiling"
# A stub sudo: it records its arguments, runs what follows "--" (the build) for systemd-run, and only records for
# systemctl. SUDO_FAIL=1 makes it refuse.
cat > "$STUB_BIN/sudo" <<'STUBEOF'
#!/usr/bin/env bash
echo "sudo $*" >> "$SUDO_LOG"
[ -n "${SUDO_FAIL:-}" ] && { echo "sudo: a password is required" >&2; exit 1; }
args=("$@")
for i in "${!args[@]}"; do
    if [ "${args[$i]}" = "--" ]; then
        # SUDO_DROP_ENV: a sudo that keeps nothing of the environment.
        [ -n "${SUDO_DROP_ENV:-}" ] && exec env -i "PATH=$PATH" "${args[@]:$((i + 1))}"
        exec "${args[@]:$((i + 1))}"
    fi
done
exit 0
STUBEOF
chmod +x "$STUB_BIN/sudo"
ceiling() { # ceiling <name> [env assignments...]: the real script's main with a stub sudo; leaves $out and $SUDO_LOG
    local name=$1; shift
    local dir="$TMP/c-$name"
    mkdir -p "$dir"
    SUDO_LOG="$dir/sudo.log"
    : > "$SUDO_LOG"
    out=$(env PATH="$STUB_BIN:$PATH" SUDO_LOG="$SUDO_LOG" STUB="$dir" CI_MONITOR=0 CI_TRACE_FILE="$dir/trace" CI_BUILD_LOG="$dir/build" \
        CI_ATTEMPT_CMD='n=$(cat "$STUB/n" 2>/dev/null || echo 0); echo $((n + 1)) > "$STUB/n"' "$@" bash "$SCRIPT" 2>&1)
    CEILING_STATUS=$?
}
ceiling default
[ "$CEILING_STATUS" -eq 0 ] && grep -q "MemoryMax=14G" "$SUDO_LOG" && grep -q "MemorySwapMax=7G" "$SUDO_LOG" && grep -q -- "--unit=ci-nextest-.*-1 " "$SUDO_LOG" \
    && ok "the attempt runs in a scope with 14 GB and 7 GB of swap" || bad "the attempt runs in a scope with 14 GB and 7 GB of swap" "$out / $(cat "$SUDO_LOG")"
echo "$out" | grep -q "memory ceiling: the build runs in its own cgroup, 14G" && ok "the log says the ceiling is on" || bad "the log says the ceiling is on" "$out"
grep -q "systemctl kill --signal=KILL ci-nextest-.*-1.scope" "$SUDO_LOG" && ok "what the attempt left in its scope is killed" || bad "what the attempt left in its scope is killed" "$(cat "$SUDO_LOG")"
grep -q -- "--setenv=PATH=" "$SUDO_LOG" && grep -q -- "--setenv=HOME=" "$SUDO_LOG" && ok "PATH and HOME reach the build through sudo" || bad "PATH and HOME reach the build through sudo" "$(cat "$SUDO_LOG")"
grep -q -- "--preserve-env=[^ ]*CI_NEXTEST_PROBE" "$SUDO_LOG" && ok "the environment's variable names are passed to sudo" || bad "the environment's variable names are passed to sudo" "$(cat "$SUDO_LOG")"
ceiling override CI_MEM_MAX=9G CI_SWAP_MAX=3G
grep -q "MemoryMax=9G" "$SUDO_LOG" && grep -q "MemorySwapMax=3G" "$SUDO_LOG" && ok "CI_MEM_MAX and CI_SWAP_MAX set the ceiling" || bad "CI_MEM_MAX and CI_SWAP_MAX set the ceiling" "$(cat "$SUDO_LOG")"
ceiling refused SUDO_FAIL=1
{ [ "$CEILING_STATUS" -eq 0 ] && [ "$(cat "$TMP/c-refused/n")" = 1 ] && echo "$out" | grep -q "memory ceiling not available, the build runs without it: sudo: a password is required"; } \
    && ok "a refused sudo leaves the step as it was, and says why" || bad "a refused sudo leaves the step as it was, and says why" "$out"
ceiling dropenv SUDO_DROP_ENV=1
{ [ "$CEILING_STATUS" -eq 0 ] && [ "$(cat "$TMP/c-dropenv/n")" = 1 ] && echo "$out" | grep -q "memory ceiling not available, the build runs without it: the environment did not arrive in the scope"; } \
    && ok "a sudo that drops the environment is detected, and the build runs without the ceiling" || bad "a sudo that drops the environment is detected" "$out"
ceiling off CI_CEILING=0
{ echo "$out" | grep -q "memory ceiling: off" && [ ! -s "$SUDO_LOG" ]; } && ok "CI_CEILING=0 starts no sudo" || bad "CI_CEILING=0 starts no sudo" "$out / $(cat "$SUDO_LOG")"

echo "the attempt's cgroup in the trace"
CGROOT="$TMP/cgroot"
mkdir -p "$CGROOT/system.slice/run-1.scope"
echo 3221225472 > "$CGROOT/system.slice/run-1.scope/memory.current"
printf 'low 0\nhigh 0\nmax 12\noom 2\noom_kill 1\n' > "$CGROOT/system.slice/run-1.scope/memory.events"
TRACE="$TMP/scope.trace"
echo /system.slice/run-1.scope > "$TRACE.cgroup"
CGROUP_ROOT="$CGROOT"
read_scope
[ "$SCOPE_FIELDS" = " scope=3072MB oom_kill=1" ] && [ "$SCOPE_OOM" = 1 ] && ok "the scope's memory and kills are read" || bad "the scope's memory and kills are read" "$SCOPE_FIELDS / $SCOPE_OOM"
echo /system.slice/gone.scope > "$TRACE.cgroup"
read_scope
[ -z "$SCOPE_FIELDS" ] && [ "$SCOPE_OOM" = 0 ] && ok "a scope that is gone reads as nothing" || bad "a scope that is gone reads as nothing" "$SCOPE_FIELDS"
# The monitor notices a kill by the kernel, says so in the trace, and marks the attempt as stalled so it is retried.
echo /system.slice/run-1.scope > "$TRACE.cgroup"
FIRED="$TRACE.watchdog"
rm -f "$FIRED"
: > "$TRACE"
monitor > /dev/null 2>&1 &
mon=$!
for _ in 1 2 3 4 5 6 7 8 9 10; do [ -e "$FIRED" ] && break; read -r -t 0.3 <> <(:) || true; done
kill "$mon" 2>/dev/null
wait "$mon" 2>/dev/null
{ [ -e "$FIRED" ] && grep -q "the memory ceiling killed 1 process" "$TRACE" && grep -q "scope=3072MB oom_kill=1" "$TRACE"; } \
    && ok "a kill by the kernel in the scope is reported and counts as a stall" || bad "a kill by the kernel in the scope is reported and counts as a stall" "$(cat "$TRACE")"
rm -f "$TRACE.tick"

echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
