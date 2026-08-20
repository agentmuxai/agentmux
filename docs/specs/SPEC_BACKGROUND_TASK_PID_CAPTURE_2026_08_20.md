# Spec: Background Task PID Capture (Phase A)

**Date:** 2026-08-20
**Author:** AgentA
**Status:** Proposed
**Depends on:** PR #2590 (`db_background_tasks` registry, merged 2026-08-16)
**Blocks:** `SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md` (Phase B), `SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md` (Phase C)

## 1. Problem

`db_background_tasks` (schema v22, `agentmux-srv/src/backend/storage/background_tasks.rs`) has a `pid` column and a `background_task_set_pid(id, pid)` method, but the method is **dead code in production** — grep for its only 3 call sites in the whole repo finds the definition plus two calls inside its own `#[cfg(test)] mod tests`. Every row written via the live `docknodestatus`/`COMMAND_BACKGROUND_TASK_COMPLETION` handlers (`websocket.rs:1073`, `:1134`) has `pid = NULL` forever.

Without a real PID, nothing downstream can act on a specific background task:
- Phase B (teardown survival) needs a PID to know which process to exempt/adopt.
- Phase C (dashboard intelligence) needs a PID to distinguish "the registry says running" from "the OS process actually still exists" (the row could be stale if bashwrap crashed without sending its completion notification).
- Any future explicit "stop this background task" UI action needs a PID to kill the right thing.

## 2. Where the PID actually becomes known

The task's OS process is `agentmux-bashwrap exec` (a disposable, one-shot process per `main.rs`'s own doc comment — "there is no daemon"). Today:

- `bash_wrap.rs::run_via_pty` captures `child_pid = child.process_id()` (`bash_wrap.rs:967`) — the PID of the *inner* `bash -c <command>` process it spawns via PTY, not bashwrap's own PID.
- The frontend never sees any PID at all — `isAcceptedBackgroundLaunch` (`tool-adapter.ts:94-98`) only sees the tool result text (`"Command running in background with ID: <id>"`), where `<id>` is Claude Code's own opaque background-shell handle, not an OS PID.

**Decision: capture bashwrap's own PID (the wrapper process), not the inner bash child's PID.** Rationale:
- Phase B's breakaway design (see the teardown-survival spec) operates on bashwrap's own spawn, since bashwrap is the one AgentMux-controlled process in the chain — it is the natural attachment point for "this is the root of a background task's tree" bookkeeping, and Phase B's Windows breakaway must happen at bashwrap's own re-spawn point regardless.
- The inner bash PID is:
  - Not known to anything outside `bash_wrap.rs` itself.
  - Windows: `portable-pty`'s `ChildKiller` doesn't expose it for tree operations beyond what `kill_process_tree` (bash_wrap.rs:352) already does internally.
- Bashwrap's own PID is visible to its parent (the `claude` CLI process) and, more importantly, to `agentmux-srv` via the same channel that already threads `AGENTMUX_BLOCKID`/`AGENTMUX_AUTH_KEY` env vars into the spawn — i.e., srv can be told "here is the wrapper's own PID" via a WPS chunk at spawn time, symmetric to how `tool_id`/`block_id` already reach `websocket.rs`'s handlers.

## 3. Design

### 3.1 bashwrap reports its own PID at spawn

`bash_wrap.rs::run()` (`bash_wrap.rs:429`), immediately after the existing `publish_system(..., "[bashwrap] starting: N chars")` call (`bash_wrap.rs:467-479`), adds one more WPS publish only when `args.declared_background` is true:

```rust
if args.declared_background {
    if let Some(client) = wps.as_ref() {
        let _ = publish_pid(client, &args.tool_id, args.block_id.as_deref(), std::process::id()).await;
    }
}
```

