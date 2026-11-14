#!/usr/bin/env bash
# A memory line in the build log every N seconds while a long build step runs (macOS), so a slow step can be read
# against memory pressure. Printing only: it reads vm_stat, sysctl, memory_pressure and ps (executable names, never
# arguments or the environment), every probe may fail, and a failed probe prints "?" for what it could not read.
#
# Sourced by a packaging script (scripts/package-macos.sh):
#   . "$REPO_ROOT/scripts/mem-sampler.sh"
#   mem_sampler_start        # before the step
#   task build:backend
#   mem_sampler_stop         # right after it
# Both functions always succeed. The sampler ends by itself when the script that started it is gone, and after
# MAX_SAMPLES lines at the most.
#
# Run directly: bash scripts/mem-sampler.sh [interval_seconds] [parent_pid] [max_samples]

# One line: free, active, wired and compressed memory, swap in use, and the biggest process by resident size.
mem_line() {
    local vm page free spec act wired comp pct swap top
    vm="$(vm_stat 2> /dev/null)" || vm=""
    page="$(printf '%s\n' "$vm" | sed -n 's/.*page size of \([0-9][0-9]*\) bytes.*/\1/p' | head -n 1)"
    page="${page:-16384}"
    pages() { printf '%s\n' "$vm" | sed -n "s/^$1: *\\([0-9][0-9]*\\)\\./\\1/p" | head -n 1; }
    gb() { if [ -n "${1:-}" ]; then awk -v p="$1" -v s="$page" 'BEGIN { printf "%.1fG", p * s / 1073741824 }'; else printf '?'; fi; }
    free="$(pages 'Pages free')"
    spec="$(pages 'Pages speculative')"
    act="$(pages 'Pages active')"
    wired="$(pages 'Pages wired down')"
    comp="$(pages 'Pages occupied by compressor')"
    if [ -n "$free" ] && [ -n "$spec" ]; then free=$((free + spec)); fi
    pct="$(memory_pressure 2> /dev/null | sed -n 's/.*free percentage: *\([0-9][0-9]*\)%.*/\1/p' | head -n 1)"
    swap="$(sysctl -n vm.swapusage 2> /dev/null | sed -n 's/.*used = *\([0-9.][0-9.]*[A-Za-z]*\).*/\1/p' | head -n 1)"
    top="$(ps -Ao rss=,comm= -m 2> /dev/null | head -n 1 | awk '{ rss = $1; $1 = ""; sub(/^ +/, ""); n = split($0, p, "/"); printf "%s %dMB", p[n], rss / 1024 }')"
    echo "mem: t=${SECONDS}s free=$(gb "$free")(${pct:-?}%) active=$(gb "$act") wired=$(gb "$wired") compressed=$(gb "$comp") swap_used=${swap:-?} top=${top:-?}"
}

mem_sampler_start() {
    MEM_SAMPLER_PID=""
    bash "${BASH_SOURCE[0]}" "${MEM_SAMPLER_INTERVAL:-30}" "$$" "${MEM_SAMPLER_MAX:-600}" &
    MEM_SAMPLER_PID=$!
    return 0
}

mem_sampler_stop() {
    if [ -n "${MEM_SAMPLER_PID:-}" ]; then
        kill "$MEM_SAMPLER_PID" 2> /dev/null || true
        wait "$MEM_SAMPLER_PID" 2> /dev/null || true
        MEM_SAMPLER_PID=""
    fi
    return 0
}

# When run, not sourced: print a line every interval until the parent is gone or the count is reached.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    set +e
    interval="${1:-30}"
    parent="${2:-$PPID}"
    max="${3:-600}"
    trap 'exit 0' TERM INT
    i=0
    while [ "$i" -lt "$max" ]; do
        kill -0 "$parent" 2> /dev/null || exit 0
        mem_line 2> /dev/null || echo "mem: unreadable"
        i=$((i + 1))
        sleep "$interval" &
        wait $!
    done
    exit 0
fi
