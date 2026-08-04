# Retro: renaming a window via the raw `setmeta` RPC silently doesn't update the live UI

## What happened

Asked to rename "Window 1" to "agent3 work" using the app API, I connected
to the running `task dev` instance via CDP and called the frontend's
globally-exposed `window.RpcApi.SetMetaCommand(window.TabRpcClient, {
oref: "window:<id>", meta: {"window:displayname": "agent3 work"} })` —
the same generic meta-setter used throughout the codebase for per-block
settings (zoom, pane title, connection state).

The call resolved successfully, with no error. The window title never
changed. A full page reload confirmed the write hadn't even persisted to
`window:displayname` — `window.globalAtoms.waveWindow().meta` showed only
`{"host:label": "main"}`, no `window:displayname` key at all.

The user then pointed at `agentmux-docs` for "a fast way to do this" —
which surfaced the actual documented mechanism (`SetName` MCP tool /
`POST /api/v1/window/name`) and, in tracing why it differs from what I'd
used, surfaced a real architectural gap.

## Root cause

Two parallel "set a WaveObject's metadata" mechanisms exist in this
codebase, with different reactivity guarantees:

**1. Legacy: raw WS `setmeta` RPC** (`agentmux-srv/src/server/websocket.rs`,
`COMMAND_SET_META` handler — what `frontend`'s `RpcApi.SetMetaCommand`
calls). Writes directly via `update_object_meta`, then does its own
ad-hoc broadcast:

```rust
let update_data = if oref.otype == "block" {
    // ...fetch the block, embed it in the broadcast payload
} else { None };
event_bus.broadcast_event(&WSEventType {
    eventtype: "waveobj:update".to_string(),
    oref: oref_str,
    data: update_data,   // None for every otype except "block"
});
```

**2. Modern: HTTP `object.UpdateObjectMeta` service call**
(`agentmux-srv/src/server/service/object.rs`, what
`ObjectService.UpdateObjectMeta` / the documented `SetName` tool /
`POST /api/v1/window/name` all route through). Dispatches into the
reducer (`Command::UpdateWindowMeta` / `UpdateWorkspaceMeta` /
`UpdateTabMeta` / `UpdateBlockMeta`), persists synchronously, then
publishes the reducer's emitted event onto `srv_events_tx`. A dedicated
subscriber — `agentmux-srv/src/server/wave_obj_bridge.rs`, purpose-built
for exactly this class of bug (see `docs/specs/SPEC_OBJ_UPDATE_BRIDGE_2026-05-14.md`,
triggered by an earlier **workspace**-rename reactivity bug) — picks up
the event and broadcasts a full-payload `waveobj:update` to every
connected client, correctly, for workspace/window/tab/block alike.

On the frontend, both paths funnel into the exact same consumer —
`wpsSubscribeToObject`'s handler (`frontend/app/store/wos.ts:85-93`) calls
`updateWaveObject(event.data)`, and `updateWaveObject` (`wos.ts:260-261`)
is a silent no-op when `data == null`:

```ts
function updateWaveObject(update: WaveObjUpdate) {
    if (update == null) return;   // <-- legacy path's non-block broadcasts die here
    ...
}
```

So the legacy `setmeta` path works *perfectly* for `block` objects (the
one otype it special-cases) and *silently does nothing observable* for
every other otype — window, workspace, tab — with:
- no error returned to the caller,
- no console warning,
- no type-level signal that `SetMetaCommand`'s generic `{oref, meta}`
  signature is actually block-only in practice.

## Why this wasn't already a shipped bug

Checked every current production call site of `RpcApi.SetMetaCommand`
(29 files) — **100% of them target a `block:` oref.** Zoom, pane title,
connection state, term settings — all per-block. Nobody had reached for
this command against a window/workspace/tab oref before, so the gap has
sat latent since whenever `setmeta` was written, un-exercised.

It surfaced the moment an unfamiliar caller (me, despite deep session
familiarity with this codebase) reached for the obviously-generic-looking
command name to do a generic-sounding thing ("set this object's meta").
Nothing about the command's name, its TypeScript signature
(`{oref: string, meta: MetaMapType}` — `oref` is just a string, no
otype narrowing), or its runtime behavior (no error!) signals that only
one specific otype actually works.

## This is not a one-off — a second, independent instance of the same pattern

