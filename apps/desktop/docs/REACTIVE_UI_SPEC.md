# Reactive UI Specification

**Status:** Proposed
**Version:** 1.0
**Date:** 2025-10-14

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
**CLI → UI:** All CLI operations must immediately reflect in UI
**UI → CLI:** All UI actions must show in embedded terminal/CLI output

### 2. External CLI Support
When desktop app is launched from command line with args, UI must:
- Initialize with specified configuration
- Show debug output in terminal
- Reflect CLI-specified state

### 3. Real-time Updates
- Agent list updates when agents spawn/terminate
- Message stream updates when messages sent/received
- Bus status updates when bus starts/stops
- Log export operations show in UI

---

## Proposed Architecture

### Event-Driven State Synchronization

```
┌──────────────────────────────────────────────────────────────┐
│                     Service Layer (Rust)                     │
│                                                               │
│  ┌─────────────┐  ┌────────────┐  ┌──────────────┐         │
│  │   Agent     │  │  Message   │  │     Log      │         │
│  │  Service    │  │  Service   │  │   Service    │         │
│  └──────┬──────┘  └──────┬─────┘  └──────┬───────┘         │
│         │                 │                │                  │
│         └─────────────────┼────────────────┘                  │
│                           │                                   │
│                      Emit Events                              │
│                           │                                   │
└───────────────────────────┼───────────────────────────────────┘
                            │
            ┌───────────────┴───────────────┐
            │                               │
       ┌────▼─────┐                   ┌────▼────┐
       │   CLI    │                   │   UI    │
       │ Handler  │                   │  React  │
       └──────────┘                   └─────────┘
       │                                      │
       │  Parse command                       │  Update components
       │  Call service                        │  Refresh lists
       │  Subscribe to events                 │  Show notifications
       └──────────────────────────────────────┘
```

### Event Types

**State Change Events** (service → UI/CLI):
```typescript
// When agents change
agent_spawned { instanceName, pid, wsPort }
agent_terminated { instanceName }
agent_list_updated { agents: Agent[] }

// When messages are sent/received
message_sent { from, to, message }
message_received { from, message }
message_broadcast { from, message }

// When bus state changes
bus_started { host, port }
bus_stopped {}
bus_agent_connected { agentId }
bus_agent_disconnected { agentId }

// When logs are exported
logs_exported { path, format, count }

// CLI command executed (for terminal echo)
cli_executed { command, output, success }
```

---

## Implementation Plan

### Phase 1: Service Layer Event Emission

**Changes needed in services:**

```rust
// services/logs.rs
pub fn export_logs(request: LogExportRequest, app_handle: Option<AppHandle>) -> LogExportResult {
    // ... existing logic ...

    let result = LogExportResult { ... };

    // Emit event
    if let Some(handle) = app_handle {
        let _ = handle.emit_all("logs_exported", json!({
            "path": result.output_path,
            "format": request.format,
            "count": result.entries_count,
        }));
    }

    result
}
```

**Update service layer functions:**
- Add optional `AppHandle` parameter
- Emit events after successful operations
- Keep services decoupled (events are optional)

### Phase 2: CLI Handler Event Integration

**Update CLI handlers to emit events:**

```rust
// cli/handlers.rs
async fn handle_agent_action(
    action: AgentAction,
    format: OutputFormat,
    state: Option<State<'_, ClaudeInstancesState>>,
    app_handle: Option<AppHandle>,  // NEW
) -> CliResponse {
    match action {
        AgentAction::Spawn { name, ... } => {
            let instance = ClaudeInstance::spawn(...).await?;

            // Emit event
            if let Some(handle) = app_handle {
                let _ = handle.emit_all("agent_spawned", json!({
                    "instanceName": instance.instance_name,
                    "pid": instance.pid,
                    "wsPort": instance.ws_port,
                }));
            }

            CliResponse::success(...)
        }
        // ... other actions ...
    }
}
```

### Phase 3: UI Event Listeners

**Update UI components to listen for events:**

```typescript
// components/AgentsManager.tsx
onMount(async () => {
    // Initial load
    await loadAgents();

    // Listen for agent changes
    await listen('agent_spawned', (event) => {
        const agent = event.payload as Agent;
        setAgents([...agents(), agent]);
    });

    await listen('agent_terminated', (event) => {
        const { instanceName } = event.payload;
        setAgents(agents().filter(a => a.instanceName !== instanceName));
    });

    await listen('agent_list_updated', (event) => {
        const { agents: newAgents } = event.payload;
        setAgents(newAgents);
    });
});
```

**Update all components:**
- `AgentsManager` - listen to agent events
- `MessageStream` - listen to message events
- `Dashboard` - listen to bus events
- `BusControl` - listen to bus events
- Add visual feedback (toast notifications)

### Phase 4: External CLI Integration

**Handle CLI arguments on launch:**

