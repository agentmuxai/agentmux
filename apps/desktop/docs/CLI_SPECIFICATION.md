# AgentMux Desktop CLI Specification

**Version:** 1.0
**Date:** 2025-10-13
**Purpose:** Command-line interface for AgentMux Desktop for scripting, automation, and debugging

---

## Overview

AgentMux Desktop will support two CLI interaction modes:

1. **Launch-time CLI arguments** - Pass commands when starting the executable
2. **In-app command console** - Interactive command input within the running application

All UI operations will be scriptable via CLI commands, enabling:
- Automated testing
- CI/CD integration
- Remote debugging
- Agent-driven self-testing

---

## 1. Launch-Time CLI Arguments

### Syntax
```bash
agentmux-desktop.exe [OPTIONS] [COMMAND] [ARGS...]
```

### Global Options
- `--headless` - Run without opening GUI window
- `--json` - Output results in JSON format
- `--verbose` - Enable debug logging
- `--port <PORT>` - WebSocket server port (default: auto-assign)
- `--help` - Show help message
- `--version` - Show version information

### Commands

#### Agent Management
```bash
# List all Claude instances
agentmux-desktop.exe agents list [--json]

# Spawn new Claude instance
agentmux-desktop.exe agents spawn <name> [--command <cmd>] [--port <port>]

# Stop Claude instance
agentmux-desktop.exe agents stop <name>

# Send input to Claude instance
agentmux-desktop.exe agents input <name> <text>

# Get agent status
agentmux-desktop.exe agents status <name> [--json]
```

#### Message Bus Operations
```bash
# Send message to agent
agentmux-desktop.exe messages send --to <agent-id> --message <text> [--priority high|normal|low]

# List recent messages
agentmux-desktop.exe messages list [--limit 10] [--type command|status|message] [--json]

# Reply to message
agentmux-desktop.exe messages reply --id <message-id> --reply <text>

# Get active agents on bus
agentmux-desktop.exe messages agents [--json]
```

#### Dashboard / Monitoring
```bash
# Get bus status
agentmux-desktop.exe status bus [--json]

# Get all agent stats
agentmux-desktop.exe status agents [--json]

# Export debug logs
agentmux-desktop.exe logs export [--output <file>] [--format json|text]
```

### Examples

```bash
# Launch and spawn agent immediately
agentmux-desktop.exe agents spawn TestAgent

# List agents headless
agentmux-desktop.exe --headless --json agents list

# Send message from script
agentmux-desktop.exe messages send --to Agent1 --message "Status check"

# Debug mode with verbose logging
agentmux-desktop.exe --verbose agents status Agent1
```

---

## 2. In-App Command Console

### UI Component Location
**Bottom of Debug Console panel** - New text input field with command prompt

### UI Design
```
┌─────────────────────────────────────────────────────────┐
│ Debug Console                                    [▼] [x] │
├─────────────────────────────────────────────────────────┤
│ 22:33:40 [LOG] [Af] [WS] Connection opened              │
│ 22:33:42 [LOG] [AgentsManager] Loading instances...     │
│ ... (scrollable log view) ...                           │
├─────────────────────────────────────────────────────────┤
│ > █                                            [Execute] │
└─────────────────────────────────────────────────────────┘
```

### Command Syntax
Same as launch-time CLI, but without executable name:

```
> agents list
> agents spawn TestAgent2
> messages send --to Agent1 --message "Hello"
```

### Features
- **History navigation** - Up/Down arrows to recall previous commands
- **Tab completion** - Complete command names, agent IDs
- **Multiline support** - Shift+Enter for multiline input
- **Real-time output** - Command results appear in Debug Console above
- **Error highlighting** - Red text for errors, green for success

### Special Commands
- `clear` - Clear debug console
- `help [command]` - Show command help
- `exit` - Close command console (not the app)
- `debug on|off` - Toggle verbose debug logging

---

## 3. Command Response Format

