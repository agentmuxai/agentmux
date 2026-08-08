# Per-agent zoom persistence

**Date:** 2026-06-22
**Status:** Proposed (design) (superseded — see note below)
**Owner:** AgentC

> **2026-08-07 audit note:** Superseded by the generalized universal-zoom
> framework (`term:zoom` block-meta key), not the agent-specific design
> proposed here. See
> `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.
**Area:** agent pane / view zoom, per-agent content storage

---

## 1. Problem

Zooming an agent pane (Ctrl + mouse wheel) is lost the moment the pane is
closed. Reopening the *same* agent comes back at the default 1.0 zoom. Users who
prefer a particular agent at a larger or smaller size have to re-zoom every time
they reopen it.

The zoom is a genuine **per-agent reading preference**, but today it is bound to
a throwaway object (the block), so it cannot outlive a single pane.

## 2. Current behavior (root cause)

Agent zoom lives in the **block's `meta`** under the key `term:zoom`:

- **Read / apply:** `frontend/app/view/agent/agent-view.tsx:917` reads
  `meta?.["term:zoom"]` into a memo and applies it as the CSS `zoom` on
  `.agent-view` (`:1000`). The terminal view reads the same key
  (`frontend/app/view/term/term.tsx:75`).
- **Write:** `frontend/app/view/term/term.tsx:222-226` handles Ctrl+Wheel and
  calls `SetMetaCommand` on `block:<id>` with `{ "term:zoom": next }` (or `null`
  at 1.0, by convention, so a default pane carries no key). This routes through
  the block-meta update path (`UpdateBlockMeta` reducer,
  `agentmux-srv/src/reducer/block.rs`) and persists onto the **block** row.

The block is ephemeral:

- **Close pane** → `DeleteBlock` removes the block (and its `meta`, including
  `term:zoom`).
- **Reopen agent** → `agent.open` (`agentmux-srv/src/server/app_api.rs`, the
  `register_agent_open` handler) builds a **brand-new block** and seeds its meta
  with `agentId`, `agentProvider`, `controller`, `cmd*`, etc. — **but not**
  `term:zoom`. So the new block defaults to 1.0.

Nothing connects the zoom to the agent's stable identity (`agent.id`), so it
cannot survive the block.

## 3. Goal & requirements

1. **Persist across close/reopen.** Reopening the same agent (by `agent.id`)
   restores the last zoom the user set for it.
2. **Keyed by agent identity, not block.** Zoom follows the agent, including a
   cold reopen in a different tab/window/session.
3. **Global, like the agent.** Agents are global cross-channel
   (`CLAUDE.md`, agent/auth globalization). Zoom should follow the agent across
   channels and instances, not be siloed per data dir.
4. **No write amplification.** Ctrl+Wheel emits a burst of `term:zoom` updates;
   persistence must coalesce so the durable store is written once per zoom
   gesture, not once per tick.
5. **Default stays clean.** An agent the user never zoomed (or reset to 1.0)
   stores nothing — no row, no key, no migration.
6. **No regression to the live path.** The block-meta `term:zoom` remains the
   single live source the views render from; this feature only seeds it on open
   and mirrors it on change.

Non-goals (this spec): terminal-pane zoom persistence (terminals have no stable
cross-open identity to key on — see §8), and live cross-pane sync of zoom while
two panes of the same agent are open simultaneously (see §8).

## 4. Design

### 4.1 Storage — reuse the per-agent content store

Persist zoom in the existing per-agent KV store
(`agentmux-srv/src/backend/storage/content.rs`,
`Store::agent_content_get` / `agent_content_set`), the same mechanism that
backs `env` and `instructions`:

- **`agent_id`** = the agent definition id.
- **`content_type`** = `"ui:zoom"`.
- **`content`** = the zoom factor as a short decimal string (e.g. `"1.3"`).
  Absent row = default 1.0.

Why this store:

- Already **global cross-channel** — `agent_content_get` falls back to the
  shared def registry for non-local agents (`content.rs:60-84`), and
  `agent_content_set` re-mirrors via `registry_def_upsert` (`:106`). Zoom
  inherits the agent's global reach for free.
- Keyed by `agent.id`, exactly the identity we need.
- Additive: new `content_type`, no schema change, no migration.

> Alternative considered: `AgentDef.meta["ui:zoom"]`. Rejected as the primary
> home because mutating the definition for a transient UI pref is heavier and
> conflates "what the agent is" with "how I like to view it." The content store
> is the established per-agent *side-data* channel. (See §7.)

### 4.2 Restore — seed `term:zoom` at `agent.open`

In `register_agent_open` (`app_api.rs`), while building the new block's `meta`
(right where `agentId` etc. are inserted), read the saved zoom and seed it:

```text
if let Ok(Some(c)) = wstore.agent_content_get(&agent.id, "ui:zoom") {
    if let Ok(z) = c.content.trim().parse::<f64>() {
        if (z - 1.0).abs() > f64::EPSILON && (0.5..=2.0).contains(&z) {
            meta.insert("term:zoom".to_string(), json!(z));
        }
    }
}
```

The new block is therefore born at the saved zoom; the views render it through
the unchanged `term:zoom` read path. Clamp to the same `[0.5, 2.0]` range the
frontend enforces (`term.tsx:223`) so a corrupt value can't escape it. A `1.0`
value (or unparviceable content) seeds nothing → default.

### 4.3 Persist — mirror `term:zoom` → `ui:zoom` on change (debounced)

The frontend keeps writing `term:zoom` to block meta exactly as today (no
frontend change required for the core). The **backend** mirrors it: when a block
that carries an `agentId` has its `term:zoom` updated, write the value through
to the per-agent store.

- **Hook point:** the block-meta update RPC handler in
  `agentmux-srv/src/server/service.rs` (the `SetMeta` / `UpdateBlockMeta`
  service path), NOT the pure reducer — keep agent-zoom knowledge out of
  `reducer/block.rs`. After applying the meta update, if the updated keys
  include `term:zoom` and the block's resolved meta has a non-empty `agentId`,
  enqueue a debounced mirror.
- **Coalescing:** maintain a per-`agent_id` debounce (≈300 ms trailing). Ctrl
  +Wheel produces ~10 updates/sec; the trailing write persists only the final
  resting zoom, so the global registry re-mirror fires once per gesture.
- **Reset semantics:** the frontend already sends `term:zoom: null` at exactly
  1.0. On a `null`/absent `term:zoom`, the mirror **deletes** the `ui:zoom` row
  (`agent_content_delete`, `content.rs:174`) so a reset-to-default agent goes
  back to storing nothing (requirement 5).
- **`updated_at`:** stamp with the server clock at write time (consistent with
  other `AgentContent` writes).

> Why mirror server-side rather than add a second frontend RPC: the frontend
> already emits the authoritative `term:zoom` change through one path. Mirroring
> there keeps a single source of truth, avoids a race between two client writes,
> and means every producer of `term:zoom` (agent view, and any future surface)
> is covered without touching each call site.

### 4.4 Data flow (end to end)

```
Ctrl+Wheel  ──SetMeta(block, term:zoom)──►  block.meta.term:zoom   (live render, unchanged)
                                                   │
                                      service.rs mirror hook (debounced, agentId present)
                                                   ▼
                                   agent_content_set(agent_id, "ui:zoom", "1.3")
                                                   │  (global re-mirror)
                                                   ▼
