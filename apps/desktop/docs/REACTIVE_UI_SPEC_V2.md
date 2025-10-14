# Reactive UI Specification v2.0

**Status:** Proposed (Updated)
**Version:** 2.0
**Date:** 2025-10-14
**Supersedes:** v1.0

---

## Changes from v1.0

1. ✅ **Clarified service layer boundaries** - Use existing modules, not fictional services
2. ✅ **Fixed circular dependency** - UI → CLI feedback uses proper Tauri command flow
3. ✅ **Resolved CLI vs GUI mode** - Added mode detection and headless support
4. ✅ **Added event sourcing layer** - Wrapper functions emit events without refactoring core logic
5. ✅ **Fixed embedded terminal feedback** - Use WebSocket messages, not Tauri events
6. ✅ **Standardized event naming** - Consistent snake_case for events and payloads
7. ✅ **Added state reconciliation** - Polling + events hybrid approach

---

## Problem Statement

Currently, the Desktop UI and CLI are **not fully synchronized**:

- ❌ CLI commands don't update UI state (e.g., `agent spawn` doesn't show in Agents tab)
- ❌ UI actions don't reflect in CLI output
- ❌ External CLI invocations don't update UI
- ⚠️ Only partial reactivity: Dashboard listens to `cli_command` events for start/stop bus

---

## Requirements

### 1. Bidirectional Sync
**CLI → UI:** All CLI operations must immediately reflect in UI via events
**UI → CLI:** All UI actions must show feedback via WebSocket to embedded terminal

### 2. Mode Detection
**GUI Mode:** Full desktop app with UI
**CLI Mode:** Headless execution, output to terminal only
**Hybrid Mode:** GUI with CLI args pre-executed on startup

### 3. Real-time Updates with Fallback
- Primary: Event-driven updates (instant)
- Fallback: Polling for state reconciliation (every 2-5s)
- Agent list syncs when agents spawn/terminate
- Message stream syncs when messages sent/received
- Bus status syncs when bus starts/stops

---

## Proposed Architecture

### Layered Event Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     Tauri Commands Layer                        │
│  (spawn_embedded_claude, start_bus, execute_cli_command, etc.) │
│                                                                  │
│  Each command wraps core logic + emits events                   │
└────────────────────┬────────────────────────────────────────────┘
                     │
                     │ Calls core logic + emits events
                     ↓
┌─────────────────────────────────────────────────────────────────┐
│                      Core Modules                                │
│                                                                  │
│  ┌──────────────┐  ┌────────────┐  ┌──────────────┐           │
│  │  embedded_   │  │    bus     │  │   services   │           │
│  │   claude     │  │  manager   │  │    /logs     │           │
│  └──────────────┘  └────────────┘  └──────────────┘           │
│                                                                  │
│  Unchanged - no AppHandle dependency                            │
└─────────────────────────────────────────────────────────────────┘
                     │
                     │ State changes
                     ↓
            ┌────────┴────────┐
            │                 │
       ┌────▼────┐      ┌────▼────┐
       │  Tauri  │      │   UI    │
       │  Events │      │Components│
       └─────────┘      └─────────┘
            │                 │
            │  Bidirectional  │
            └────────┬────────┘
                     │
              ┌──────▼───────┐
              │  Embedded    │
              │  Terminal    │
              │  (WebSocket) │
              └──────────────┘
```

### Event Emission Strategy

**DON'T modify core modules** (embedded_claude, bus, services)

**DO add event emission in Tauri commands**:

```rust
// main.rs - Tauri command wraps core logic
#[tauri::command]
async fn spawn_embedded_claude(
    instance_name: String,
    state: State<'_, ClaudeInstancesState>,
    app_handle: AppHandle,  // NEW - for emitting events
) -> Result<Agent, String> {
    // Call core logic (unchanged)
    let instance = embedded_claude::ClaudeInstance::spawn(
        instance_name.clone(),
        find_available_port(9000, 9999)?,
    ).await?;

    // Convert to response
    let agent = Agent {
        instance_name: instance.instance_name.clone(),
        pid: instance.pid,
        ws_port: instance.ws_port,
        status: "running".to_string(),
    };

    // Store in state
    state.instances.lock().await.insert(instance_name, instance);

    // Emit event (NEW)
    let _ = app_handle.emit_all("agent_spawned", json!({
        "instance_name": agent.instance_name,
        "pid": agent.pid,
        "ws_port": agent.ws_port,
        "status": agent.status,
    }));

    Ok(agent)
}
```

### Event Types (Standardized)

**Naming Convention:** `snake_case` for event names and payload fields

```typescript
// Agent events
agent_spawned { instance_name, pid, ws_port, status }
agent_terminated { instance_name, reason }