New `ChunkMessage`-sibling wire type (mirrors `TerminalMessage`'s shape):

```rust
#[derive(Serialize)]
struct PidMessage<'a> {
    op: &'static str, // "pid"
    tool_id: &'a str,
    pid: u32,
    timestamp: u64,
}
```

Gated on `declared_background` specifically (not every bashwrap invocation) because:
- `db_background_tasks` rows only ever exist for `run_in_background: true` accepted launches (per `websocket.rs:1045-1050`'s existing gate) — publishing a PID for every ordinary Bash call would be pure overhead with no reader.
- `args.declared_background` is already threaded through from the hook (`hook.rs`, per the 08-15 status doc §2.1's finding that the raw `run_in_background` field is already available at the hook boundary) — no new plumbing needed to know this at the point of the new publish call.

### 3.2 srv relays the PID into the registry

New WPS-consumed command, `COMMAND_BACKGROUND_TASK_PID` (mirrors the existing `COMMAND_BACKGROUND_TASK_COMPLETION` pattern at `websocket.rs:1105-1145` — a dedicated command rather than folding into `docknodestatus`, for the same reason #2590 kept completion separate: `docknodestatus`'s `push_delta` fully overwrites a node's dock snapshot and this event has no `tool_name`/`run_in_background` to preserve).

Handler calls `background_task_set_pid(id, pid)` where `id` is the tool_id (matches `db_background_tasks.id`, per the existing convention that `id` mirrors the frontend's `tool_use_id`).

Frontend change: `tool-adapter.ts` needs a new small case in whatever already parses bashwrap's WPS chunk stream (the existing `kind: "system"|"stdout"|"stderr"` handling plus the pre-existing `terminal` op handling for exit code) to recognize `op: "pid"` and forward it as a new outgoing RPC command, `CommandBackgroundTaskPid`, symmetric to the existing `CommandBackgroundTaskCompletionData` (`gotypes.d.ts:208-222`). This is the one frontend-touching piece of this otherwise backend-only phase — flagged explicitly since PR #2590's commit message noted pride in needing "no frontend changes," this phase does need one, minimal, wire-shape-only change.

### 3.3 Race handling

`background_task_observe` (the `INSERT OR IGNORE` + conditional `UPDATE`) and this new PID-set call are not ordered relative to each other by any lock — the accepted-launch `docknodestatus` push and bashwrap's own PID publish are two independent async paths racing to reach srv first. `background_task_set_pid`'s existing implementation (`background_tasks.rs:104-111`) is a bare `UPDATE ... WHERE id = ?2` with no existence check beyond the `WHERE` clause matching zero rows silently — already safe for "PID arrives before the observe row exists" (no-op, PID is lost) and "observe arrives first, then PID" (normal case). The already-lost-PID case (wrapper's publish raced ahead of the frontend's accepted-launch push landing) is handled by having bashwrap **retry the PID publish once, after a short delay** if the initial publish happens before `block_id` is even resolvable — in practice this should be rare enough (the front029-side accept signal fires as soon as the tool result text is seen, generally before bashwrap's own async publish task even schedules) that a single best-effort retry is sufficient; this is not a correctness-critical path (Phase B does not strictly require the PID to always land — see that spec's fallback behavior).

## 4. Testing

- `background_tasks.rs`: existing tests already cover `set_pid`'s DB-layer behavior — no new DB-layer tests needed, just confirming the production call path now exists.
- `bash_wrap.rs`: unit test that `declared_background: true` triggers a `PidMessage` publish with `std::process::id()`; `declared_background: false` does not.
- `websocket.rs`: handler test for `COMMAND_BACKGROUND_TASK_PID` — asserts `background_task_get(id).pid` reflects the published value after the command is processed.
- Integration: none required for this phase alone (no OS-level behavior changes) — folded into Phase B's live-verify pass instead, since PID capture has no observable effect on its own until Phase B consumes it.

## 5. Non-goals

- No change to bashwrap's idle-timeout behavior (already handled, #2491/PR #2589).
- No change to teardown/kill behavior (Phase B).
- No frontend UI changes beyond the wire-shape relay described in §3.2 (Phase C).