close pane → DeleteBlock                    db_agent_content / shared def registry
reopen agent → agent.open ──agent_content_get(agent_id,"ui:zoom")──► seed new block.meta.term:zoom
```

## 5. Implementation plan

1. **Backend — restore (smallest, highest value):** in `register_agent_open`
   (`app_api.rs`), seed `term:zoom` from `agent_content_get(agent.id,"ui:zoom")`
   as in §4.2. This alone fixes the reported bug for any agent whose zoom is
   already stored.
2. **Backend — persist:** add the debounced mirror in the `SetMeta` block path
   (`service.rs`), §4.3, including the delete-on-reset branch. A small
   per-`agent_id` trailing-debounce map on `AppState` (or a lightweight
   `tokio` task keyed by agent_id).
3. **Tests:**
   - `agent_content` round-trip for `ui:zoom` (set → get → parse).
   - `agent.open` seeds `term:zoom` when a `ui:zoom` row exists; seeds nothing at
     1.0 / missing; clamps out-of-range.
   - Mirror: a `term:zoom` SetMeta on an agent block writes `ui:zoom`; a `null`
     `term:zoom` deletes it; a SetMeta on a non-agent block (no `agentId`) does
     nothing.
   - Debounce: N rapid updates collapse to one `agent_content_set`.
4. **Frontend:** none required for the core. (Optional polish in §8.)

## 6. Test / acceptance

- Zoom an agent pane to e.g. 1.4, close the pane, reopen the same agent → it
  returns at 1.4.
- Reopen the same agent in a **different window** → 1.4.
- Reset to 1.0 (zoom back), close, reopen → default 1.0, and the `ui:zoom` row
  is gone.
- An agent never zoomed → default 1.0, no row written.
- Rapid Ctrl+Wheel from 1.0 → 1.8 writes the durable store **once** (assert via
  store write count / `updated_at` stability).

## 7. Alternatives considered

| Option | Verdict |
|---|---|
| Keep zoom on the block only (status quo) | Rejected — the reported bug. |
| `AgentDef.meta["ui:zoom"]` | Viable, but conflates definition with view pref and is a heavier write; content store is the established per-agent side-data path. |
| Frontend-only (persisted atom / localStorage keyed by agentId) | Rejected — not global cross-channel, renderer-scoped, and fragile across windows. |
| New dedicated `db_agent_ui_prefs` table | Rejected for now — over-engineered for one scalar; revisit if more per-agent UI prefs accumulate (then migrate `ui:*` content_types into it). |

## 8. Open questions / future work

- **Live cross-pane sync.** If the same agent is open in two panes at once, this
  design seeds each on open and last-writer-wins to the store; it does **not**
  live-sync an in-flight zoom across the two open panes. If desired, broadcast
  the `ui:zoom` change so sibling agent panes update their `term:zoom`. Likely a
  small follow-up, not needed for the reported bug.
- **Terminal panes.** Terminals share the `term:zoom` key but have no stable
  cross-open identity to persist against. Out of scope here; a separate spec
  could persist terminal zoom per-tab or as a global default.
- **Generalize `ui:*` prefs.** If we later persist more per-agent view prefs
  (font, wrap, theme), promote `ui:zoom` into a structured `ui:prefs` JSON
  content blob rather than one content_type per pref.
- **Per-pane override.** Should a user be able to zoom one pane *without*
  changing the agent's saved default (a transient override)? Current design
  treats every zoom as the new saved default; flag if product wants a "this pane
  only" mode.
