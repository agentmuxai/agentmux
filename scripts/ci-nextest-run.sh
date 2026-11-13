#!/usr/bin/env bash
# Copyright 2026, AgentMux Corp.
# SPDX-License-Identifier: Apache-2.0
#
# The Ubuntu leg's cargo-nextest step of ci-pr.yml: build and run the CEF-free crates' tests.
#
#   scripts/ci-nextest-run.sh              run it (the workflow's step)
#   scripts/ci-nextest-run.sh --attempt    one attempt, run by the orchestrator below
#
# A run takes 13 to 18 minutes on a PR and about 7 in the merge queue. On 2026-11-12 one stalled for 40 minutes on a
# PR that changed no Rust: the log was never uploaded, and the step's own timeout did not end it, because the job's
# 35-minute limit (counted from the job's start) came before the step's 35 (counted from the step's start, two
# minutes later), and a runner that has stopped answering cannot acknowledge a cancel, so the service waited out its
# grace period. This script keeps a stall from costing that:
#
#   1. A resource trace every 5 seconds, written to a file as well as the log (CI_TRACE_FILE), which the workflow
#      uploads when the step fails or is cancelled.
#   2. A watchdog on that trace. The stall is a machine that has run out of memory and spends its time paging: the
#      runner's agent starves with everything else and can neither stream the log nor end the step. Three samples in
#      a row with almost no memory left (under 300 MB available, or under 512 MB with swap nearly full too) kill the
#      biggest process (the rustc that is growing), which is what the kernel's OOM killer would do after a long time
#      paging. The step then fails with the cause printed, the runner stays able to upload the trace, and the step can
#      retry. Each trace line also carries the kernel's memory pressure (psi_full10).
#   3. A time limit per attempt (CI_ATTEMPT1_SECONDS, CI_ATTEMPT2_SECONDS) set near twice the normal run, and one
#      automatic retry of a STALLED attempt (timed out, killed by the watchdog or by a signal). The retry reuses what
#      the first attempt compiled. A failing test or a compile error is not retried.
#
# CI_ATTEMPT_CMD replaces the cargo commands, CI_MONITOR=0 turns the trace and watchdog off: both are for
# scripts/ci-nextest-run.test.sh.

set -uo pipefail

SELF="${BASH_SOURCE[0]}"
TRACE="${CI_TRACE_FILE:-${RUNNER_TEMP:-/tmp}/resource-trace.log}"
BUILD_LOG="${CI_BUILD_LOG:-${RUNNER_TEMP:-/tmp}/nextest-build.log}"
FIRED="$TRACE.watchdog"
CRATES="-p agentmux-common -p agentmux-srv -p agentmux-launcher -p agentmux-bashwrap -p agentmux-mcp -p agentmux-remote -p agentmux-procstats"

# Whether the machine is out of memory: below 512 MB available and 1 GB of swap free, or below 300 MB available
# whatever the swap. The second rule is for the stall that left no log (2026-11-12 and 11-13, nine times: a job ran to
# its 50-minute limit and the runner never answered the cancel). Healthy runs reach 437 to 483 MB available while
# rustc holds ~14.9 GB of the runner's 16 GB, and use only 3 to 4 GB of the 16 GB of swap, so a few hundred MB more
# does not exhaust the swap: the machine pages the code and cache it runs on, and crawls, with swap to spare and
# the kernel's OOM killer never called. Arguments in MB.
pressure_high() {
    local mem_avail=$1 swap_free=$2
    [ "$mem_avail" -lt 300 ] || { [ "$mem_avail" -lt 512 ] && [ "$swap_free" -lt 1024 ]; }
}

# The kernel's memory pressure: the share of the last 10 s in which every task was stalled on memory ("full avg10").
memory_psi() {
    awk '/^full/ {split($2, a, "="); print a[2]; found = 1} END {if (!found) print "na"}' /proc/pressure/memory 2>/dev/null || echo na
}

# Whether an attempt's exit status means it stalled rather than failed: timed out (124), killed (137), terminated
# (143, the runner's shutdown signal), or the watchdog killed something.
stalled() {
    case "$1" in 124 | 137 | 143) return 0 ;; esac
    [ -e "$FIRED" ]
}

# MB figures from /proc/meminfo: available memory and free swap.
meminfo_mb() {
    awk '/^MemAvailable:/ {a=$2} /^SwapFree:/ {s=$2} END {printf "%d %d", a / 1024, s / 1024}' /proc/meminfo
}

