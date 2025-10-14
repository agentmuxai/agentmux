# Shared helper functions for BATS tests

# Clean up any running agentmux instances
cleanup_agentmux() {
    # Kill any running agentmux processes
    pkill -9 agentmux 2>/dev/null || true

    # Remove lock file
    if [ -f "$HOME/.agentmux/desktop.lock" ]; then
        rm -f "$HOME/.agentmux/desktop.lock"
    fi

    # Wait for cleanup to complete
    sleep 1
}

# Wait for port to be available
wait_for_port() {
    local port=$1
    local timeout=${2:-30}
    local elapsed=0

    while [ $elapsed -lt $timeout ]; do
        if nc -z localhost $port 2>/dev/null; then
            return 0
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done

    return 1
}

# Check if process is running
is_process_running() {
    local pid=$1
    ps -p $pid > /dev/null 2>&1
}

# Get the agentmux binary path
get_agentmux_bin() {
    # Try to find the binary in release or debug build
    if [ -f "$BATS_TEST_DIRNAME/../../src-tauri/target/release/agentmux" ]; then
        echo "$BATS_TEST_DIRNAME/../../src-tauri/target/release/agentmux"
    elif [ -f "$BATS_TEST_DIRNAME/../../src-tauri/target/release/agentmux.exe" ]; then
        echo "$BATS_TEST_DIRNAME/../../src-tauri/target/release/agentmux.exe"
    elif [ -f "$BATS_TEST_DIRNAME/../../src-tauri/target/debug/agentmux" ]; then
        echo "$BATS_TEST_DIRNAME/../../src-tauri/target/debug/agentmux"
    elif [ -f "$BATS_TEST_DIRNAME/../../src-tauri/target/debug/agentmux.exe" ]; then
        echo "$BATS_TEST_DIRNAME/../../src-tauri/target/debug/agentmux.exe"
    else
        echo ""
        return 1
    fi
}

# Wait for GUI instance to start
wait_for_gui() {
    local timeout=${1:-10}
    local elapsed=0

    while [ $elapsed -lt $timeout ]; do
        if [ -f "$HOME/.agentmux/desktop.lock" ]; then
            return 0
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done

    return 1
}

# Read lock file and extract PID
get_lock_pid() {
    if [ -f "$HOME/.agentmux/desktop.lock" ]; then
        # Extract PID from JSON lock file
        grep -o '"pid":[0-9]*' "$HOME/.agentmux/desktop.lock" | grep -o '[0-9]*'
    else
        echo ""
        return 1
    fi
}

# Read lock file and extract IPC port
get_lock_port() {
    if [ -f "$HOME/.agentmux/desktop.lock" ]; then
        # Extract port from JSON lock file
        grep -o '"ipc_port":[0-9]*' "$HOME/.agentmux/desktop.lock" | grep -o '[0-9]*'
    else
        echo ""
        return 1
    fi
}
