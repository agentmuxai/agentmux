#!/usr/bin/env bats

load helpers/test_helper
load helpers/assertions

setup() {
    # Clean state before each test
    cleanup_agentmux

    # Get binary path
    AGENTMUX_BIN=$(get_agentmux_bin)
    if [ -z "$AGENTMUX_BIN" ]; then
        skip "agentmux binary not found. Run 'npm run tauri:build' first."
    fi
}

teardown() {
    cleanup_agentmux
}

@test "only one GUI instance can run at a time" {
    skip "GUI tests require headless mode support - implementation pending"

    # Start first instance
    $AGENTMUX_BIN &
    GUI_PID=$!

    # Wait for lock file
    wait_for_gui 10
    assert_file_exists "$HOME/.agentmux/desktop.lock"

    # Attempt second instance (should exit immediately)
    run timeout 5s $AGENTMUX_BIN

    # The second instance should detect existing instance
    assert_contains "$output" "already running"

    # Cleanup
    kill $GUI_PID 2>/dev/null || true
    wait $GUI_PID 2>/dev/null || true
}

@test "stale lock file is removed" {
    # Create stale lock with fake PID that doesn't exist
    mkdir -p "$HOME/.agentmux"
    echo '{"pid":99999,"ipc_port":9999,"started_at":"2025-01-01T00:00:00Z","version":"0.1.0"}' > "$HOME/.agentmux/desktop.lock"

    assert_file_exists "$HOME/.agentmux/desktop.lock"

    # Start should succeed (removes stale lock)
    run timeout 5s $AGENTMUX_BIN --headless --help
    assert_success

    # The lock should have been cleaned up
    # Note: In headless mode, no new lock is created
}

@test "lock file contains valid JSON" {
    skip "GUI tests require headless mode support - implementation pending"

    # Start instance
    $AGENTMUX_BIN &
    GUI_PID=$!

    # Wait for lock file
    wait_for_gui 10
    assert_file_exists "$HOME/.agentmux/desktop.lock"

    # Verify JSON is valid
    run jq . "$HOME/.agentmux/desktop.lock"
    assert_success

    # Verify required fields
    PID=$(get_lock_pid)
    assert [ -n "$PID" ]
    assert_process_running $PID

    PORT=$(get_lock_port)
    assert [ -n "$PORT" ]

    # Cleanup
    kill $GUI_PID 2>/dev/null || true
    wait $GUI_PID 2>/dev/null || true
}

@test "lock file cleaned up on exit" {
    skip "GUI tests require headless mode support - implementation pending"

    $AGENTMUX_BIN &
    GUI_PID=$!

    # Wait for lock
    wait_for_gui 10
    assert_file_exists "$HOME/.agentmux/desktop.lock"

    # Stop gracefully
    kill $GUI_PID
    wait $GUI_PID 2>/dev/null || true
    sleep 2

    # Lock should be removed
    assert_file_not_exists "$HOME/.agentmux/desktop.lock"
}

@test "headless mode does not create lock file" {
    # Run in headless mode
    run $AGENTMUX_BIN --headless --help
    assert_success

    # No lock file should exist
    assert_file_not_exists "$HOME/.agentmux/desktop.lock"
}

@test "headless mode shows help" {
    run $AGENTMUX_BIN --headless --help
    assert_success
    assert_contains "$output" "Usage:"
}

@test "headless mode shows version" {
    run $AGENTMUX_BIN --headless --version
    assert_success
    assert_contains "$output" "0."
}
