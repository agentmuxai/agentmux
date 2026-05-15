# SPEC: Reactive Workspace + Object Sync (frontend reactivity gap)

**Date:** 2026-05-14
**Author:** AgentX
**Status:** Draft
**Reported by:** user — "when I update the starter workspace to a new string, it doesnt update the task bar or even the status bar version panel"
**Related:** PR #841 (window-title format), `SPEC_WINDOW_INSTANCE_NAMING_CLEANUP_2026-05-14.md`

---

## 1. Problem

Renaming a **workspace** does not propagate reactively to:

- The OS taskbar / window title (`document.title` set by `installWindowTitleEffect()` in `app-init.ts`)
- The bottom-right **InstancePanel**'s window-list rows (`resolveName()` in `InstancePanel.tsx`)

Both surfaces are *supposed* to react via `atoms.workspace()?.name` / `getObjectValue<Workspace>(...)?.name` — and they would, if the workspace cache received an update event. It doesn't.

This is a single bug shaped two ways: a per-RPC defect (workspace mutations don't broadcast) and a class-of-bug shaped like "the convention to attach `WaveObjUpdate` to the response is easy to forget for new mutation RPCs."

For comparison: **window display-name renames work fine** — they go through a different RPC (`ObjectService.UpdateObjectMeta`) that *does* attach updates to the response.

---

## 2. Root cause

### 2.1 The signal-death chain

| # | Layer | File:line | What happens |
|---|-------|-----------|--------------|
| 1 | Frontend mutation call | `frontend/app/store/services.ts:172-173` | `WorkspaceService.UpdateWorkspace(workspaceId, name)` over RPC |
| 2 | Backend RPC handler | `agentmux-srv/src/server/service.rs:1274-1307` | Dispatches `RenameWorkspace` → reducer; applies emitted `WorkspaceRenamed` event to SQLite via `apply_event_to_wstore` (line 1298); publishes event to internal `srv_events_tx` channel (line 1305); **returns `WebReturnType::success_empty()` (line 1306)** ← bug |
| 3 | Response broadcast loop | `agentmux-srv/src/server/service.rs:39-52` | Iterates `result.updates` and broadcasts each to all connected WS clients as `waveobj:update`. **Empty response → nothing to broadcast.** |
| 4 | Frontend WS event handler | `frontend/app/store/wos.ts:259-279` | `updateWaveObject()` would call `wov.setData()`, triggering SolidJS signal reactivity. **Never invoked because no event arrives.** |
| 5 | Reactive consumers | `frontend/app/store/global.ts:62-66` (`atoms.workspace()`), `frontend/app-init.ts:518-544` (title effect), `frontend/app/statusbar/InstancePanel.tsx:142-153` (panel `resolveName`) | All depend on the WOS-cached `Workspace` signal. **Signal never changes → memos/effects never re-run.** |

### 2.2 Why `window:displayname` works (the working comparison)

`InstancePanel.tsx:171-175` calls:
```ts
ObjectService.UpdateObjectMeta(makeORef("window", windowId), { "window:displayname": name })
```

Backend handler `agentmux-srv/src/server/service.rs:308-390` returns:
```rust
WebReturnType::success_with_updates(vec![WaveObjUpdate {
    updatetype: "update".into(),
    otype: OTYPE_WINDOW.to_string(),
    oid: oref.oid.clone(),
    obj: Some(wave_obj_to_value(&window)),
}])
```

The inline `WaveObjUpdate` triggers the broadcast loop, which fires `waveobj:update` to every connected client, which updates the WOS cache, which trips the reactive signal. The whole chain works because the response carries the change.

**The convention for mutation RPCs is: write to store + publish events + return updated objects in response. UpdateWorkspace skipped step 3.**

---

## 3. Scope — confirmed and likely-broken

### 3.1 Confirmed broken

- `UpdateWorkspace` (`agentmux-srv/src/server/service.rs:1274-1307`) — workspace name rename. The reported case.

### 3.2 Suspected broken (audit needed)

Any backend mutation RPC handler that publishes events to `srv_events_tx` but does not also return a `WaveObjUpdate` in the response will exhibit the same bug. Audit by grepping handlers that:
- call `publish_events()` or write to `srv_events_tx`, and
- end with `WebReturnType::success_empty()`

Likely suspects (by name only — needs verification):
- Workspace tab list mutations (`AddTab`, `RemoveTab`, `ReorderTab` on workspace)
- Workspace `activetabid` change (when user switches tabs — the title's `atoms.activeTabId()` needs this to update)
- Other workspace meta updates beyond `name`
- Tab renames (`TabRename` if it exists; if it goes through `UpdateObjectMeta` it's fine, but if it has a dedicated handler it might be broken)

### 3.3 Confirmed working (do not touch)

- `UpdateObjectMeta` for `window:displayname` — returns inline update.
- Block updates via `UpdateObjectMeta` — same path.

---

## 4. Fix options

### 4.1 Option A — per-handler fix (surgical)

For each broken handler, add the inline update to the response.

```rust
// agentmux-srv/src/server/service.rs:1274-1307 — UpdateWorkspace
// AFTER the existing publish_events() call:

let updated_ws = state.wstore.lock().unwrap().get_workspace(&workspace_id).cloned();
match updated_ws {
    Some(ws) => WebReturnType::success_with_updates(vec![WaveObjUpdate {
        updatetype: "update".into(),
        otype: OTYPE_WORKSPACE.to_string(),
        oid: workspace_id.clone(),
        obj: Some(wave_obj_to_value(&ws)),
    }]),
    None => WebReturnType::success_empty(),
}
```

**Pros:** Minimal, surgical, easy to review per handler.
**Cons:** The next person who adds a mutation RPC and forgets the convention reintroduces this exact bug in a new place. Doesn't catch the class.

### 4.2 Option B — broadcast bridge (architectural)

Bridge `srv_events_tx` → frontend `waveobj:update` so any event published internally automatically becomes a frontend update event. Make the per-handler convention a belt instead of a suspender.

Sketch:
1. New task in `agentmux-srv/src/server/service.rs` subscribes to `srv_events_tx` at startup.
2. For each event type that affects a `WaveObj` (`WorkspaceRenamed`, `WorkspaceTabAdded`, `WorkspaceTabRemoved`, `TabRenamed`, `BlockUpdated`, …), translate to a `WaveObjUpdate` and broadcast to all WS clients via the same broadcast channel the response loop uses.
3. The per-handler "return updates in response" pattern stays for synchronous-RPC-completion cases, but it's no longer the only signal — the bridge guarantees that anything that changes in the store gets seen.

**Pros:** Fixes the whole class. New mutations automatically wire up. Self-healing.
**Cons:** Risk of double-broadcast (both the response loop AND the bridge fire) — needs dedup. Bigger surface change. Needs careful event-type-to-WaveObjUpdate mapping table.

### 4.3 Recommendation

**Ship Option A for `UpdateWorkspace` immediately** (one-line behavior fix, addresses the user-reported bug today). **Open a follow-up issue for Option B** with a complete audit of `srv_events_tx` event types that should fan out as `WaveObjUpdate`s, plus a dedup/idempotency story.

---

## 5. Test plan

### 5.1 Manual — for Option A

- [ ] Open AgentMux. Default workspace name visible somewhere (e.g. status bar via InstancePanel or settings panel).
- [ ] Rename the workspace to "Pulse" via the workspace settings UI.
- [ ] **Expected:** OS taskbar title updates from `Window 1 - <tab> - AgentMux` to `Pulse - <tab> - AgentMux` immediately.
- [ ] **Expected:** Open InstancePanel (click version chip). The window row shows `Pulse` (not `Window 1`).
- [ ] Rename back to empty / different. Title + panel update again.
- [ ] Rename a workspace assigned to a different (not-current) window. That window's row in InstancePanel updates; current window unaffected.

### 5.2 Automated

- [ ] Add a Rust unit test for `UpdateWorkspace` that inspects the returned `updates` field and asserts a `WaveObjUpdate` of `otype: "workspace"` with the new name in `obj`.
- [ ] (If time) frontend integration test: mock the WS channel, fire a `waveobj:update` for a workspace, assert `atoms.workspace()` re-derives.

### 5.3 Regression — confirm window:displayname unaffected

- [ ] InstancePanel double-click rename flow still updates title + panel (this is the path that was already working).

---

## 6. Out of scope (for this PR)

- Option B (broadcast bridge) — separate follow-up.
- Audit of every other `success_empty()`-returning mutation RPC — separate follow-up.
- The "starter workspace" terminology in the user's report — that's the workspace assigned at startup; no rename needed in code.
- Workspace.name validation (length cap, character set, etc.) — orthogonal.

---

## 7. Risk

- **Low risk** for Option A: adds a fetch + serialize after `publish_events()`. Same shape as the working `UpdateObjectMeta` handler. The fetch happens after the SQLite write completes, so there's no race.
- **One subtle correctness concern:** `state.wstore.lock().unwrap()` — if this lock is contested under load, the fetch could block briefly. Same lock is already taken inside `publish_events()` upstream, so no new deadlock surface; the brief hold is pre-existing.

---

## 8. Open questions

1. **Are there other "name-like" fields on workspace** (e.g. `description`, `icon`) that have the same RPC shape? If yes, the same fix pattern applies.
2. **Does `RenameTab` exist as a dedicated handler** or does it route through `UpdateObjectMeta`? If dedicated, audit it for the same bug.
3. **For the broadcast bridge (Option B):** which `Event::*` variants in the reducer should fan out as `WaveObjUpdate`? Need a mapping table — likely all the `*Renamed`, `*Updated`, `*Created`, `*Deleted` variants for objects with frontend-visible representations.
