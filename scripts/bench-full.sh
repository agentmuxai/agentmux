#!/usr/bin/env bash
# bench-full.sh — Full Tauri vs CEF benchmark
# Measures: startup, baseline memory, process breakdown
set -euo pipefail

TAURI_DIR="$HOME/Desktop/agentmux-0.32.106-x64-portable"
CEF_DIR="$HOME/Desktop/agentmux-cef-0.32.110-x64-portable"
RUNS=3
SETTLE=5

total_rss_kb() {
    local total=0
    for pattern in "$@"; do
        while IFS=, read -r img pid sess sessnum mem; do
            local kb
            kb=$(echo "$mem" | tr -d '"' | tr -d ' K' | tr -d ',')
            if [[ "$kb" =~ ^[0-9]+$ ]]; then
                total=$((total + kb))
            fi
        done < <(tasklist //fi "IMAGENAME eq ${pattern}" //fo csv //nh 2>/dev/null | grep -iv "No tasks")
    done
    echo "$total"
}

process_count() {
    local count=0
    for pattern in "$@"; do
        local c
        c=$(tasklist //fi "IMAGENAME eq ${pattern}" //fo csv //nh 2>/dev/null | grep -iv "No tasks" | wc -l)
        count=$((count + c))
    done
    echo "$count"
}

breakdown() {
    for pattern in "$@"; do
        local total=0
        local count=0
        while IFS=, read -r img pid sess sessnum mem; do
            local kb
            kb=$(echo "$mem" | tr -d '"' | tr -d ' K' | tr -d ',')
            if [[ "$kb" =~ ^[0-9]+$ ]]; then
                total=$((total + kb))
                count=$((count + 1))
            fi
        done < <(tasklist //fi "IMAGENAME eq ${pattern}" //fo csv //nh 2>/dev/null | grep -iv "No tasks")
        if [ $total -gt 0 ]; then
            printf "    %-30s %4d MB  (%d procs)\n" "$pattern" "$((total / 1024))" "$count"
        fi
    done
}

kill_all() {
    taskkill //f //im agentmux.exe //t 2>/dev/null || true
    taskkill //f //im agentmux-cef.exe //t 2>/dev/null || true
    taskkill //f //im agentmuxsrv-rs.x64.exe //t 2>/dev/null || true
    taskkill //f //im msedgewebview2.exe //t 2>/dev/null || true
    sleep $SETTLE
}

bench_build() {
    local label="$1"
    local dir="$2"
    local exe="$3"
    shift 3
    local patterns=("$@")

    local startup_times=()
    local memory_values=()
    local proc_counts=()

    echo "============================================"
    echo " $label"
    echo "============================================"

    for run in $(seq 1 $RUNS); do
        # Startup timing
        local t0=$(date +%s%N)
        cd "$dir" && $exe --use-alloy-style &>/dev/null &
        cd - > /dev/null 2>&1 || true

        while ! tasklist //fi "IMAGENAME eq agentmuxsrv*" //fo csv //nh 2>/dev/null | grep -qi "agentmuxsrv"; do
            sleep 0.05
        done
        local t1=$(date +%s%N)
        local startup_ms=$(( (t1 - t0) / 1000000 ))
        startup_times+=($startup_ms)

        # Wait for UI to fully load and stabilize
        sleep 8

        # Memory
        local rss=$(total_rss_kb "${patterns[@]}")
        local rss_mb=$((rss / 1024))
        memory_values+=($rss_mb)

        local procs=$(process_count "${patterns[@]}")
        proc_counts+=($procs)

        echo "  Run $run: startup=${startup_ms}ms  memory=${rss_mb}MB  processes=${procs}"

        # Breakdown on last run
        if [ $run -eq $RUNS ]; then
            echo "  Breakdown:"
            breakdown "${patterns[@]}"
        fi

        kill_all
    done

    # Medians
    IFS=$'\n' sorted_s=($(sort -n <<<"${startup_times[*]}")); unset IFS
    IFS=$'\n' sorted_m=($(sort -n <<<"${memory_values[*]}")); unset IFS
    IFS=$'\n' sorted_p=($(sort -n <<<"${proc_counts[*]}")); unset IFS
    local mid=$((RUNS / 2))

    echo ""
    echo "  MEDIAN: startup=${sorted_s[$mid]}ms  memory=${sorted_m[$mid]}MB  processes=${sorted_p[$mid]}"
    echo ""

    # Export
    eval "${label}_STARTUP=${sorted_s[$mid]}"
    eval "${label}_MEMORY=${sorted_m[$mid]}"
    eval "${label}_PROCS=${sorted_p[$mid]}"
}

echo ""
echo "Cleaning up..."
kill_all

# Tauri
bench_build "TAURI" "$TAURI_DIR" "./agentmux.exe" \
    "agentmux.exe" "agentmuxsrv-rs.x64.exe" "msedgewebview2.exe"

# CEF
bench_build "CEF" "$CEF_DIR" "./agentmux-cef.exe" \
    "agentmux-cef.exe" "agentmuxsrv-rs.x64.exe"

# Disk
tauri_mb=$(du -sm "$TAURI_DIR" | cut -f1)
cef_mb=$(du -sm "$CEF_DIR" | cut -f1)

# Summary
echo "============================================"
echo " FINAL RESULTS"
echo "============================================"
echo ""
printf "%-25s %15s %15s %15s\n" "Metric" "Tauri" "CEF" "Delta"
printf "%-25s %15s %15s %15s\n" "---" "---" "---" "---"
printf "%-25s %12s MB %12s MB %+12s MB\n" "Disk size" "$tauri_mb" "$cef_mb" "$((cef_mb - tauri_mb))"
printf "%-25s %12s ms %12s ms %+12s ms\n" "Startup (to sidecar)" "$TAURI_STARTUP" "$CEF_STARTUP" "$((CEF_STARTUP - TAURI_STARTUP))"
printf "%-25s %12s MB %12s MB %+12s MB\n" "Baseline RSS (1 term)" "$TAURI_MEMORY" "$CEF_MEMORY" "$((CEF_MEMORY - TAURI_MEMORY))"
printf "%-25s %15s %15s %+15s\n" "Process count" "$TAURI_PROCS" "$CEF_PROCS" "$((CEF_PROCS - TAURI_PROCS))"
echo ""

# CDP check for CEF scroll FPS
echo "============================================"
echo " SCROLL FPS (CEF only — CDP on port 9222)"
echo "============================================"
echo ""
echo "Launching CEF for scroll test..."
cd "$CEF_DIR" && ./agentmux-cef.exe --use-alloy-style &>/dev/null &
cd - > /dev/null 2>&1 || true
sleep 10

# Check if CDP is available
if curl -s "http://localhost:9222/json" > /dev/null 2>&1; then
    echo "CDP available on port 9222"
    targets=$(curl -s "http://localhost:9222/json")
    echo "Targets: $(echo "$targets" | grep -c '"type"')"
    echo ""
    echo "To measure scroll FPS manually:"
    echo "  1. In the AgentMux terminal, run: seq 1 100000"
    echo "  2. Connect to CDP and use Performance.getMetrics"
    echo ""

    # Try to get basic performance metrics
    ws_url=$(echo "$targets" | grep -o '"webSocketDebuggerUrl":"[^"]*"' | head -1 | cut -d'"' -f4)
    if [ -n "$ws_url" ]; then
        echo "CDP WebSocket: $ws_url"
    fi
else
    echo "CDP not available on port 9222"
fi

echo ""
echo "CEF is running for manual scroll/input testing."
echo "Kill when done: taskkill //f //im agentmux-cef.exe //t"
