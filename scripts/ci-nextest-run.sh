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
# 2026-11-14: the stall is the srv test compile, and the watchdog above could not catch it. Its last sample before a
# freeze (PR 7232, run 43538724481) read 724 MB available with 11.5 GB of swap free, nowhere near the trigger, and then
# the sampler was starved too: no sample for 20 minutes 49 seconds, until the 25-minute limit killed rustc and the
# machine was fine within 5 seconds. Every healthy run sits at the same edge (26 runs: 338 to 530 MB available at the
# lowest, 15.2 to 15.6 GB used of ~15.6, 2.7 to 5.1 GB swapped, memory pressure peaks of 9 to 69), so no threshold on
# those readings tells a healthy run from the one that is about to freeze. What changed:
#
#   4. The build runs with CARGO_BUILD_JOBS=2 (CI_CARGO_JOBS): the srv test compile holds about 10 GB when type-checking
#      ends and takes 4+ GB more in the parallel LLVM phase, with ~19 GB wanted in all (14.9 GB resident + the swap) on a
#      16 GB runner. Two LLVM threads instead of four is the cut, a candidate until the trace of the first runs shows
#      the new peak in the `top=` and `demand=` fields.
#   5. The sampler starts no process (it reads /proc with the shell's own builtins), so a machine that is paging has
#      less to page in to keep it running, and a gap longer than GAP_SECONDS between two samples on a machine with
#      under GAP_MEM_MB free counts as three strikes at once: the sampler found it was starved, which is the stall.
#      Healthy runs never leave more than 8 seconds between samples. The trace says when it was starved.
#
#   6. The attempt runs in its own cgroup with a memory ceiling (CI_MEM_MAX, 14 GB of the runner's 15.6, and CI_SWAP_MAX,
#      7 GB of swap): a build that wants more pages inside the ceiling, or is killed by the kernel when it passes it, and
#      the rest of the machine (the runner's agent, the sampler, the step's own timeout) keeps its memory and keeps
#      answering. That is the cut that does not depend on guessing a threshold: the freeze needs the whole machine to be
#      short of memory. It needs sudo and systemd (the runner has both); when either is missing the step says so and runs
#      without the ceiling (CI_CEILING=0 turns it off). A process the ceiling kills is reported by the sampler and
#      counts as a stall, so the attempt is retried once. The trace lines gain `scope=` (the attempt's memory) and
#      `oom_kill=` (what the kernel killed in it).
#
# CI_ATTEMPT_CMD replaces the cargo commands, CI_MONITOR=0 turns the trace and watchdog off, CI_CEILING=0 turns the
# ceiling off: all three are for scripts/ci-nextest-run.test.sh.

set -uo pipefail

SELF="${BASH_SOURCE[0]}"
TRACE="${CI_TRACE_FILE:-${RUNNER_TEMP:-/tmp}/resource-trace.log}"
BUILD_LOG="${CI_BUILD_LOG:-${RUNNER_TEMP:-/tmp}/nextest-build.log}"
FIRED="$TRACE.watchdog"
CARGO_JOBS="${CI_CARGO_JOBS:-2}"
CGROUP_ROOT="${CI_CGROUP_ROOT:-/sys/fs/cgroup}"
CONFINE_BASE=()
CEILING_RAM="${CI_MEM_MAX:-14G}"
CEILING_SWAP="${CI_SWAP_MAX:-7G}"
GAP_SECONDS=20
GAP_MEM_MB=2048
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

# The sampler below starts no process: every figure comes from /proc through the shell's own builtins (a read loop, printf's
# time format, a read with a timeout for the sleep), because on a machine that is paging each new process has to be paged in
# first, and the old one (ps, awk, free, df, date, tee) went 20 minutes without a sample.

# The kernel's memory pressure: the share of the last 10 s in which every task was stalled on memory ("full avg10").
# Sets PSI ("na" without it).
memory_psi() {
    local kind a rest
    PSI=na
    while read -r kind a rest; do
        [ "$kind" = full ] && PSI=${a#avg10=}
    done 2>/dev/null < /proc/pressure/memory
    return 0
}

# Whether an attempt's exit status means it stalled rather than failed: timed out (124), killed (137), terminated
# (143, the runner's shutdown signal), or the watchdog killed something.
stalled() {
    case "$1" in 124 | 137 | 143) return 0 ;; esac
    [ -e "$FIRED" ]
}

