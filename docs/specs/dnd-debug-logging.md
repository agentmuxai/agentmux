# Drag-and-Drop Debug Logging Spec

## Problem

File drag-and-drop into terminal panes shows "No working directory detected"
even after the `cmd:cwd` seeding fix. The fix added a `waveobj:update` broadcast
after writing to the store, but we can't tell where the chain is breaking.

---

## Data Flow (what needs to work)

```
User opens terminal pane
  │
  ▼
Frontend: TermWrap.resyncController()
  └─► RPC: ControllerResyncCommand ──────────────────────────────────► Backend
                                                                           │
                                                             resync_controller()
                                                                           │
                                                         ShellController::start()
                                                                           │
                                                         ShellController::run_inner()
                                                                           │
                                                              [shell spawns]
                                                                           │
                                                    seed cmd:cwd in store  │
                                                    broadcast waveobj:update│
                                                                           │
Frontend: EventBus listener ◄────────────────── waveobj:update event ─────┘
  │
  ▼
Jotai atom: WOS.getWaveObjectAtom("block:<id>") updates
  │
  ▼
React re-render: blockData.meta["cmd:cwd"] is set
  │
  ▼
User drags file → overlay shows "Copy to <cwd>"
  │
  ▼
File dropped → invoke("copy_file_to_dir", ...)
```

## Known Failure Modes

### A. Seeding never runs (wstore is None)
The ShellController is constructed with `wstore: Option<Arc<WaveStore>>`.
If `wstore` is `None` at construction time, the entire seeding block is skipped silently.

### B. Shell already running — spawn path skipped
`resync_controller` short-circuits if a controller is already in `STATUS_RUNNING`:
```rust
return Ok(());  // no spawn, no seed
```
For a terminal opened before our fix was deployed, the spawn already happened.
Opening a NEW pane after the fix is deployed should work; an existing running pane won't.

### C. store.must_get fails silently
```rust
Err(e) => tracing::warn!(...)  // logged but flow continues without seeding
```

### D. update_object_meta fails silently
```rust
Err(e) => tracing::warn!(...)  // logged but broadcast never happens
```

### E. event_bus is None
The controller might be created without an `event_bus` in some startup paths.
Seeding succeeds (store updated) but no broadcast sent — frontend never sees the value.

### F. waveobj:update event not reaching frontend
The EventBus broadcasts to registered WebSocket connections. If the frontend
WebSocket isn't registered with the event bus at the time of broadcast, the
event is dropped.

### G. Frontend atom doesn't handle waveobj:update for blocks
The Jotai WOS atom subscription might not be active yet when the event fires
(race condition: atom created after event sent).

### H. cmd:cwd already set (stale value from old session)
The seeding guard `if cmd_cwd_is_empty` prevents overwriting.
If a stale CWD from a previous session is in the DB, it won't be refreshed.
But then the frontend should show it... unless the old value isn't being loaded.

---

## Logging Plan

### 1. Backend: tracing in shell.rs (already added, need to verify reaching logs)

Add explicit `tracing::info!` at each decision point so we can trace in log files:

```rust
// After spawn, before seeding:
tracing::info!(block_id = %self.block_id, wstore_present = self.wstore.is_some(), event_bus_present = self.event_bus.is_some(), "pre-seed state");

// If wstore is None:
tracing::warn!(block_id = %self.block_id, "wstore is None — cmd:cwd seed skipped entirely");

// When block read succeeds:
tracing::info!(block_id = %self.block_id, existing_cwd = %existing, "block read for seed check");

// When update succeeds:
tracing::info!(block_id = %self.block_id, cwd = %effective_cwd, "cmd:cwd written to store");

// When broadcast fires:
tracing::info!(block_id = %self.block_id, "waveobj:update broadcast sent for cmd:cwd");
```

**Log location:** `~/.agentmux/logs/agentmuxsrv-rs.log.*` (check exact path)

### 2. Backend: tracing in resync_controller (mod.rs)

Log the wstore presence at the resync entry point:
```rust
tracing::info!(
    block_id = %block_id,
    controller_type = %controller_type,
    wstore_present = wstore.is_some(),
    event_bus_present = event_bus.is_some(),
    "resync_controller entry"
);
```

