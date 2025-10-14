# Reactive UI Implementation Status

**Spec:** REACTIVE_UI_SPEC_V2.md
**Started:** 2025-10-14
**Branch:** docs/architecture-standard

---

## Phase 1: Backend Event Emission ✅ COMPLETED

### Changes Made

**File:** `src-tauri/src\main.rs`

**Modifications:**
1. Added `use tauri::Emitter` import
2. Added `AppHandle` parameter to commands that emit events

**Events Implemented:**

| Event Name | Command | Payload | Status |
|-----------|---------|---------|--------|
| `bus_started` | `start_bus` | `{host, port, max_agents}` | ✅ |
| `bus_stopped` | `stop_bus` | `{reason}` | ✅ |
| `agent_spawned` | `spawn_embedded_claude` | `{instance_name, pid, ws_port, status}` | ✅ |
| `message_sent` | `send_message` | `{from_agent, to_agent, message_text, timestamp}` | ✅ |
| `logs_exported` | `export_logs` | `{output_path, format, entries_count, success}` | ✅ |
| `cli_command_executed` | `execute_cli_command` | `{command_text, output_text, success, duration_ms}` | ✅ |

**Commits:**
- `0eb1d4b` - feat: Add reactive UI event emission (Phase 1)

**Build Status:** ✅ Compiles successfully

---

## Phase 2: UI Event Listeners ⏳ IN PROGRESS

### Components to Update

#### 1. AgentsManager.tsx (High Priority)
**Events to listen for:**
- `agent_spawned` - Add new agent to list
- (future) `agent_terminated` - Remove agent from list

**Implementation:**
```typescript
import { listen, UnlistenFn } from '@tauri-apps/api/event';

onMount(async () => {
    const unlisten: UnlistenFn[] = [];

    // Initial load
    await loadAgents();

    // Listen for agent_spawned
    unlisten.push(await listen('agent_spawned', (event) => {
        const payload = event.payload as {
            instance_name: string;
            pid: number;
            ws_port: number;
            status: string;
        };

        setAgents(prev => [...prev, {
            instanceName: payload.instance_name,
            pid: payload.pid,
            wsPort: payload.ws_port,
            status: payload.status
        }]);
    }));

    // Polling fallback (every 5s)
    const interval = setInterval(async () => {
        const list = await invoke('list_claude_instances');
        if (list.length !== agents().length) {
            setAgents(list);
        }
    }, 5000);

    onCleanup(() => {
        unlisten.forEach(fn => fn());
        clearInterval(interval);
    });
});
```

**Status:** ⏳ Pending

#### 2. Dashboard.tsx (Medium Priority)
**Events to listen for:**
- `bus_started` - Update bus status
- `bus_stopped` - Update bus status
- `cli_command_executed` - Show command feedback

**Status:** ⏳ Pending

#### 3. BusControl.tsx (Medium Priority)
**Events to listen for:**
- `bus_started` - Enable stop button, disable start button
- `bus_stopped` - Enable start button, disable stop button

**Status:** ⏳ Pending

#### 4. MessageStream.tsx (Low Priority)
**Events to listen for:**
- `message_sent` - Add to message list

**Status:** ⏳ Pending

#### 5. LogsExport Component (Low Priority)
**Events to listen for:**
- `logs_exported` - Show success notification

**Status:** ⏳ Pending

---

## Phase 3: External CLI Support ⏳ NOT STARTED

### Requirements
1. Parse CLI args in main.rs
2. Execute commands on app startup
3. Support --debug flag
4. Handle --help and --version
5. Initialize UI based on CLI state

**Status:** ⏳ Not started

---

## Phase 4: Single-Instance IPC ⏳ NOT STARTED

### Requirements (from SINGLE_INSTANCE_IPC.md)
1. Add `tiny_http` dependency
2. Implement lock file management (`~/.agentmux/desktop.lock`)
3. Implement stale lock detection
4. Add IPC HTTP server in main.rs setup()
5. Implement instance detection on CLI startup
6. Implement command forwarding via HTTP POST
7. Add window focus/show on IPC request
8. Handle timeouts and errors

**Status:** ⏳ Not started

---

## Testing Checklist

### Phase 1 ✅
- [x] Rust code compiles
- [x] Events are emitted (verify via browser devtools)
- [x] Event payloads match spec

### Phase 2 ⏳
- [ ] Agent spawning reflects in UI instantly
- [ ] Bus start/stop updates UI
- [ ] CLI commands show feedback
- [ ] Multiple rapid operations don't cause race conditions
- [ ] Event cleanup on component unmount

### Phase 3 ⏳
- [ ] CLI → UI sync works
- [ ] External CLI launch initializes UI correctly
- [ ] Debug output shows in terminal

### Phase 4 ⏳
- [ ] Single instance detection works
- [ ] IPC command forwarding works
- [ ] Window focuses on IPC command
- [ ] Stale lock detection works
- [ ] Timeout handling works

---

## Known Issues

None currently.

---

## Next Steps

1. **Immediate:** Update `AgentsManager.tsx` to listen for `agent_spawned` event
2. **Next:** Update `Dashboard.tsx` and `BusControl.tsx` for bus events
3. **Later:** Implement external CLI support
4. **Final:** Implement single-instance IPC

---

## Notes

- All events use snake_case naming convention
- Events include full context (no partial updates)
- Polling fallback (5s) ensures state reconciliation
- Components must call unlisten() on cleanup to prevent memory leaks
