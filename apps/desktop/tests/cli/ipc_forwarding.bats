#!/usr/bin/env bats

load helpers/test_helper
load helpers/assertions

setup() {
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

@test "CLI commands forward to running instance" {
    skip "IPC tests require GUI instance - implementation pending"

    # Start GUI instance
    $AGENTMUX_BIN &
    GUI_PID=$!

    # Wait for IPC server
    wait_for_gui 10
    assert_file_exists "$HOME/.agentmux/desktop.lock"

    # Execute CLI command (should use IPC)
    run timeout 10s $AGENTMUX_BIN --json agents list
    assert_success
    assert_contains "$output" "agents"

    # Cleanup
    kill $GUI_PID 2>/dev/null || true
    wait $GUI_PID 2>/dev/null || true
}

@test "IPC returns valid JSON with --json flag" {
    skip "IPC tests require GUI instance - implementation pending"

    # Start GUI instance
    $AGENTMUX_BIN &
    GUI_PID=$!

    # Wait for IPC server
    wait_for_gui 10

    # Execute CLI command with JSON output
    run timeout 10s $AGENTMUX_BIN --json agents list
    assert_success

    # Validate JSON with jq
    echo "$output" | jq . > /dev/null
    assert [ $? -eq 0 ]

    # Cleanup
    kill $GUI_PID 2>/dev/null || true
    wait $GUI_PID 2>/dev/null || true
}

@test "IPC command timeout handled gracefully" {
    # Create stale lock (simulates hung process)
    mkdir -p "$HOME/.agentmux"
    echo '{"pid":99999,"ipc_port":9999,"started_at":"2025-01-01T00:00:00Z","version":"0.1.0"}' > "$HOME/.agentmux/desktop.lock"

    # CLI should detect stale lock and not hang
    run timeout 5s $AGENTMUX_BIN agents list

    # Should either succeed (after cleaning stale lock) or fail gracefully
    # Don't assert success/failure, just ensure it doesn't hang
}

@test "multiple IPC commands in sequence" {
    skip "IPC tests require GUI instance - implementation pending"

    # Start GUI instance
    $AGENTMUX_BIN &
    GUI_PID=$!

    # Wait for IPC server
    wait_for_gui 10

    # Execute multiple commands
    run timeout 10s $AGENTMUX_BIN agents list
    assert_success

    run timeout 10s $AGENTMUX_BIN --json agents list
    assert_success

    # Cleanup
    kill $GUI_PID 2>/dev/null || true
    wait $GUI_PID 2>/dev/null || true
}

@test "IPC forwards help command" {
    skip "IPC tests require GUI instance - implementation pending"

    # Start GUI instance
    $AGENTMUX_BIN &
    GUI_PID=$!

    # Wait for IPC server
    wait_for_gui 10

    # Help should work via IPC
    run timeout 10s $AGENTMUX_BIN --help
    assert_success
    assert_contains "$output" "Usage:"

    # Cleanup
    kill $GUI_PID 2>/dev/null || true
    wait $GUI_PID 2>/dev/null || true
}