### Text Format (Default)
```
✓ Agent spawned successfully
  Instance: TestAgent
  PID: 12345
  WebSocket: localhost:9001
  Status: running
```

### JSON Format (`--json`)
```json
{
  "success": true,
  "command": "agents spawn",
  "data": {
    "instanceName": "TestAgent",
    "pid": 12345,
    "wsPort": 9001,
    "status": "running"
  },
  "timestamp": "2025-10-13T22:45:00.000Z"
}
```

### Error Format
```json
{
  "success": false,
  "error": "Agent 'Unknown' not found",
  "code": "AGENT_NOT_FOUND",
  "timestamp": "2025-10-13T22:45:00.000Z"
}
```

---

## 4. Implementation Architecture

### Rust Backend (Tauri Commands)

```rust
// apps/desktop/src-tauri/src/cli_handler.rs

#[tauri::command]
async fn execute_cli_command(command: String, args: Vec<String>) -> Result<CliResponse, String> {
    match command.as_str() {
        "agents" => handle_agent_command(args).await,
        "messages" => handle_message_command(args).await,
        "status" => handle_status_command(args).await,
        _ => Err(format!("Unknown command: {}", command))
    }
}

struct CliResponse {
    success: bool,
    output: String,
    data: Option<serde_json::Value>,
}
```

### Frontend Component (SolidJS)

```typescript
// apps/desktop/src/components/CommandConsole.tsx

export function CommandConsole() {
  const [input, setInput] = createSignal('');
  const [history, setHistory] = createSignal<string[]>([]);
  const [historyIndex, setHistoryIndex] = createSignal(-1);

  const executeCommand = async () => {
    const cmd = input().trim();
    if (!cmd) return;

    // Add to history
    setHistory([cmd, ...history()]);

    // Parse command
    const [command, ...args] = cmd.split(' ');

    // Execute via Tauri
    try {
      const result = await invoke('execute_cli_command', { command, args });
      console.log('[CLI]', result.output);
    } catch (err) {
      console.error('[CLI]', err);
    }

    setInput('');
  };

  return (
    <div class="command-console">
      <input
        type="text"
        value={input()}
        onInput={(e) => setInput(e.currentTarget.value)}
        onKeyDown={handleKeyDown}
        placeholder="Enter command..."
      />
      <button onClick={executeCommand}>Execute</button>
    </div>
  );
}
```

---

## 5. Claude Agent Self-Testing Strategy

### Scenario: Claude Takes Control via CLI

**Objective:** Claude uses CLI to diagnose and fix the WebSocket connection loop issue

#### Step 1: Gather Diagnostics
```bash
# Check current agent status
> agents list --json

# Get specific agent details
> agents status Af --json

# Export recent logs
> logs export --output /tmp/debug.json --format json
```

#### Step 2: Analyze Logs
```bash
# Claude reads exported logs, identifies:
# - Close code 1006 (abnormal closure)
# - Pattern: connect → close → reconnect every 2 seconds
# - Hypothesis: SimpleTerminal component recreation
```

#### Step 3: Test Fix
```bash
# Stop problematic agent
> agents stop Af

# Spawn new agent with verbose logging
> debug on
> agents spawn TestFix

# Monitor logs in real-time
# (Debug Console shows all connection lifecycle events)
```

#### Step 4: Verify Solution
```bash
# Check connection stability (should show single persistent connection)
> agents status TestFix

# Expected output:
# ✓ Agent: TestFix
#   Status: running
#   Connections: 1 (stable for 60s)
#   WebSocket: 127.0.0.1:9003
```

#### Step 5: Document Findings
```bash
# Export test session logs
> logs export --output /tmp/solution-verification.json

# Claude analyzes logs, confirms:
# - Single WebSocket connection maintained
# - No reconnection loop
# - Fix successful
```