# Memory in MB from /proc/meminfo: MEM_AVAIL_MB, MEM_USED_MB (total less available, which is what free shows as used),
# SWAP_FREE_MB, SWAP_USED_MB.
read_mem() {
    local key val rest total=0 avail=0 swap_total=0 swap_free=0
    while read -r key val rest; do
        case "$key" in
            MemTotal:) total=$val ;;
            MemAvailable:) avail=$val ;;
            SwapTotal:) swap_total=$val ;;
            SwapFree:) swap_free=$val ;;
        esac
    done < /proc/meminfo
    MEM_AVAIL_MB=$((avail / 1024))
    MEM_USED_MB=$(((total - avail) / 1024))
    SWAP_FREE_MB=$((swap_free / 1024))
    SWAP_USED_MB=$(((swap_total - swap_free) / 1024))
}

# The process holding the most memory: TOP_PID, TOP_COMM and TOP_MB. With an argument, the runner's own processes and the
# system's are left out (what the watchdog may kill).
top_process() {
    local d rss comm best=0
    TOP_PID="" TOP_COMM="-" TOP_MB=0
    for d in /proc/[0-9]*; do
        { read -r _ rss _; } 2>/dev/null < "$d/statm" || continue
        [ "${rss:-0}" -gt "$best" ] || continue
        { read -r comm; } 2>/dev/null < "$d/comm" || continue
        if [ -n "${1:-}" ]; then
            case "$comm" in Runner* | dotnet* | systemd* | sshd* | bash | ps | awk) continue ;; esac
        fi
        best=$rss TOP_PID=${d#/proc/} TOP_COMM=$comm TOP_MB=$((rss * 4 / 1024))
    done
}

# The attempt's cgroup: the attempt writes its own path to $TRACE.cgroup when it starts. Sets SCOPE_FIELDS (" scope=…MB
# oom_kill=N", or nothing before the first attempt or once its cgroup is gone) and SCOPE_OOM, the kernel's count of
# processes it killed there.
read_scope() {
    local path cur key val
    SCOPE_FIELDS="" SCOPE_OOM=0
    [ -r "$TRACE.cgroup" ] || return 0
    read -r path < "$TRACE.cgroup" || return 0
    { read -r cur; } 2>/dev/null < "$CGROUP_ROOT$path/memory.current" || return 0
    while read -r key val; do
        [ "$key" = oom_kill ] && SCOPE_OOM=$val
    done 2>/dev/null < "$CGROUP_ROOT$path/memory.events"
    SCOPE_FIELDS=" scope=$((cur / 1048576))MB oom_kill=$SCOPE_OOM"
}

# Sets SAMPLE: one trace line. `demand` is memory used plus swap used: what the build wants, past what the machine has.
sample() {
    local now
    TZ=UTC printf -v now '%(%H:%M:%S)T' -1
    read_mem
    memory_psi
    top_process
    read_scope
    SAMPLE="[resources] $now mem_avail=${MEM_AVAIL_MB}MB swap_free=${SWAP_FREE_MB}MB mem_used=${MEM_USED_MB}MB swap_used=${SWAP_USED_MB}MB demand=$((MEM_USED_MB + SWAP_USED_MB))MB top=${TOP_COMM}:${TOP_MB}MB psi_full10=$PSI$SCOPE_FIELDS"
}

# Kill the process that holds the most memory, leaving the runner's own processes alone, and say what it was. The kill comes
# first: the report below starts processes, which a paging machine may take a while to start.
kill_biggest() {
    top_process runner-safe
    [ -n "$TOP_PID" ] || return 0
    : > "$FIRED"
    kill -KILL "$TOP_PID" 2>/dev/null || true
    local line="[watchdog] memory is nearly gone; killed pid $TOP_PID ($TOP_COMM, $TOP_MB MB). Biggest processes:"
    echo "$line"
    echo "$line" >> "$TRACE"
    ps -eo pid=,rss=,etimes=,args= --sort=-rss | head -8 | cut -c1-200 | tee -a "$TRACE"
}

# One reading's verdict: STRIKES, from the strikes so far, the readings (MB) and the seconds since the last sample. Three
# strikes in a row, or one sampler that was starved for GAP_SECONDS on a machine with under GAP_MEM_MB free, fire the watchdog.
judge() {
    local strikes=$1 mem=$2 swap=$3 gap=$4
    if pressure_high "$mem" "$swap"; then strikes=$((strikes + 1)); else strikes=0; fi
    if [ "$gap" -ge "$GAP_SECONDS" ] && [ "$mem" -lt "$GAP_MEM_MB" ]; then strikes=3; fi
    STRIKES=$strikes
}