sample() {
    local mem swap
    read -r mem swap < <(meminfo_mb)
    echo "[resources] $(date -u +%H:%M:%S) mem_avail=${mem}MB swap_free=${swap}MB $(free -m | awk 'NR==2{printf "mem_used=%dMB ", $3} NR==3{printf "swap_used=%dMB", $3}') disk_root_avail=$(df -m --output=avail / | tail -1 | tr -d ' ')MB top=$(ps -eo rss=,comm= --sort=-rss | head -1 | awk '{printf "%s:%dMB", $2, $1/1024}') psi_full10=$(memory_psi)"
}

# Kill the process that holds the most memory, leaving the runner's own processes alone, and say what it was.
kill_biggest() {
    local pid rss comm
    read -r pid rss comm < <(ps -eo pid=,rss=,comm= --sort=-rss | awk '$3 !~ /^(Runner|dotnet|systemd|sshd|ps|awk)/ {print; exit}')
    [ -n "${pid:-}" ] || return 0
    echo "[watchdog] memory is nearly gone; killing pid $pid ($comm, $((rss / 1024)) MB). Biggest processes:"
    ps -eo pid=,rss=,etimes=,args= --sort=-rss | head -8 | cut -c1-200
    : > "$FIRED"
    kill -KILL "$pid" 2>/dev/null || true
}

monitor() {
    local strikes=0 mem swap
    while true; do
        sample | tee -a "$TRACE"
        read -r mem swap < <(meminfo_mb)
        if pressure_high "$mem" "$swap"; then strikes=$((strikes + 1)); else strikes=0; fi
        if [ "$strikes" -ge 3 ]; then
            kill_biggest 2>&1 | tee -a "$TRACE"
            strikes=0
        fi
        sleep 5
    done
}

# One attempt: the build and the tests. A failure of any command is the attempt's status.
attempt() {
    set -o pipefail
    if [ -n "${CI_ATTEMPT_CMD:-}" ]; then
        bash -c "$CI_ATTEMPT_CMD"
        return $?
    fi
    if [ "${IN_QUEUE:-}" = true ]; then
        # The queue builds only what its tests need: srv's unit tests (where most guards are), mcp's unit tests and
        # common's one integration test. The other crates keep the cheaper check (srv's own is covered by its test
        # build).
        cargo check --tests -p agentmux-common -p agentmux-launcher -p agentmux-bashwrap -p agentmux-mcp -p agentmux-remote -p agentmux-procstats 2>&1 | tee -a "$BUILD_LOG" || return $?
        cargo nextest run --profile queue -p agentmux-common -p agentmux-srv -p agentmux-mcp --bin agentmux-srv --bin agentmux-mcp --test spawn_no_window_inventory 2>&1 | tee -a "$BUILD_LOG" || return $?
        return 0
    fi
    # shellcheck disable=SC2086 # CRATES is a list of arguments
    cargo nextest run --no-run $CRATES --bin agentmux-srv 2>&1 | tee -a "$BUILD_LOG" || return $?
    # shellcheck disable=SC2086
    cargo nextest run --profile ci $CRATES 2>&1 | tee -a "$BUILD_LOG"
}

main() {
    : > "$TRACE"
    : > "$BUILD_LOG"
    rm -f "$FIRED"
    local monitor_pid=""
    if [ "${CI_MONITOR:-1}" != 0 ]; then
        monitor &
        monitor_pid=$!
        # shellcheck disable=SC2064
        trap "kill $monitor_pid 2>/dev/null" EXIT
    fi
    local limits=("${CI_ATTEMPT1_SECONDS:-1500}" "${CI_ATTEMPT2_SECONDS:-900}") status=0 n
    for n in 1 2; do
        timeout --kill-after=30 "${limits[$((n - 1))]}" bash "$SELF" --attempt
        status=$?
        [ "$status" -eq 0 ] && return 0
        if [ "$n" -eq 1 ] && stalled "$status"; then
            echo "::warning::The nextest step stalled (status $status after at most ${limits[0]} s); running it once more, which reuses what was built."
            tail -n 5 "$TRACE" 2>/dev/null
            rm -f "$FIRED"
            continue
        fi
        break
    done
    if stalled "$status"; then
        echo "::error::The nextest step stalled twice (status $status). The resource trace is in the job's artifacts; its last lines:"
        tail -n 10 "$TRACE" 2>/dev/null
    fi
    return "$status"
}

if [ "${1:-}" = "--attempt" ]; then
    attempt
    exit $?
fi
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    main
    exit $?
fi
