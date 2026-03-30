#!/usr/bin/env bash
# bench-compare.sh — Tauri vs CEF benchmark comparison
# Measures: disk size, startup time, baseline memory, per-terminal memory scaling
set -euo pipefail

TAURI_DIR="$HOME/Desktop/agentmux-0.32.106-x64-portable"
TAURI_EXE="$TAURI_DIR/agentmux.exe"
CEF_DIR="$HOME/Desktop/agentmux-cef-0.32.110-x64-portable"
CEF_EXE="$CEF_DIR/agentmux-cef.exe"
RUNS=3
SETTLE=5

[ -f "$TAURI_EXE" ] || { echo "ERROR: Tauri portable not found"; exit 1; }
[ -f "$CEF_EXE" ] || { echo "ERROR: CEF portable not found"; exit 1; }

echo "============================================"
echo " AgentMux Benchmark: Tauri vs CEF"
echo "============================================"
echo "Tauri: v0.32.106 (WebView2)"
echo "CEF:   v0.32.110 (CEF 146)"
echo "Runs:  $RUNS each"
echo "Date:  $(date)"
echo ""

# ─── Disk Size ────────────────────────────────────
echo "--- DISK SIZE ---"
tauri_mb=$(du -sm "$TAURI_DIR" | cut -f1)
cef_mb=$(du -sm "$CEF_DIR" | cut -f1)
tauri_files=$(find "$TAURI_DIR" -type f | wc -l | tr -d ' ')
cef_files=$(find "$CEF_DIR" -type f | wc -l | tr -d ' ')
echo "Tauri: ${tauri_mb} MB  (${tauri_files} files)"
echo "CEF:   ${cef_mb} MB  (${cef_files} files)"
echo "Delta: +$((cef_mb - tauri_mb)) MB"
echo ""

# ─── Helper: kill all app processes ───────────────
kill_all() {
    taskkill //f //im agentmux.exe //t 2>/dev/null || true
    taskkill //f //im agentmux-cef.exe //t 2>/dev/null || true
    taskkill //f //im agentmuxsrv-rs.x64.exe //t 2>/dev/null || true
    taskkill //f //im "agentmuxsrv-rs.x64.exe" //t 2>/dev/null || true
    taskkill //f //im msedgewebview2.exe //t 2>/dev/null || true
    sleep $SETTLE
}

# ─── Helper: total RSS of matching processes (KB) ─
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

# ─── Startup + Baseline Memory ────────────────────
bench_one() {
    local label="$1"
    local exe="$2"
    local dir="$3"
    shift 3
    local patterns=("$@")

    echo "--- ${label} STARTUP ---"

    local times=()
    local mems=()

    for i in $(seq 1 $RUNS); do
        # Start
        local t0
        t0=$(date +%s%N)

        cd "$dir"
        "$exe" --use-alloy-style &>/dev/null &
        local app_pid=$!
        cd - > /dev/null

        # Wait for sidecar
        local waited=0
        while ! tasklist //fi "IMAGENAME eq agentmuxsrv*" //fo csv //nh 2>/dev/null | grep -qi "agentmuxsrv"; do
            sleep 0.2
            waited=$((waited + 1))
            if [ $waited -gt 50 ]; then
                echo "  Run $i: TIMEOUT (sidecar never appeared)"
                break
            fi
        done

        # Let UI render
        sleep 5

        local t1
        t1=$(date +%s%N)
        local ms=$(( (t1 - t0) / 1000000 ))
        times+=($ms)

        # Memory snapshot (all processes)
        local rss_kb
        rss_kb=$(total_rss_kb "${patterns[@]}")
        local rss_mb=$((rss_kb / 1024))
        mems+=($rss_mb)

        echo "  Run $i: ${ms} ms startup, ${rss_mb} MB total RSS"

        # Per-process breakdown (last run only)
        if [ $i -eq $RUNS ]; then
            echo "  Process breakdown (run $i):"
            for p in "${patterns[@]}"; do
                local pmem
                pmem=$(total_rss_kb "$p")
                if [ "$pmem" -gt 0 ]; then
                    echo "    $p: $((pmem / 1024)) MB"
                fi
            done
        fi

        kill_all
    done

    # Median
    IFS=$'\n' sorted_t=($(sort -n <<<"${times[*]}")); unset IFS
    IFS=$'\n' sorted_m=($(sort -n <<<"${mems[*]}")); unset IFS
    local mid=$((RUNS / 2))
    echo "  MEDIAN: ${sorted_t[$mid]} ms startup, ${sorted_m[$mid]} MB RSS"
    echo ""

    # Export for results
    eval "${label}_MEDIAN_MS=${sorted_t[$mid]}"
    eval "${label}_MEDIAN_MB=${sorted_m[$mid]}"
}

# ─── Kill existing ────────────────────────────────
echo "Cleaning up existing processes..."
kill_all
echo ""

# ─── Run Tauri benchmarks ─────────────────────────
bench_one "TAURI" "$TAURI_EXE" "$TAURI_DIR" \
    "agentmux.exe" "agentmuxsrv-rs.x64.exe" "msedgewebview2.exe"

# ─── Run CEF benchmarks ──────────────────────────
bench_one "CEF" "$CEF_EXE" "$CEF_DIR" \
    "agentmux-cef.exe" "agentmuxsrv-rs.x64.exe"

# ─── Summary ─────────────────────────────────────
echo "============================================"
echo " RESULTS SUMMARY"
echo "============================================"
echo ""
printf "%-20s %12s %12s %12s\n" "Metric" "Tauri" "CEF" "Delta"
printf "%-20s %12s %12s %12s\n" "----" "----" "----" "----"
printf "%-20s %10s MB %10s MB %+10s MB\n" "Disk size" "$tauri_mb" "$cef_mb" "$((cef_mb - tauri_mb))"
printf "%-20s %12s %12s %12s\n" "File count" "$tauri_files" "$cef_files" "+$((cef_files - tauri_files))"
printf "%-20s %9s ms %9s ms %+9s ms\n" "Startup (median)" "$TAURI_MEDIAN_MS" "$CEF_MEDIAN_MS" "$((CEF_MEDIAN_MS - TAURI_MEDIAN_MS))"
printf "%-20s %10s MB %10s MB %+10s MB\n" "Baseline RSS" "$TAURI_MEDIAN_MB" "$CEF_MEDIAN_MB" "$((CEF_MEDIAN_MB - TAURI_MEDIAN_MB))"
echo ""
echo "============================================"