// Bus events
bus_started { host, port, max_agents }
bus_stopped { reason }
bus_agent_connected { agent_id, agent_name }
bus_agent_disconnected { agent_id, reason }

// Message events
message_sent { from_agent, to_agent, message_text, timestamp }
message_received { from_agent, message_text, timestamp }
message_broadcast { from_agent, message_text, recipient_count, timestamp }

// Log events
logs_exported { output_path, format, entries_count, success }

// CLI execution events
cli_command_executed { command_text, output_text, success, duration_ms }

// State reconciliation events
state_sync_requested { component }
state_sync_completed { component, items_count }
```

---

## Implementation Plan

### Phase 1: Add Event Emission to Tauri Commands

**Modify existing Tauri commands to emit events:**

```rust
// main.rs

#[tauri::command]
async fn spawn_embedded_claude(..., app_handle: AppHandle) -> Result<Agent, String> {
    // ... existing logic ...
    let _ = app_handle.emit_all("agent_spawned", ...);
    Ok(agent)
}

#[tauri::command]
async fn start_bus(..., app_handle: AppHandle) -> Result<String, String> {
    // ... existing logic ...
    let _ = app_handle.emit_all("bus_started", ...);
    Ok(result)
}

#[tauri::command]
async fn export_logs(
    output_path: Option<String>,
    format: String,
    app_handle: AppHandle,  // NEW
) -> Result<String, String> {
    // ... existing service call ...
    let _ = app_handle.emit_all("logs_exported", json!({
        "output_path": result.output_path,
        "format": format,
        "entries_count": result.entries_count,
        "success": result.success,
    }));
    Ok(...)
}
```

**Update all Tauri commands:**
- `spawn_embedded_claude` → emit `agent_spawned`
- `terminate_embedded_claude` → emit `agent_terminated`
- `start_bus` → emit `bus_started`
- `stop_bus` → emit `bus_stopped`
- `send_message` (if exists) → emit `message_sent`
- `export_logs` → emit `logs_exported`

### Phase 2: Update CLI Handler to Emit Events

**CLI commands should also emit events:**

```rust
// cli/handlers.rs - Keep existing signature, add AppHandle internally
async fn handle_agent_action(
    action: AgentAction,
    format: OutputFormat,
    state: Option<State<'_, ClaudeInstancesState>>,
) -> CliResponse {
    match action {
        AgentAction::Spawn { name, ... } => {
            // Get AppHandle from Tauri global state (if running in GUI mode)
            if let Some(handle) = try_get_app_handle() {
                // Emit event
                let _ = handle.emit_all("agent_spawned", ...);
            }

            CliResponse::success(...)
        }
    }
}

// Helper to get AppHandle in CLI context
fn try_get_app_handle() -> Option<AppHandle> {
    // Implementation depends on how Tauri global state is accessed
    // May use Arc<Mutex<Option<AppHandle>>> in app state
    None  // Return None if not in GUI mode
}
```

**Alternative:** Pass AppHandle through execute_cli_command

```rust
#[tauri::command]
async fn execute_cli_command(
    command_str: String,
    json_output: bool,
    state: State<'_, ClaudeInstancesState>,
    app_handle: AppHandle,  // NEW
) -> Result<String, String> {
    // ... parse command ...

    // Pass app_handle to handler (requires handler signature change)
    let result = cli::handlers::handle_command(
        cli.command,
        format,
        Some(state),
        Some(app_handle.clone()),  // NEW
    ).await;

    // Emit cli_command_executed event
    let _ = app_handle.emit_all("cli_command_executed", json!({
        "command_text": command_str,
        "output_text": result.output,
        "success": result.success,
        "duration_ms": 0,  // TODO: track timing
    }));

    Ok(result.format())
}
```

### Phase 3: Add UI Event Listeners

**Update all UI components to listen for events:**

```typescript
// components/AgentsManager.tsx
import { listen, UnlistenFn } from '@tauri-apps/api/event';