monitor() {
    local strikes=0 now last=0 gap=0 fd="" tick="$TRACE.tick" oom_seen=0
    mkfifo "$tick" 2>/dev/null && exec {fd}<>"$tick"
    while true; do
        printf -v now '%(%s)T' -1
        [ "$last" -gt 0 ] && gap=$((now - last))
        last=$now
        sample
        echo "$SAMPLE"
        echo "$SAMPLE" >> "$TRACE"
        if [ "$gap" -ge "$GAP_SECONDS" ]; then
            echo "[watchdog] the sampler was starved for ${gap}s: the machine was paging"
            echo "[watchdog] the sampler was starved for ${gap}s: the machine was paging" >> "$TRACE"
        fi
        if [ "$SCOPE_OOM" -gt "$oom_seen" ]; then
            echo "[watchdog] the memory ceiling killed $((SCOPE_OOM - oom_seen)) process(es) in the build's cgroup"
            echo "[watchdog] the memory ceiling killed $((SCOPE_OOM - oom_seen)) process(es) in the build's cgroup" >> "$TRACE"
            : > "$FIRED"
        fi
        oom_seen=$SCOPE_OOM
        judge "$strikes" "$MEM_AVAIL_MB" "$SWAP_FREE_MB" "$gap"
        strikes=$STRIKES
        if [ "$strikes" -ge 3 ]; then
            kill_biggest 2>&1
            strikes=0
        fi
        gap=0
        if [ -n "$fd" ]; then read -r -t 5 -u "$fd" || true; else sleep 5; fi
    done
}

# One attempt: the build and the tests. A failure of any command is the attempt's status.
attempt() {
    set -o pipefail
    local line=""
    read -r line < /proc/self/cgroup 2>/dev/null && echo "${line#*::}" > "$TRACE.cgroup"
    if [ -n "${CI_ATTEMPT_CMD:-}" ]; then
        bash -c "$CI_ATTEMPT_CMD"
        return $?
    fi
    # Two LLVM threads in the srv test compile instead of four: see 4. in the header.
    export CARGO_BUILD_JOBS="$CARGO_JOBS"
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

# The command prefix that runs the attempt in its own cgroup with the memory ceiling (sets CONFINE_BASE, an array; empty
# when the ceiling is off or cannot be had) and says which. A trial scope first: no sudo, no systemd, or a scope that
# will not start leaves the step as it was, with the reason in the log.
confine_probe() {
    CONFINE_BASE=()
    if [ "${CI_CEILING:-1}" = 0 ]; then
        echo "memory ceiling: off (CI_CEILING=0)"
        return 0
    fi
    # sudo resets the environment, and `-E` is not honoured by every sudo (sudo-rs ignores it), so the names of the variables
    # to keep are listed one by one; PATH and HOME are set for the build itself (sudo's secure_path replaces PATH).
    export CI_NEXTEST_PROBE=1
    local names
    names=$(compgen -e | grep -E '^[A-Za-z_][A-Za-z0-9_]*$' | grep -vE '^(PATH|HOME|SUDO_.*)$' | paste -sd, -)
    local cmd=(sudo -n "--preserve-env=$names" systemd-run --scope --quiet --collect "--uid=$(id -u)" "--gid=$(id -g)" -p "MemoryMax=$CEILING_RAM" -p "MemorySwapMax=$CEILING_SWAP" "--setenv=PATH=$PATH" "--setenv=HOME=$HOME")
    local why status
    # The trial also checks that the environment arrives, which the build cannot do without.
    if why=$("${cmd[@]}" -- bash -c '[ "${CI_NEXTEST_PROBE:-}" = 1 ] || { echo "the environment did not arrive in the scope"; exit 3; }' 2>&1); then
        CONFINE_BASE=("${cmd[@]}")
        echo "memory ceiling: the build runs in its own cgroup, $CEILING_RAM of memory and $CEILING_SWAP of swap"
    else
        status=$?
        echo "::warning::memory ceiling not available, the build runs without it: ${why:-no output} (exit $status)"
    fi
}

# Whatever an attempt left in its cgroup is killed, so a retry never starts beside the remains of the first attempt (sudo
# runs the build in a session of its own, which the timeout's signal to its process group does not reach).
confine_cleanup() {
    [ "${#CONFINE_BASE[@]}" -gt 0 ] || return 0
    sudo -n systemctl kill --signal=KILL "ci-nextest-$$-$1.scope" > /dev/null 2>&1 || true
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
        trap "kill $monitor_pid 2>/dev/null; rm -f '$TRACE.tick' '$TRACE.cgroup'" EXIT
    fi
    confine_probe
    local limits=("${CI_ATTEMPT1_SECONDS:-1500}" "${CI_ATTEMPT2_SECONDS:-900}") status=0 n
    for n in 1 2; do
        local confine=()
        [ "${#CONFINE_BASE[@]}" -gt 0 ] && confine=("${CONFINE_BASE[@]}" "--unit=ci-nextest-$$-$n" --)
        timeout --kill-after=30 "${limits[$((n - 1))]}" "${confine[@]}" bash "$SELF" --attempt
        status=$?
        confine_cleanup "$n"
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
