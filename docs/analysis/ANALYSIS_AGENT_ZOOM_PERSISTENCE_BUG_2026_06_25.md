# Bug Report — Agent Pane Zoom Persistence Not Working

**Date:** 2026-06-25
**Status:** Root cause confirmed, fix identified
**Area:** Agent pane / per-agent zoom storage

---

## Summary

Two PRs shipped zoom persistence (#1700, #1754) yet reopening an agent always
resets to 1.0×. The save path silently does nothing because the frontend's zoom
write goes through the WebSocket `setmeta` handler, which calls `update_object_meta`
directly — the function that **only writes the block row and returns**. The zoom
mirror that copies `term:zoom` → `ui:zoom` lives exclusively in the HTTP
`UpdateObjectMeta` handler and is never reached.

---

## The Two PRs and What They Claimed to Fix

### PR #1700 — `feat(agent): persist per-agent zoom across pane close/reopen`

Added the full save/restore plumbing:

- **Restore on open** (`app_api.rs:341-349`): `agent.open` reads `ui:zoom` from
  the per-agent content store and seeds the new block's `term:zoom`.
- **Save on change** (`service.rs:605-622`): the `UpdateObjectMeta` HTTP handler
  calls `schedule_agent_zoom_mirror` whenever a block's `term:zoom` key changes.
- **Debounce** (`service.rs:2831-2871`): `schedule_agent_zoom_mirror` waits 300ms
  and coalesces bursts into a single `agent_content_set("ui:zoom", ...)`.

### PR #1754 — `fix(zoom): persist zoom for cross-channel agents via global registry`

Fixed a secondary issue where `agent_content_set` failed with an FK error for
cross-channel agents not present in `db_agent_definitions`. Added a separate path
through `registry_def_update_content_field` for those agents.

Neither PR noticed that the frontend never exercises the HTTP `UpdateObjectMeta`
handler for zoom writes.

---

## Root Cause

### Two independent paths for `SetMeta` — only one has the mirror

There are two server-side handlers that mutate object metadata:

| Path | Handler | Trigger | Has zoom mirror? |
|------|---------|---------|-----------------|
| HTTP (legacy wave protocol) | `"UpdateObjectMeta"` in `service.rs:537` | registered in `mod.rs:703,770` | **YES** (`service.rs:605-622`) |
| WebSocket (wshrpc/wshnet) | `"setmeta"` in `websocket.rs:565` | registered in WebSocket engine | **NO** |

### The frontend uses the WebSocket path

`zoom.win32.ts` (and `.darwin.ts`, `.linux.ts`) call:

```typescript
// zoom.win32.ts:76-80
fireAndForget(() =>
    RpcApi.SetMetaCommand(TabRpcClient, {
        oref: WOS.makeORef("block", blockId),
        meta: { "term:zoom": metaValue },
    })
);
```

`RpcApi.SetMetaCommand` → `client.rpcCall("setmeta", ...)` → WebSocket.

The WebSocket `setmeta` handler (`websocket.rs:565-601`) does:

```rust
update_object_meta(&wstore, &oref_str, &cmd.meta)?;
// broadcast WaveObjUpdate
// return Ok(None)
```

`update_object_meta` (`service.rs:2777-2826`) writes the block row and returns.
There is no zoom mirror call anywhere in this path.

### What the zoom mirror call looks like in the HTTP path (never reached)

```rust
// service.rs:605-622 — inside the "UpdateObjectMeta" HTTP handler
if oref.otype == OTYPE_BLOCK && meta_update.contains_key("term:zoom") {
    if let Ok(block) = store.must_get::<Block>(&oref.oid) {
        let agent_id = block.meta.get("agentId")
            .and_then(|v| v.as_str()).unwrap_or("").to_string();
        if !agent_id.is_empty() {
            let zoom = meta_update.get("term:zoom").and_then(|v| v.as_f64());
            schedule_agent_zoom_mirror(store.clone(), agent_id, zoom);
        }
    }
}
```

This code is correct and complete. It just never runs.

---

## Full Call Chain (Current, Broken)

```
User: Ctrl+Wheel on agent pane
  ↓
zoomBlockIn/Out  (zoom.win32.ts)
  ↓
setBlockZoom  (zoom.win32.ts:67-82)
  ↓
RpcApi.SetMetaCommand(TabRpcClient, { oref: "block:<id>", meta: { "term:zoom": 1.3 } })
  ↓
WebSocket rpcCall("setmeta", ...)
  ↓
websocket.rs:565 — setmeta handler
  ↓
update_object_meta(&wstore, "block:<id>", { "term:zoom": 1.3 })
  ↓
[writes block row, returns — NO ZOOM MIRROR]
  ↓
broadcast WaveObjUpdate → frontend sees new zoom, renders correctly

--- pane closes ---

block deleted → term:zoom gone
agent_content "ui:zoom" row: never written ← BUG

--- agent.open ---

app_api.rs:346: agent_content_get(agent.id, "ui:zoom") → None
term:zoom not seeded → new block defaults to 1.0
```

---

## Why It Went Unnoticed

The restore half (`app_api.rs:341-349`) and debounce/mirror infrastructure
(`schedule_agent_zoom_mirror`) were built and unit-tested in isolation. The
integration test that would catch it — "zoom, close, reopen, assert zoom
restored" — was not written. Because the block-level `term:zoom` persists within
a session (the block exists until the pane is closed), the feature appears to
work during a session; the break only surfaces on the close→reopen cycle.

PR #1754 then fixed a secondary failure in the cross-channel write path — a real
bug, but one that can only be triggered after the primary mirror is actually
called, which it never is.

---

## Fix

Add the zoom mirror call to the WebSocket `setmeta` handler
(`agentmux-srv/src/server/websocket.rs`), immediately after the
`update_object_meta` call:

```rust
// websocket.rs — inside the "setmeta" async handler, after update_object_meta:

// Per-agent zoom persistence: mirror term:zoom → ui:zoom (same logic as
// the UpdateObjectMeta HTTP handler in service.rs).
let oref_parsed = crate::backend::ORef::parse(&oref_str)
    .map_err(|e| e.to_string())?;
if oref_parsed.otype == "block" && cmd.meta.contains_key("term:zoom") {
    if let Ok(block) = wstore.must_get::<Block>(&oref_parsed.oid) {
        let agent_id = block.meta.get("agentId")
            .and_then(|v| v.as_str()).unwrap_or("").to_string();
        if !agent_id.is_empty() {
            let zoom = cmd.meta.get("term:zoom").and_then(|v| v.as_f64());
            crate::server::service::schedule_agent_zoom_mirror(
                wstore.clone(), agent_id, zoom,
            );
        }
    }
}
```

`schedule_agent_zoom_mirror` is already `pub(crate)` accessible. The WebSocket
handler is async so `tokio::spawn` inside the function works fine.

Alternatively, move the mirror logic into `update_object_meta` itself — but that
function is synchronous and takes `&Store` (not `Arc<Store>`), so spawning would
require an API change. The handler-level fix is lower risk.

---

## Affected Files

| File | Role |
|------|------|
| `agentmux-srv/src/server/websocket.rs:565` | `setmeta` handler — where fix goes |
| `agentmux-srv/src/server/service.rs:537-622` | `UpdateObjectMeta` HTTP handler — has the mirror, but never called by frontend |
| `agentmux-srv/src/server/service.rs:2831-2871` | `schedule_agent_zoom_mirror` — correct, unused |
| `agentmux-srv/src/server/app_api.rs:341-349` | Restore on `agent.open` — correct, would work once mirror is fixed |
| `agentmux-srv/src/backend/storage/content.rs` | `agent_content_get/set` — correct |
| `frontend/app/store/zoom.win32.ts:67-82` | `setBlockZoom` — uses WebSocket, correct |