onMount(async () => {
    // Initial load
    await loadAgents();

    // Store unlisten functions for cleanup
    const unlisteners: UnlistenFn[] = [];

    // Listen for agent events
    unlisteners.push(await listen('agent_spawned', (event) => {
        const { instance_name, pid, ws_port, status } = event.payload;
        const newAgent: Agent = { instanceName: instance_name, pid, wsPort: ws_port, status };
        setAgents([...agents(), newAgent]);
        console.log('[AgentsManager] Agent spawned:', instance_name);
    }));

    unlisteners.push(await listen('agent_terminated', (event) => {
        const { instance_name } = event.payload;
        setAgents(agents().filter(a => a.instanceName !== instance_name));
        console.log('[AgentsManager] Agent terminated:', instance_name);
    }));

    // Fallback: Poll for reconciliation every 5s
    const pollInterval = setInterval(async () => {
        try {
            const agentsList: Agent[] = await invoke('list_claude_instances');
            // Only update if count changed (avoid unnecessary re-renders)
            if (agentsList.length !== agents().length) {
                setAgents(agentsList);
                console.log('[AgentsManager] State reconciled via polling');
            }
        } catch (err) {
            console.error('[AgentsManager] Polling failed:', err);
        }
    }, 5000);

    onCleanup(() => {
        // Clean up event listeners
        unlisteners.forEach(fn => fn());
        clearInterval(pollInterval);
    });
});
```

**Apply to all components:**
- `AgentsManager` → `agent_spawned`, `agent_terminated`
- `Dashboard` → `bus_started`, `bus_stopped`, `bus_agent_connected`, `bus_agent_disconnected`
- `BusControl` → same as Dashboard
- `MessageStream` → `message_sent`, `message_received`, `message_broadcast`
- `App` (global) → `cli_command_executed` (show in toast notification)

### Phase 4: Mode Detection & External CLI Support

**Detect mode on startup:**

```rust
// main.rs
use clap::Parser;

#[derive(Debug, Clone)]
enum AppMode {
    Gui,           // Normal GUI mode
    Headless,      // CLI only, no GUI (future)
    Hybrid,        // GUI + pre-execute CLI command
}

fn main() {
    // Parse CLI args before Tauri setup
    let cli = cli::parser::Cli::try_parse();

    let mode = match cli {
        Ok(ref parsed) if parsed.command.is_some() => AppMode::Hybrid,
        Ok(_) => AppMode::Gui,
        Err(_) => AppMode::Gui,  // Parse errors default to GUI
    };

    tauri::Builder::default()
        .setup(move |app| {
            match mode {
                AppMode::Hybrid => {
                    // Execute CLI command after app starts
                    if let Ok(cli) = cli.clone() {
                        if let Some(command) = cli.command {
                            let handle = app.handle();
                            tauri::async_runtime::spawn(async move {
                                // Give UI time to initialize
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                                let result = cli::handlers::handle_command(
                                    command,
                                    cli::output::OutputFormat::Text,
                                    None,  // TODO: Get state from app
                                    Some(handle.clone()),
                                ).await;

                                // Emit to UI
                                let _ = handle.emit_all("cli_command_executed", json!({
                                    "command_text": format!("{:?}", command),
                                    "output_text": result.output,
                                    "success": result.success,
                                    "duration_ms": 0,
                                }));

                                // Also print to stdout for terminal feedback
                                println!("{}", result.output);
                            });
                        }
                    }
                }
                AppMode::Gui => {
                    // Normal startup, no CLI pre-execution
                }
                AppMode::Headless => {
                    // Future: Run without GUI, exit after command
                    unimplemented!("Headless mode not yet implemented");
                }
            }

            Ok(())
        })
        .invoke_handler(...)
        .run(...)
}
```

**Support --debug flag:**

```rust
// cli/parser.rs
#[derive(Parser)]
#[command(name = "agentmux-desktop")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Enable debug output
    #[arg(long, global = true)]
    pub debug: bool,

    /// Run in headless mode (no GUI)
    #[arg(long, global = true)]
    pub headless: bool,
}