### Automated Test Script
```bash
#!/bin/bash
# test-agent-spawn.sh

echo "Testing AgentMux CLI..."

# Test 1: Spawn agent
echo -n "Spawning agent... "
RESULT=$(agentmux-desktop.exe --headless agents spawn CLITest --json)
PID=$(echo $RESULT | jq -r '.data.pid')
if [ "$PID" != "null" ]; then
  echo "✓ (PID: $PID)"
else
  echo "✗ FAILED"
  exit 1
fi

# Test 2: Check status
sleep 2
echo -n "Checking status... "
STATUS=$(agentmux-desktop.exe --headless agents status CLITest --json)
RUNNING=$(echo $STATUS | jq -r '.data.status')
if [ "$RUNNING" == "running" ]; then
  echo "✓"
else
  echo "✗ FAILED (status: $RUNNING)"
  exit 1
fi

# Test 3: Send message
echo -n "Sending message... "
agentmux-desktop.exe --headless messages send --to CLITest --message "Hello from test"
echo "✓"

# Test 4: Stop agent
echo -n "Stopping agent... "
agentmux-desktop.exe --headless agents stop CLITest
echo "✓"

echo "All tests passed!"
```

---

## 6. Implementation Phases

### Phase 1: Core CLI Infrastructure (v0.3.0)
- [ ] Rust CLI parser and command routing
- [ ] Tauri command handlers for agent operations
- [ ] JSON output formatting
- [ ] Headless mode support

### Phase 2: In-App Console (v0.3.1)
- [ ] CommandConsole SolidJS component
- [ ] Command history and navigation
- [ ] Output integration with Debug Console
- [ ] Tab completion

### Phase 3: Advanced Features (v0.3.2)
- [ ] Message bus CLI operations
- [ ] Log export and filtering
- [ ] Automated testing examples
- [ ] CI/CD integration guide

### Phase 4: Claude Self-Testing (v0.3.3)
- [ ] Agent-driven test scripts
- [ ] Self-diagnosis commands
- [ ] Automated bug reproduction
- [ ] Solution verification workflows

---

## 7. Testing Checklist

### Manual Testing
- [ ] Launch with `--help` shows usage
- [ ] Spawn agent via CLI creates process
- [ ] List agents shows correct count
- [ ] Stop agent terminates process
- [ ] Send message delivers to agent
- [ ] JSON output is valid
- [ ] Headless mode works without GUI

### Automated Testing
- [ ] test-agent-spawn.sh passes
- [ ] test-messages.sh passes
- [ ] test-status.sh passes
- [ ] CI integration tests pass

### Claude Self-Testing
- [ ] Claude can spawn agents via CLI
- [ ] Claude can read logs via CLI
- [ ] Claude can analyze patterns
- [ ] Claude can verify fixes
- [ ] Claude can document findings

---

## 8. Security Considerations

### Command Validation
- All commands must validate input (no shell injection)
- Agent names restricted to alphanumeric + underscore
- Message content sanitized
- File paths must be absolute and validated

### Access Control
- CLI commands run with same permissions as GUI
- No privilege escalation
- Commands log to audit trail

### Rate Limiting
- Prevent CLI command spam
- Max 100 commands/minute per session

---

## 9. Documentation

### User Guide
- CLI command reference (markdown)
- Example workflows
- Troubleshooting common errors

### Developer Guide
- Adding new commands
- Command handler patterns
- Testing CLI features

### CI/CD Integration Guide
- Using CLI in GitHub Actions
- Automated agent testing
- Log aggregation

---

## Summary

This CLI specification enables:

1. **Full UI parity** - Every GUI operation has CLI equivalent
2. **Automation** - Script complex workflows
3. **Testing** - Automated test suites
4. **Debugging** - Real-time diagnostics
5. **Claude control** - Agent can self-test and debug

**Next Steps:**
1. Implement Phase 1 (Core CLI Infrastructure)
2. Create test script examples
3. Document command reference
4. Enable Claude self-testing workflows