Log the short-circuit path:
```rust
// When already running:
tracing::info!(block_id = %block_id, "controller already running — skipping spawn and seed");
return Ok(());
```

### 3. Frontend: console.log in term.tsx

Add to the render section so we see meta on every render while dragging:

```typescript
// In the component body, near the cwd check:
const { isDragOver, handlers: dropHandlers } = useFileDrop(handleFilesDropped);
const cwd = blockData?.meta?.["cmd:cwd"];

// Temporary debug:
React.useEffect(() => {
    if (isDragOver) {
        console.log("[dnd-debug] drag over — blockData.meta:", JSON.stringify(blockData?.meta ?? {}));
        console.log("[dnd-debug] cmd:cwd =", cwd ?? "(undefined)");
    }
}, [isDragOver, blockData, cwd]);
```

### 4. Frontend: log waveobj:update reception

In the WOS atom subscription (wherever `waveobj:update` events are consumed),
add a log when a block update arrives:

```typescript
// Look for the waveobj:update handler in wos.ts or global.ts
console.log("[wos] waveobj:update received for", oref, "keys:", Object.keys(update?.meta ?? {}));
```

### 5. Frontend: log on ControllerResyncCommand send

In `termwrap.ts` before calling `ControllerResyncCommand`:
```typescript
console.log("[termwrap] sending ControllerResyncCommand for block", this.blockId, "tab", tabId);
```

---

## How to Read the Logs

### Backend logs
```bash
# Find log file
ls ~/.agentmux/logs/

# Tail for dnd-related entries
tail -f ~/.agentmux/logs/agentmuxsrv-rs.log.* | grep -E "seed|cmd:cwd|wstore|resync"
```

### Frontend logs
Open DevTools in the dev instance:
- Windows: `Ctrl+Shift+I` in the AgentMux window (if enabled in dev mode)
- Or: look for a "Open DevTools" menu option

Filter console by `[dnd` or `[wos` or `[termwrap`.

---

## Diagnostic Decision Tree

```
Is "pre-seed state" logged?
  NO  → ShellController::run_inner not reached (spawn failed or wrong code path)
  YES → Is wstore_present = true?
          NO  → wstore is None, seeding can't happen (bug in controller construction)
          YES → Is "cmd:cwd written to store" logged?
                  NO  → block read failed or existing_cwd was not empty
                  YES → Is "waveobj:update broadcast sent" logged?
                          NO  → event_bus is None
                          YES → Is frontend receiving waveobj:update?
                                  NO  → EventBus → WebSocket delivery broken
                                  YES → Is atom updating?
                                          NO  → WOS atom subscription bug
                                          YES → React not re-rendering (stale closure?)
```

---

## Implementation Plan

| Step | File | Change | Notes |
|------|------|--------|-------|
| 1 | `blockcontroller/shell.rs` | Add info logs at pre-seed, seed-skip, store-write, broadcast | Already partially done |
| 2 | `blockcontroller/mod.rs` | Log wstore/event_bus presence at resync_controller entry; log short-circuit | 5 min |
| 3 | `frontend/app/view/term/term.tsx` | Add `useEffect` logging on isDragOver | 5 min |
| 4 | `frontend/app/store/wos.ts` | Log incoming waveobj:update events | Find the handler first |
| 5 | `frontend/app/view/term/termwrap.ts` | Log ControllerResyncCommand sends | 2 min |

Once we have all 5 logs active, a single "open terminal, drag file" test will tell
us exactly which step in the chain is failing.

---

## Alternative: Force-Seed on Drag Start

If the log investigation is slow, an immediate workaround:

When the drag starts and `cmd:cwd` is missing, query the backend for the shell's
actual CWD using `wsh` or a new RPC command, rather than relying on the spawn-time seed.

This is more robust anyway since the spawn-time seed uses the backend's CWD as a
fallback, not the shell's actual interactive CWD.

**New RPC command:** `GetShellCwd(blockId) -> string`
- Backend queries `/proc/{pid}/cwd` (Linux/macOS) or `NtQueryInformationProcess` (Windows)
- Or: read from the shell's environment via PTY injection (less reliable)
- Frontend calls this when `isDragOver && !cwd`