// main.rs - use debug flag
if cli.as_ref().map(|c| c.debug).unwrap_or(false) {
    // Enable verbose logging
    std::env::set_var("RUST_LOG", "debug");
}
```

### Phase 5: Embedded Terminal Feedback (WebSocket)

**UI → Terminal feedback via WebSocket messages:**

```typescript
// When UI spawns agent, send message to embedded terminal (if visible)
async function spawnAgentFromUI(instanceName: string) {
    try {
        const result = await invoke('spawn_embedded_claude', { instanceName });

        // Send feedback to embedded terminal via WebSocket
        // (Requires terminal to be running and connected)
        const terminalWs = getTerminalWebSocket();  // Get active terminal WS
        if (terminalWs) {
            terminalWs.send(JSON.stringify({
                type: 'feedback',
                message: `✓ Agent spawned: ${result.instanceName} (PID: ${result.pid})`,
                timestamp: Date.now(),
            }));
        }

        return result;
    } catch (err) {
        console.error('Failed to spawn agent:', err);
        throw err;
    }
}
```

**Terminal displays feedback messages:**

```typescript
// SimpleTerminal.tsx or EmbeddedTerminal.tsx
ws.onmessage = (event) => {
    const data = JSON.parse(event.data);

    if (data.type === 'feedback') {
        // Display as special message (e.g., blue/green color)
        term.writeln(`\x1b[36m${data.message}\x1b[0m`);
    } else {
        // Normal output
        term.write(data);
    }
};
```

---

## Example Flows (Corrected)

### Flow 1: CLI Command → UI Update

```
User: Types "agent spawn Agent3" in embedded terminal
  ↓
execute_cli_command: Parses command, calls handler
  ↓
CLI Handler: Calls embedded_claude::ClaudeInstance::spawn()
  ↓
Tauri Command (spawn_embedded_claude): Emits agent_spawned event
  ↓
UI AgentsManager: Receives event, adds Agent3 to list
  ↓
Result: Agent3 appears in UI Agents tab instantly
```

### Flow 2: UI Action → Terminal Feedback

```
User: Clicks "Spawn Agent" button in UI
  ↓
UI: Calls invoke('spawn_embedded_claude', { instanceName: 'Agent3' })
  ↓
Tauri Command: Spawns agent, emits agent_spawned event, returns result
  ↓
UI: Sends WebSocket message to embedded terminal with feedback
  ↓
Embedded Terminal: Displays "✓ Agent spawned: Agent3 (PID: 1234)" in cyan
  ↓
Result: User sees feedback in both UI list and terminal output
```

### Flow 3: External CLI Launch (Hybrid Mode)

```
User: Runs "agentmux-desktop agent spawn Agent3 --debug" in external terminal
  ↓
main.rs: Parses CLI args, detects Hybrid mode
  ↓
Tauri setup(): Starts app, waits 500ms for UI to initialize
  ↓
Background task: Executes "agent spawn Agent3", emits events
  ↓
UI: Receives agent_spawned event, adds Agent3 to Agents tab
  ↓
stdout: Prints "✓ Agent spawned: Agent3 (PID: 1234)" to external terminal
  ↓