```rust
// main.rs
fn main() {
    use clap::Parser;

    // Parse CLI args
    let cli = cli::parser::Cli::parse();

    tauri::Builder::default()
        .setup(move |app| {
            if let Some(command) = cli.command {
                // Execute command and emit to UI
                let handle = app.handle();
                tauri::async_runtime::spawn(async move {
                    let result = cli::handlers::handle_command(
                        command,
                        cli::output::OutputFormat::Text,
                        None,
                        Some(handle.clone()),
                    ).await;

                    // Emit to UI for display
                    let _ = handle.emit_all("cli_executed", json!({
                        "command": "...",
                        "output": result.output,
                        "success": result.success,
                    }));
                });
            }
            Ok(())
        })
        .invoke_handler(...)
        .run(...)
}
```

**Add CLI output panel:**
- Show CLI command results in UI
- Auto-scroll to latest output
- Support for --debug flag to show verbose logs

---

## Example Flows

### Flow 1: CLI Command → UI Update

```
User: Types "agent spawn Agent3" in embedded terminal
  ↓
CLI Handler: Parses command, calls service
  ↓
Agent Service: Spawns agent, returns result
  ↓
CLI Handler: Emits agent_spawned event
  ↓
UI: AgentsManager receives event, adds to list
  ↓
Result: Agent appears in UI instantly
```

### Flow 2: UI Action → CLI Feedback

```
User: Clicks "Spawn Agent" button in UI
  ↓
UI: Calls spawn_embedded_claude Tauri command
  ↓
Tauri Command: Calls Agent Service
  ↓
Agent Service: Spawns agent, emits agent_spawned
  ↓
CLI Terminal: Receives event, displays: "✓ Agent spawned: Agent3 (PID: 1234)"
  ↓
Result: User sees feedback in both UI and terminal
```

### Flow 3: External CLI Launch

```
User: Runs "agentmux-desktop agent spawn Agent3 --debug"
  ↓
main.rs: Parses CLI args, starts app with command
  ↓
setup(): Executes command, emits cli_executed event
  ↓
UI: Opens with Agents tab active, shows Agent3
  ↓
Terminal: Shows debug logs and command output
  ↓
Result: App initialized with CLI-specified state
```

---

## Implementation Checklist

### Phase 1: Service Layer Events
- [ ] Add AppHandle parameter to service functions
- [ ] Emit `logs_exported` event in logs service
- [ ] Emit `agent_spawned`/`agent_terminated` events
- [ ] Emit `message_sent`/`message_received` events
- [ ] Emit `bus_started`/`bus_stopped` events
- [ ] Keep services backward compatible (optional handle)

### Phase 2: CLI Handler Integration
- [ ] Update CLI handlers to accept AppHandle
- [ ] Pass AppHandle to service calls
- [ ] Emit `cli_executed` event for all commands
- [ ] Update `execute_cli_command` Tauri command

### Phase 3: UI Event Listeners
- [ ] Add event listeners to AgentsManager
- [ ] Add event listeners to MessageStream
- [ ] Add event listeners to Dashboard
- [ ] Add event listeners to BusControl
- [ ] Implement toast notifications for events
- [ ] Add CLI output panel/section

### Phase 4: External CLI Support
- [ ] Parse CLI args in main.rs
- [ ] Execute commands on app startup
- [ ] Support --debug flag for verbose output
- [ ] Handle --help and --version flags
- [ ] Initialize UI based on CLI state

### Phase 5: Testing
- [ ] Test CLI → UI sync (agent spawn)
- [ ] Test UI → CLI feedback (button clicks)
- [ ] Test external CLI launch
- [ ] Test concurrent operations
- [ ] Test event cleanup on component unmount

---

## Benefits

### For Users
- **Instant feedback** - See changes immediately in UI
- **Consistent state** - UI always matches actual state
- **Flexible interface** - Use CLI or GUI interchangeably
- **Debug visibility** - See what's happening behind the scenes

### For Developers
- **Maintainable** - Single source of truth (services)
- **Testable** - Events are observable and loggable
- **Scalable** - Add new events without breaking existing code
- **Decoupled** - Services don't know about UI/CLI

---

## Risks & Mitigation

### Risk: Event Flood
**Problem:** Rapid operations could flood UI with events
**Mitigation:**
- Debounce updates (e.g., agent_list_updated batches changes)
- Limit event frequency with rate limiting
- Use aggregate events instead of individual events

### Risk: Event Loss
**Problem:** Events emitted before UI is ready
**Mitigation:**
- UI components request initial state on mount
- Events are supplementary, not the only source of truth
- Polling as fallback for critical state

### Risk: Memory Leaks
**Problem:** Event listeners not cleaned up
**Mitigation:**
- Use `onCleanup()` in all components
- Store unlisten functions and call on unmount
- Test component mount/unmount cycles

---

## Future Enhancements

- **Event History:** Store recent events for replay
- **Event Filtering:** UI can filter which events to show
- **Performance Metrics:** Track event latency
- **Event Recording:** Record events for debugging
- **Cross-Instance Sync:** Multiple desktop app instances share state

---

## References

- Tauri Events API: https://tauri.app/v1/guides/features/events/
- SolidJS Event Handling: https://www.solidjs.com/tutorial/lifecycles_onmount
- Service Layer Architecture: `apps/desktop/docs/ARCHITECTURE.md`
