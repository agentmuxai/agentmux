# SPEC: Persistent Shell Node — Phase 3 (Stop / Lifecycle)

**Date:** 2026-06-14
**Status:** Draft → implementing
**Builds on:** `SPEC_PERSISTENT_SHELL_NODE_2026_06_11.md` (Phases 1–2 merged: #1338, #1356) and the MSYS cwd fix (#1415).

---

## 1. Problem

Phases 1–2 let an agent launch a long-running process (`task dev`, vite, watchers) via the `Shell` MCP tool; it streams into a colored `PersistentShellBlock` row. But **there is no way to stop it**:

- No `ShellStop` tool, no stop endpoint, no process registry.
- `PersistentShellBlock` has no stop button (the Phase-1 spec promised `■`).
- `ShellNodeRunner` is fire-and-forget; nothing holds a kill handle.
- The `"stopped"` status exists in the type but is never produced.

Consequences observed in the field (2026-06-14, agent "Mazs"): an agent with no clean way to stop its own launch resorted to **`taskkill task.exe`**, which — because `task.exe` is shared across instances/peers — killed *other* instances' dev servers and itself. A self-stop handle removes the entire reason to reach for `taskkill`.

Additionally, `tokio::process::Child::kill` / `kill_on_drop` only kill the **direct child** (`cmd /C` / `sh -c`), not the process *tree* (`task dev` → `task.exe` → `cargo`/`node`…). A real stop must kill the tree.

---

## 2. Goals

- Agent can stop a shell it started: `ShellStop(shell_id)`.
- User can stop a running shell from the UI: a `■` button on the running `PersistentShellBlock`.
- Stop kills the **whole process tree** (not just the wrapper shell), so `task dev`/vite actually terminate.
- The row transitions to **`stopped`** status (grey) — distinct from `exited-err`.
- Baseline orphan protection: `kill_on_drop` + registry so shells don't leak on srv shutdown.

## 3. Non-Goals (follow-ups)

- `ShellInput(shell_id, text)` (stdin) — Phase 3b.
- `ShellStatus(shell_id)` query tool — Phase 3b.
- Pane-close cleanup integration with `process_tracker` kill-tree — Phase 3c (registry is in place; wiring the host-exit/pane-close hook is separate).
- PTY / live-output fidelity for `\r` progress bars — separate track.

---

## 4. Design

### 4.1 Backend — `ShellSessionRegistry`

New registry in `AppState` (mirrors `InstallSessionRegistry`), mapping `shell_id → oneshot::Sender<()>`:

```rust
#[derive(Default)]
pub struct ShellSessionRegistry {
    shells: Mutex<HashMap<String, oneshot::Sender<()>>>,
}
impl ShellSessionRegistry {
    fn insert(&self, shell_id, tx);
    pub fn stop(&self, shell_id) -> bool;   // remove + send(()) → triggers tree-kill
    fn remove(&self, shell_id);             // idempotent (natural exit)
    pub fn stop_all(&self);                 // shutdown cleanup
}
```

### 4.2 `ShellNodeRunner` changes (`backend/shell_node.rs`)

- Spawn with a process group on Unix (`#[cfg(unix)] cmd.process_group(0)`) and `kill_on_drop(true)`.
- Register a `cancel_tx` keyed by `shell_id`; capture `child.id()` (pid).
- A small `kill_task` awaits `cancel_rx`:
  - `Ok(())` → **stop requested** → `kill_tree(pid)` → `was_stopped = true`.
  - `Err(_)` (sender dropped on natural exit) → `was_stopped = false`.
- Main read loop is unchanged; when the child dies (naturally or killed) its pipes close, the loop ends, then `child.wait()`.
- On loop end, `registry.remove(shell_id)` (drops the sender so `kill_task` resolves on natural exit), `await kill_task` for `was_stopped`.
- `publish_exit` gains a `stopped: bool` field.

`kill_tree(pid)`:
- **Windows:** `taskkill /PID <pid> /T /F` (kills the tree — the same mechanism, used correctly and scoped to *our* pid, not by image name).
- **Unix:** `kill(-pgid, SIGTERM)` then `SIGKILL` after a short grace.

### 4.3 Endpoints

- **HTTP** `POST /api/v1/shell/stop` `{ shell_id }` → `state.shell_sessions.stop(shell_id)` (for the MCP `ShellStop` tool). Auth-gated like `/shell/create`.
- **WS RPC** `shellstop` `{ shell_id }` → same registry call (for the UI stop button).

Both are thin wrappers over `ShellSessionRegistry::stop`.

### 4.4 MCP — `agentmux-mcp`

Add `ShellStop` tool:
```json
{ "name": "ShellStop",
  "description": "Stop a running shell started by Shell(). Kills the whole process tree.",
  "inputSchema": { "type": "object", "properties": { "shell_id": {"type":"string"} }, "required": ["shell_id"] } }
```
Handler POSTs `{ shell_id }` to `/api/v1/shell/stop`.

### 4.5 Frontend

- `RpcApi.ShellStopCommand({ shell_id })` → WS `shellstop`.
- `PersistentShellBlock`: render a `■` stop button when `status === "running"`; `onClick` → `ShellStopCommand`, `stopPropagation` (don't toggle expand). Optimistic: leave status to the `exit` event.
- `useAgentStream` shell `exit` handler: when `stopped === true`, dispatch `ShellStatusUpdate` with status `"stopped"` instead of `exited-ok/err`. (Reducer already supports `"stopped"`.)

---

## 5. Files

| File | Change |
|------|--------|
| `agentmux-srv/src/server/shell_handlers.rs` *(new)* or `server/mod.rs` | `ShellSessionRegistry` + `/api/v1/shell/stop` handler |
| `agentmux-srv/src/server/mod.rs` | add `shell_sessions` to `AppState`; pass to runner; register route |
| `agentmux-srv/src/main.rs` | construct `shell_sessions` |
| `agentmux-srv/src/backend/shell_node.rs` | registry hook, `kill_tree`, process group, `stopped` in exit |
| `agentmux-srv/src/server/websocket.rs` | `shellstop` WS RPC handler |
| `agentmux-mcp/src/main.rs` | `ShellStop` tool + handler |
| `frontend/app/store/rpc-api.ts` | `ShellStopCommand` |
| `frontend/app/view/agent/components/PersistentShellBlock.tsx` | `■` stop button |
| `frontend/app/view/agent/useAgentStream.ts` | `stopped` → status `"stopped"` |

## 6. Test / Verify

- Unit: `ShellSessionRegistry` stop/remove idempotency.
- Live (dev build): `Shell("task dev")` → row green/running → click `■` → tree dies (no orphan `task.exe`/node), row turns grey `stopped`. Agent `ShellStop(shell_id)` does the same.
- Confirm no leftover `node`/`task.exe` after stop (the orphan check that bit us earlier).

## 7. Changeset

`patch` — additive feature, no schema break.