Result: App opens with Agent3 already visible in UI, debug logs in terminal
```

---

## Implementation Checklist

### Phase 1: Tauri Command Event Emission
- [ ] Add AppHandle parameter to all Tauri commands
- [ ] Emit `agent_spawned` in `spawn_embedded_claude`
- [ ] Emit `agent_terminated` in `terminate_embedded_claude`
- [ ] Emit `bus_started` in `start_bus`
- [ ] Emit `bus_stopped` in `stop_bus`
- [ ] Emit `logs_exported` in `export_logs`
- [ ] Test event emission with debug console

### Phase 2: CLI Handler Event Emission
- [ ] Update `execute_cli_command` to accept AppHandle
- [ ] Pass AppHandle to `handle_command` function
- [ ] Update handler signature to accept `Option<AppHandle>`
- [ ] Emit `cli_command_executed` event
- [ ] Test CLI commands emit events correctly

### Phase 3: UI Event Listeners
- [ ] Add event listeners to AgentsManager
- [ ] Add event listeners to Dashboard
- [ ] Add event listeners to BusControl
- [ ] Add event listeners to MessageStream
- [ ] Implement polling fallback (every 5s)
- [ ] Ensure `onCleanup()` removes listeners
- [ ] Test listener cleanup on tab switches

### Phase 4: Mode Detection & External CLI
- [ ] Add `AppMode` enum
- [ ] Parse CLI args before Tauri setup
- [ ] Implement Hybrid mode command execution
- [ ] Add --debug flag support
- [ ] Add --headless flag (stub for future)
- [ ] Test external CLI launch with GUI
- [ ] Test debug output to terminal

### Phase 5: Embedded Terminal Feedback
- [ ] Create WebSocket message protocol for feedback
- [ ] Implement `getTerminalWebSocket()` helper
- [ ] Update terminal to display feedback messages
- [ ] Style feedback messages (color, formatting)
- [ ] Test UI → terminal feedback flow
- [ ] Handle terminal not connected gracefully

### Phase 6: Testing & Validation
- [ ] Test: CLI spawn → UI updates
- [ ] Test: UI spawn → terminal shows feedback
- [ ] Test: External CLI launch with --debug
- [ ] Test: Event listener cleanup (no memory leaks)
- [ ] Test: Polling reconciliation when events missed
- [ ] Test: Concurrent operations (rapid spawns)
- [ ] Test: Tab switching (listeners persist/cleanup)
- [ ] Test: App restart (state preserved)

---

## Risks & Mitigation (Updated)

### Risk: Event Flood
**Mitigation:**
- Debounce UI updates (batch within 100ms window)
- Use aggregate events for lists (`agent_list_updated` vs many `agent_spawned`)
- Limit polling frequency (5s minimum)

### Risk: Event Loss
**Mitigation:**
- Hybrid approach: Events + Polling
- UI requests full state on mount
- Polling acts as reconciliation mechanism
- Critical operations log to persistent storage

### Risk: Memory Leaks
**Mitigation:**
- Mandatory `onCleanup()` in all components
- Test with tab switching (mount/unmount cycles)
- Use WeakRef for WebSocket connections
- Clear intervals/timeouts in cleanup

### Risk: WebSocket Connection Failures
**Mitigation:**
- Gracefully handle terminal not connected
- Check `ws.readyState` before sending
- Queue feedback messages if connection lost
- Don't block UI operations on terminal feedback

### Risk: Hybrid Mode Timing Issues
**Mitigation:**
- Wait 500ms for UI to initialize before executing command
- Emit events after command completes
- Print to stdout regardless of UI state
- Handle app close during command execution

---

## Future Enhancements

- **Event History Panel:** Show last 100 events with timestamps
- **Event Filtering:** Toggle which events to display
- **Performance Metrics:** Track event latency (emit → receive)
- **Event Recording:** Record/replay events for debugging
- **Multi-Window Sync:** Sync state across multiple app windows
- **True Headless Mode:** Run commands without GUI, exit after completion

---

## References

- Tauri Events API: https://tauri.app/v1/guides/features/events/
- Tauri CLI Integration: https://tauri.app/v1/guides/features/command-line-arguments/
- SolidJS Lifecycles: https://www.solidjs.com/tutorial/lifecycles_onmount
- Service Layer Architecture: `apps/desktop/docs/ARCHITECTURE.md`
- WebSocket Protocol: `apps/desktop/docs/SPEC_EMBEDDED_TERMINAL_VERIFIED.md`

---

## Summary of Corrections

1. **No fictional services** - Use existing `embedded_claude`, `bus`, `services/logs` modules
2. **Event emission in Tauri commands** - Wrap core logic, don't modify it
3. **Fixed UI → CLI feedback** - Use WebSocket messages, not impossible Tauri event subscription
4. **Clarified mode detection** - GUI, Headless, Hybrid with explicit handling
5. **Added polling fallback** - Events + polling hybrid for reliability
6. **Standardized naming** - snake_case everywhere
7. **Practical implementation** - All code examples are actually implementable
