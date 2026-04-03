#!/usr/bin/env bash
# AgentMux Performance Benchmarking Script (Unix)
# Measures startup time, memory usage, and bundle size
# Usage: ./measure-performance.sh [runs]

set -euo pipefail

RUNS="${1:-5}"
RESULTS_FILE="benchmark-results.txt"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

print_header() {
    echo ""
    echo -e "${CYAN}========================================${NC}"
    echo -e "${CYAN}  $1${NC}"
    echo -e "${CYAN}========================================${NC}"
    echo ""
}

detect_platform() {
    case "$(uname -s)" in
        Darwin*)
            echo "macos"
            ;;
        Linux*)
            echo "linux"
            ;;
        *)
            echo "unknown"
            ;;
    esac
}

find_app_binary() {
    local platform=$1

    case "$platform" in
        macos)
            if [ -d "src-tauri/target/release/bundle/macos/AgentMux.app" ]; then
                echo "src-tauri/target/release/bundle/macos/AgentMux.app/Contents/MacOS/AgentMux"
            else
                echo "src-tauri/target/release/agentmux"
            fi
            ;;
        linux)
            echo "src-tauri/target/release/agentmux"
            ;;
        *)
            echo ""
            ;;
    esac
}

measure_startup_time() {
    local app_path=$1
    local iterations=$2

    print_header "Startup Time Measurement"
    echo -e "${YELLOW}Running $iterations iterations...${NC}"

    local total=0
    local times=()

    for ((i=1; i<=iterations; i++)); do
        echo -n "Run $i/$iterations... "

        # Measure startup time
        local start=$(date +%s%3N)
        timeout 30s "$app_path" &
        local pid=$!

        # Wait for process to be responsive (simple check)
        sleep 2

        local end=$(date +%s%3N)
        local elapsed=$((end - start))

        # Kill the process
        kill -9 $pid 2>/dev/null || true
        sleep 0.5

        times+=("$elapsed")
        total=$((total + elapsed))

        echo -e "${GREEN}${elapsed}ms${NC}"
    done

    local avg=$((total / iterations))

    # Calculate median (simple approach)
    IFS=$'\n' sorted=($(sort -n <<<"${times[*]}"))
    local median=${sorted[$((iterations / 2))]}

    # Calculate min/max
    local min=${sorted[0]}
    local max=${sorted[-1]}

    echo ""
    echo -e "${CYAN}Results:${NC}"
    echo -e "  Average: ${avg}ms"
    echo -e "  Median:  ${median}ms"
    echo -e "  Min:     ${GREEN}${min}ms${NC}"
    echo -e "  Max:     ${YELLOW}${max}ms${NC}"

    echo "startup_avg=$avg" >> "$RESULTS_FILE"
    echo "startup_median=$median" >> "$RESULTS_FILE"
}

measure_memory_usage() {
    local app_path=$1
    local platform=$2

    print_header "Memory Usage Measurement"

    echo -e "${YELLOW}Starting application...${NC}"
    "$app_path" &
    local pid=$!

    # Wait for startup
    sleep 5

    # Measure memory
    local memory
    case "$platform" in
        macos)
            # macOS: use ps
            memory=$(ps -o rss= -p $pid | awk '{print $1/1024}')
            ;;
        linux)
            # Linux: use /proc
            memory=$(awk '/VmRSS/ {print $2/1024}' /proc/$pid/status)
            ;;
    esac

    echo -e "Idle Memory: ${GREEN}$(printf "%.2f" $memory) MB${NC}"

    # Kill process
    kill -9 $pid 2>/dev/null || true

    echo "memory_idle=$memory" >> "$RESULTS_FILE"
}

measure_bundle_size() {
    local platform=$1

    print_header "Bundle Size Measurement"

    case "$platform" in
        macos)
            if [ -d "src-tauri/target/release/bundle/macos/AgentMux.app" ]; then
                local app_size=$(du -sm "src-tauri/target/release/bundle/macos/AgentMux.app" | cut -f1)
                echo -e "AgentMux.app: ${GREEN}${app_size} MB${NC}"
            fi

            # Check for DMG
            local dmg=$(find src-tauri/target/release/bundle/dmg -name "*.dmg" 2>/dev/null | head -1)
            if [ -n "$dmg" ]; then
                local dmg_size=$(stat -f%z "$dmg" | awk '{print $1/1024/1024}')
                echo -e "Installer (.dmg): ${GREEN}$(printf "%.2f" $dmg_size) MB${NC}"
            fi
            ;;

        linux)
            local bin_path="src-tauri/target/release/agentmux"
            if [ -f "$bin_path" ]; then
                local bin_size=$(stat -c%s "$bin_path" | awk '{print $1/1024/1024}')
                echo -e "agentmux binary: ${GREEN}$(printf "%.2f" $bin_size) MB${NC}"
            fi

            # Check for AppImage
            local appimage=$(find src-tauri/target/release/bundle -name "*.AppImage" 2>/dev/null | head -1)
            if [ -n "$appimage" ]; then
                local appimage_size=$(stat -c%s "$appimage" | awk '{print $1/1024/1024}')
                echo -e "Installer (.AppImage): ${GREEN}$(printf "%.2f" $appimage_size) MB${NC}"
            fi
            ;;
    esac
}

# Main execution
cat << "EOF"

 ██╗    ██╗ █████╗ ██╗   ██╗███████╗███╗   ███╗██╗   ██╗██╗  ██╗
 ██║    ██║██╔══██╗██║   ██║██╔════╝████╗ ████║██║   ██║╚██╗██╔╝
 ██║ █╗ ██║███████║██║   ██║█████╗  ██╔████╔██║██║   ██║ ╚███╔╝
 ██║███╗██║██╔══██║╚██╗ ██╔╝██╔══╝  ██║╚██╔╝██║██║   ██║ ██╔██╗
 ╚███╔███╔╝██║  ██║ ╚████╔╝ ███████╗██║ ╚═╝ ██║╚██████╔╝██╔╝ ██╗
  ╚══╝╚══╝ ╚═╝  ╚═╝  ╚═══╝  ╚══════╝╚═╝     ╚═╝ ╚═════╝ ╚═╝  ╚═╝

           Performance Benchmarking Tool v1.0

EOF

PLATFORM=$(detect_platform)
echo -e "${CYAN}Platform: $PLATFORM${NC}"

APP_PATH=$(find_app_binary "$PLATFORM")

if [ -z "$APP_PATH" ] || [ ! -f "$APP_PATH" ]; then
    echo -e "${RED}ERROR: Application not found${NC}"
    echo -e "${YELLOW}Build the release version first: task package${NC}"
    exit 1
fi

echo -e "${CYAN}App binary: $APP_PATH${NC}"

# Clear results file
> "$RESULTS_FILE"

# Run benchmarks
measure_startup_time "$APP_PATH" "$RUNS"
measure_memory_usage "$APP_PATH" "$PLATFORM"
measure_bundle_size "$PLATFORM"

print_header "Benchmark Complete"
echo -e "Results saved to: ${GREEN}$RESULTS_FILE${NC}"
echo ""