Earlier this session (PR #2329, Stash MCP/Skills tabs), a materially
different but structurally identical bug was found and fixed: the
`check_s1`-gated `mcp.bind`/`mcp.unbind`/`skill.bind`/`skill.unbind`
handlers mutated the DB but never called `broker.publish(...)` for
`mcp:changed`/`skills:changed` — so an agent binding/unbinding itself via
its own tool calls left any already-open Stash tab stale until manually
reopened. Different pub/sub system (WPS custom events, not
`waveobj:update`/WaveObject bridge), same root shape: **a write succeeds;
whether anyone reactively observes it depends on whether the specific
handler remembered a manual publish call, with nothing structural
enforcing that it did.**

Two independently-discovered instances of the identical failure mode in
one session is a strong signal this is systemic, not coincidental — the
`wave_obj_bridge.rs` module's own doc comment already anticipated exactly
this ("per-RPC handlers were responsible for attaching updates to their
responses. Forgetting that call left the frontend WOS cache stale"), and
built a *bridge* for the WaveObject side specifically because the
per-handler convention proved unreliable. The bridge doesn't cover the
WPS custom-event side (`mcp:changed` etc.), and the pre-bridge legacy
`setmeta` command was never migrated onto the bridge either.

## Recommended fix (not yet implemented — design only)

**Immediate / tactical:** anything renaming a window/workspace/tab should
go through `object.UpdateObjectMeta` (HTTP service call) or the
documented Agent App API (`SetName` / `/api/v1/window/name`), never the
raw `setmeta` WS RPC. Block metadata (zoom, pane title, etc.) should keep
using `setmeta` — it's correct and lower-overhead for that one otype.

**Structural, in rough priority order:**

1. **Close the trap at the type/API level.** Either (a) rename
   `SetMetaCommand`/`setmeta` to make the block-only scope explicit
   (e.g. `SetBlockMetaCommand`) and narrow its TypeScript signature to
   `oref: \`block:${string}\`` so a non-block call is a compile error, or
   (b) extend `setmeta`'s handler to dispatch through the reducer +
   bridge for every otype, matching `object.UpdateObjectMeta`'s
   reactivity — collapsing two mechanisms into one correct one. (b) is
   more invasive (touches a widely-used command with 29 call sites) but
   removes the footgun entirely instead of relabeling it; (a) is a
   same-day fix.
2. **Fail loud, not silent, on the remaining gap.** Whichever of the
   above lands, the backend `setmeta` handler should reject (not
   silently accept-and-drop) a write to an otype it can't correctly
   broadcast for, until/unless it's extended to handle it. A caller
   getting `FORBIDDEN: setmeta only supports block objects, use
   object.UpdateObjectMeta for window/workspace/tab` immediately reveals
   the problem instead of a title that just never updates.
3. **Extend the WaveObjUpdate bridge's coverage net to WPS custom
   events too**, closing the `mcp:changed`/`skills:changed`-shaped gap
   generally instead of one broker.publish call at a time as each is
   discovered. Concretely: a lint/test that cross-references every
   reducer `Event` variant (or every DB-mutating RPC handler) against
   whether *something* downstream re-broadcasts it — so a new
   mutating command that forgets to publish fails CI instead of shipping
   silently non-reactive, the same way `wave_obj_bridge.rs`'s own
   `dispatch_event` match's `_ => {}` catch-all currently swallows any
   event variant nobody's gotten around to wiring up yet, with no signal
   that it's incomplete.
4. **Document the two-mechanism split explicitly** in
   `docs/internals/agent-app-api.md` (or a new
   `docs/architecture/OBJECT_REACTIVITY.md`) so the next person doesn't
   have to trace three files under CDP live-testing to discover it, as
   this retro's own investigation did.

## Non-goals for this report

This is investigation + design, not an implementation PR. No code has
been changed. Item 1(b) in particular (migrating `setmeta` onto the
reducer/bridge) is a real architecture change touching 29 call sites and
the core object-mutation path — it needs its own scoped PR with its own
verification pass, not a bundled fix here.

## Files referenced

- `agentmux-srv/src/server/websocket.rs` (legacy `setmeta`/`getmeta`,
  lines ~654-736)
- `agentmux-srv/src/server/service/object.rs` (`UpdateObjectMeta`
  reducer-routed path, lines ~291-398)
- `agentmux-srv/src/server/wave_obj_bridge.rs` (the fix for the earlier,
  analogous workspace-rename bug)
- `docs/specs/SPEC_OBJ_UPDATE_BRIDGE_2026-05-14.md` (prior art — the
  exact same bug class, previously diagnosed for workspaces)
- `frontend/app/store/wos.ts` (`updateWaveObject`'s silent-no-op-on-null
  behavior, `wpsSubscribeToObject`)
- `frontend/app/store/rpc-api/workspace.ts` (`SetMetaCommand`'s generic,
  un-narrowed signature)
- `agentmux-docs/src/content/docs/internals/agent-app-api.md`
  (`SetName` — the documented, correct path)
- `agentmux-srv/src/server/app_api/mcp.rs` / `skill.rs` (PR #2329's
  independent instance of the same pattern — `bind`/`unbind` missing
  `broker.publish`)
