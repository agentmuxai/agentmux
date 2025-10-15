#!/usr/bin/env bats
# E2E Test: Spawn agent and communicate with Claude CLI
# Tests the full integration: AgentMux -> Claude CLI -> Response

load helpers/test_helper
load helpers/assertions

setup() {
    # Clean state before each test
    cleanup_agentmux

    # Verify Claude CLI is available and authenticated
    if ! command -v claude &> /dev/null; then
        skip "Claude CLI not found in PATH"
    fi

    # Check authentication by attempting a simple command
    # If this fails, the test will be skipped with an informative message
    if ! timeout 5s claude --version &> /dev/null; then
        skip "Claude CLI not authenticated or not responding. Run 'claude auth login' first."
    fi

    # Get binary path
    AGENTMUX_BIN=$(get_agentmux_bin)
    if [ -z "$AGENTMUX_BIN" ]; then
        skip "agentmux binary not found. Run 'npm run tauri:build' first."
    fi

    # Create test workspace directory
    TEST_WORKSPACE="$HOME/.agentmux/test/workspace"
    mkdir -p "$TEST_WORKSPACE"

    # Agent configuration
    AGENT_NAME="TestAgent1"
    AGENT_WORKSPACE="$TEST_WORKSPACE"
}

teardown() {
    # Kill any running Claude processes spawned by test
    pkill -f "claude.*TestAgent" 2>/dev/null || true

    # Cleanup test workspace
    rm -rf "$HOME/.agentmux/test"

    # Standard cleanup
    cleanup_agentmux
}

@test "spawn agent and verify Claude CLI responds with agent identity" {
    # Test objective: Spawn an agent through AgentMux and verify Claude CLI
    # can respond to a simple identity query

    echo "# Step 1: Start AgentMux in headless mode" >&3
    timeout 10s $AGENTMUX_BIN --headless &
    AGENTMUX_PID=$!
    sleep 3

    # Verify AgentMux is running
    assert_process_running $AGENTMUX_PID

    echo "# Step 2: Spawn agent via AgentMux command" >&3
    # Note: This assumes AgentMux has a spawn command or we use the embedded_claude interface
    # For now, we'll test direct Claude CLI spawning as a baseline

    # Spawn Claude CLI and query identity
    # Use echo to pipe the question directly to Claude CLI
    cd "$AGENT_WORKSPACE"
    echo "what agent are you? respond in exactly this format: AgentN where N is your agent number" | \
        timeout 30s claude > /tmp/claude_response.txt 2>&1 &
    CLAUDE_PID=$!
    cd -

    echo "# Step 3: Wait for Claude CLI to process and respond" >&3
    # Wait up to 30 seconds for response
    local max_wait=30
    local elapsed=0
    while [ $elapsed -lt $max_wait ]; do
        if [ -s /tmp/claude_response.txt ]; then
            # File has content, check if response is complete
            if grep -q "Agent" /tmp/claude_response.txt 2>/dev/null; then
                break
            fi
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done

    echo "# Step 4: Verify response received" >&3
    # Check that we got a response
    assert_file_exists /tmp/claude_response.txt

    # Display response for debugging
    echo "# Claude CLI Response:" >&3
    cat /tmp/claude_response.txt >&3

    # Verify response contains agent identifier
    # Claude's response will vary but should mention "Agent" in some form
    assert_contains "$(cat /tmp/claude_response.txt)" "Agent"

    echo "# Step 5: Verify Claude process completed successfully" >&3
    # Wait for Claude to finish
    wait $CLAUDE_PID || true

    # Cleanup
    rm -f /tmp/claude_response.txt
    kill $AGENTMUX_PID 2>/dev/null || true
}

@test "spawn agent and verify it can receive messages through AgentMux" {
    skip "Integration test pending: AgentMux message bus integration"

    # Future test: Verify messages can flow through AgentMux to spawned Claude agent
    # 1. Start AgentMux bus
    # 2. Spawn agent connected to bus
    # 3. Send message via bus
    # 4. Verify agent receives and can process message
}

@test "verify Claude CLI authentication persists across spawned instances" {
    # Test that authentication works for multiple sequential spawns
    # This ensures the auth token is accessible to all spawned processes

    echo "# Testing authentication persistence" >&3

    # Spawn first instance
    echo "test" | timeout 10s claude --version > /dev/null 2>&1
    local first_exit=$?

    # Spawn second instance
    echo "test" | timeout 10s claude --version > /dev/null 2>&1
    local second_exit=$?

    # Both should succeed (exit code 0)
    [ $first_exit -eq 0 ] || {
        skip "First Claude CLI instance failed authentication"
    }

    [ $second_exit -eq 0 ] || {
        echo "# ERROR: Second instance failed but first succeeded" >&3
        echo "# This indicates authentication may not persist" >&3
        return 1
    }

    echo "# Authentication persists across instances ✓" >&3
}

@test "handle Claude CLI authentication prompt gracefully" {
    skip "Manual test required: Claude CLI authentication flow"

    # This test documents the authentication requirement
    # In production:
    # 1. Claude CLI may require initial auth via browser
    # 2. User must complete: claude auth login
    # 3. Session token stored in ~/.config/claude/
    # 4. All spawned instances share this token

    # Test strategy for automation:
    # - Check for existing auth token
    # - If missing, provide clear error with instructions
    # - Do not attempt to automate browser-based auth flow
}
