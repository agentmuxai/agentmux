#!/usr/bin/env bash
# bench-memory-scaling.sh — Measure memory with 0, 1, 2, 4 terminals
# Usage: ./bench-memory-scaling.sh <tauri|cef>
set -euo pipefail

MODE="${1:-}"
if [ "$MODE" != "tauri" ] && [ "$MODE" != "cef" ]; then
    echo "Usage: $0 <tauri|cef>"
    exit 1
fi

TAURI_DIR="$HOME/Desktop/agentmux-0.32.106-x64-portable"
CEF_DIR="$HOME/Desktop/agentmux-cef-0.32.110-x64-portable"

if [ "$MODE" = "tauri" ]; then
    APP_DIR="$TAURI_DIR"
    APP_EXE="./agentmux.exe"
    PATTERNS=("agentmux.exe" "agentmuxsrv-rs.x64.exe" "msedgewebview2.exe")
else
    APP_DIR="$CEF_DIR"
    APP_EXE="./agentmux-cef.exe"
    PATTERNS=("agentmux-cef.exe" "agentmuxsrv-rs.x64.exe")
fi

total_rss_kb() {
    local total=0
    for pattern in "${PATTERNS[@]}"; do
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

per_process_breakdown() {
    for pattern in "${PATTERNS[@]}"; do
        local count=0
        local total=0
        while IFS=, read -r img pid sess sessnum mem; do
            local kb
            kb=$(echo "$mem" | tr -d '"' | tr -d ' K' | tr -d ',')
            if [[ "$kb" =~ ^[0-9]+$ ]]; then
                total=$((total + kb))
                count=$((count + 1))
            fi
        done < <(tasklist //fi "IMAGENAME eq ${pattern}" //fo csv //nh 2>/dev/null | grep -iv "No tasks")
        if [ $total -gt 0 ]; then
            echo "    $pattern: $((total / 1024)) MB ($count processes)"
        fi
    done
}

echo "=== Memory Scaling Benchmark: $MODE ==="
echo ""

# Kill existing
taskkill //f //im agentmux.exe //t 2>/dev/null || true
taskkill //f //im agentmux-cef.exe //t 2>/dev/null || true
taskkill //f //im agentmuxsrv-rs.x64.exe //t 2>/dev/null || true
taskkill //f //im msedgewebview2.exe //t 2>/dev/null || true
sleep 3

# Launch
echo "Launching $MODE..."
cd "$APP_DIR"
$APP_EXE --use-alloy-style &>/dev/null &
cd - > /dev/null 2>&1 || true

# Wait for sidecar
echo -n "Waiting for sidecar..."
while ! tasklist //fi "IMAGENAME eq agentmuxsrv*" //fo csv //nh 2>/dev/null | grep -qi "agentmuxsrv"; do
    sleep 0.1
done
echo " ready"

# Wait for UI to fully load
echo "Waiting 10s for UI to stabilize..."
sleep 10

# Baseline snapshot
echo ""
echo "--- BASELINE (default terminal) ---"
baseline_kb=$(total_rss_kb)
echo "  Total RSS: $((baseline_kb / 1024)) MB"
per_process_breakdown

echo ""
echo "=== DONE ==="
echo ""
echo "To continue the benchmark:"
echo "  1. Open terminals manually (Ctrl+Shift+T or click +)"
echo "  2. After each terminal opens and settles (5s), run:"
echo "     bash -c 'source scripts/bench-memory-scaling.sh snapshot'"
echo ""
echo "Or run the scroll benchmark in a terminal pane:"
echo "  seq 1 100000"
echo ""
echo "Leave the app running. Take snapshots with:"
SNAP_CMD="for p in ${PATTERNS[*]}; do tasklist //fi \"IMAGENAME eq \$p\" //fo csv //nh 2>/dev/null | grep -iv 'No tasks'; done"
echo "  $SNAP_CMD"
