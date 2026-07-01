# SPEC — Rename Pool-Promoted Floating Panes to `floating-<uuid>` (Option A)

**Date:** 2026-06-30
**Type:** Implementation spec (fully scoped)
**Status:** Ready to schedule
**Owner:** asaf
**Bug:** Status-bar InstancePanel reports **0 floating panes** while a user floating pane is
visible on Windows.
**Severity:** Cosmetic (miscount); the pane itself works. Not a regression — pre-existing, in the
pane-pool + status-bar code (untouched by the srv layout program).

> Goal: when a warm pane-pool window (`floating-pool-<uuid>`) is **promoted** into a user-visible
> floating pane, rename it to a real `floating-<uuid>` label so it is counted like any cold-path
> floater — **without stranding** any store, HWND cache, or renderer that keys off the old label.

---

## 1. Root cause (confirmed)

1. On Windows, opening a floating pane takes the **pane-pool fast path**
   (`commands/floating_pane.rs:219`, non-Windows `:272`) → `promote_pane_pool_window`
   (`commands/window_pool.rs:1517`), which **reuses the pool label** `floating-pool-<uuid>` and
   returns it verbatim (`window_pool.rs:1600` emit, `:1613`ish return). The cold-path label
   `floating-{uuid}` allocated at `floating_pane.rs:130` is **discarded** when the pool path hits.
2. `blockframe.tsx:246` renders floating chrome for any `label.startsWith("floating-")` — and
   `"floating-pool-…".startsWith("floating-")` is **true** → the pane *looks* floating. ✓
3. The status-bar counter uses `isFloatingPaneLabel` (`launcher-event/types.ts:157`), which does
   `if (label.startsWith("floating-pool-")) return false` → the promoted pane is **excluded** →
   panel shows **0**.

### 1.1 How the count is actually fed (this determines whether a rename is sufficient — it is)
The frontend's floating count comes from the **`list_window_instances` host snapshot**
(`commands/window/meta.rs:102`), consumed by `seedKnownEntriesFromSnapshot` (boot) and the
InstancePanel **reconcile** (`launcher-event-reducer.ts:187,229`), filtered through
`isFloatingPaneLabel`. It is **not** fed by the launcher-event stream: **the host reports nothing
about floating panes to the launcher** (no `report_window_opened`/`_instance_assigned`/
`_backend_window_id` for floaters — verified). Cold-path `floating-<uuid>` floaters are counted via
this same snapshot path (they pass `isFloatingPaneLabel`).

`list_window_instances` returns a **promoted** pool pane (it's removed from the pool set at promote,
so it survives the `!pool_labels.contains(l)` filter, `meta.rs:111`) — but under its
`floating-pool-<uuid>` label, which the counter rejects. **Therefore renaming the promoted pane to
`floating-<uuid>` is sufficient for the count** (it then appears in the snapshot under a countable
label, at parity with cold-path floaters). No launcher-event wiring is required.

> ⚠️ Timing note (pre-existing, out of scope): because floaters ride the snapshot/reconcile path,
> not live launcher events, the floating count updates on reconcile, not instantly. The rename brings
> pool floaters to the **same** timing as cold-path floaters — it does not make it worse, and fixing
> the snapshot-lag is a separate concern.

---

## 2. Decision

**Option A — rename on promotion.** Chosen over (B) making the counter promotion-aware or (C) a
promotion marker, because the "pool" prefix is *semantically wrong* on a pane that is now a real user
floater, and Option A makes every existing `startsWith("floating-")` / `isFloatingPaneLabel` /
snapshot consumer correct with no new promotion-state plumbing. Its cost is entirely in re-keying —
which this spec enumerates exhaustively.

**New label:** reuse the pool pane's uuid, dropping the `pool-` segment:
`floating-pool-<uuid>` → `floating-<uuid>`. (Same scheme as the cold path's `floating-{uuid}`;
reusing the uuid keeps host/renderer logs correlatable across the rename.)

---

## 3. The window-pool precedent (why we can't just copy it)

Regular windows are counted correctly despite *also* coming from a pool (`window-pool-<uuid>`). But
the window path does **not** rename: `promote_pool_window` (`window_pool.rs:654`) keeps the
`window-pool-*` label, and `isInstanceLabel` (`types.ts:144`) **permissively** counts anything
`startsWith("window-")` (including `window-pool-*`); unpromoted pool windows are gated out
**host-side** before being reported. The floating filter is the **opposite** — it *excludes* the
pool prefix. Mirroring the window path would mean making `isFloatingPaneLabel` permissive **and**
wiring floaters into the launcher-report path (which they bypass entirely today) — strictly more work
than the rename. So the precedent informs but does not transfer; **Option A (rename) is the smaller,
cleaner fit for floaters precisely because floaters bypass the launcher.**

---

## 4. Host changes (agentmux-cef)

### 4.1 Re-key surface — MUST update on rename (these persist for the window's life)
| Store | Location | Action |
|---|---|---|
| `HostState.browsers` (label → `BrowserHandle`) | `reducer/mod.rs:96`, `state.rs:219` | Remove old key, reinsert under new label. **Also update the duplicated `BrowserHandle.label` field.** `register_browser` rejects dup keys, so this is remove+reinsert, not insert. |
| `AppState.window_hwnds` (label → outer HWND, Windows) | `state.rs:859` | Re-key. Feeds `label_for_hwnd` reverse-scan (`state.rs:1091`) and all floater geometry/opacity ops. A partial rename desyncs forward/reverse lookups. |
| `AppState.window_meta` (label → `WindowMeta`) | `state.rs:641` | Re-key. **Also update the duplicated `WindowMeta.label` field.** Orphan cleanup removes by label (`window_pool.rs:1500`). |
| `ACTIVE_FLOATER_HWNDS` static | `floating_pane.rs:78` | Register with the **new** label at `window_pool.rs:1593` (removal is by-HWND, so rename-agnostic — just register the new key). |

### 4.2 Re-key only if present (usually absent at promote — created lazily later)
- `HostState.pane_window_states` (`reducer/mod.rs:142`) — maximize/restore state; created on first
  maximize toggle. Guard: re-key iff the old label has an entry.
- `HostState.window_opacities` (`reducer/mod.rs:130`) — created iff opacity was set. Same guard.

### 4.3 No action (ephemeral — cleared/consumed during the promote itself)
`pane_pool.{queue, unpromoted}` (popped/removed by `PopAndPromoteFrontPanePoolWindow` *before* the
rename), `PANE_POOL_HWND_CACHE` (`take_pane_pool_hwnd` removes it), `pending_window_creations`
(dequeued at `on_after_created`). Confirmed not floater-keyed: `browser_panes`,
`pending_browser_pane_creates`, `shadow_*`, tab-pool caches.

### 4.4 New reducer command: `HostCommand::RelabelBrowser { old_label, new_label }`
The rename must be a **pure reducer mutation** (single source of truth, testable) that:
- moves the `browsers` entry old→new (error if old absent or new already present),
- updates `BrowserHandle.label`,
- re-keys `window_meta` (+ `WindowMeta.label`), and conditionally `pane_window_states` /
  `window_opacities`.
`window_hwnds` / `ACTIVE_FLOATER_HWNDS` are `AppState`/static (not `HostState`) — re-key those in
`promote_pane_pool_window` directly, adjacent to the dispatch.

### 4.5 Sequencing inside `promote_pane_pool_window` (Windows block `1527-1613`)
1. `PopAndPromoteFrontPanePoolWindow` dispatch (old label leaves the pools; `is_pool=false`).
2. `take_pane_pool_hwnd(&old_label)`.
3. HWND liveness (`IsWindow`) + `SetWindowPos` / `ShowWindow`.
4. `let new_label = format!("floating-{}", uuid_of(&old_label))`.
5. **Re-key:** dispatch `RelabelBrowser { old_label, new_label }`; re-key `window_hwnds`.
6. `register_floater_hwnd(new_label.clone(), outer_hwnd, parent_hwnd)`.
7. `emit_event_to_window(state, &new_label, "pool:pane-promote", { paneId, workspaceId, windowLabel: new_label })`
   — **must** emit under the new key: `emit_event_to_window` resolves the target via
   `get_browser(label)` (`events.rs:91`), and after step 5 the browser lives under `new_label`.
8. Return `new_label` so `open_floating_pane_window` reports it as `window_label` to the caller.
9. `spawn_pane_pool_window` refill.

Apply the equivalent to the **non-Windows** promote block (`window_pool.rs:105`-region /
`floating_pane.rs:272`); it has no `window_hwnds`/HWND-cache steps but the `browsers`/`window_meta`
re-key + payload + return-label changes are identical.

---

## 5. Frontend changes

### 5.1 Carry the new label in the promote event
`pool:pane-promote` payload gains `windowLabel: string` (host §4.5 step 7).

### 5.2 Rewrite the renderer's URL label — the critical loose end
`awaitPanePoolPromote` (`init/pool.ts:116-131`) currently rewrites `floatingPaneId` / `workspaceId`
and deletes `pane-pool`, but **not** `windowLabel`. Add:
```ts
url.searchParams.set("windowLabel", payload.windowLabel);
```
Reason: multiple readers derive the renderer's own label from `?windowLabel=` (e.g.
`browser-view.tsx:61,114,376`, `floating-pane-workspace.tsx`, `blockframe.tsx:245`, drag hooks). After
a host re-key, every label-addressed IPC from this renderer (`browser_pane_create`,
`set_window_rect`/`get_window_rect`, `main_window_focus`, `closeWindowByLabel`,
`registerBackendWindow`) would target the dead `floating-pool-*` label and fail unless the URL param
is updated. Update the event's TS type to include `windowLabel`.

### 5.3 Label-source consistency (verification point, not necessarily a code change)
Two sources of the renderer's label must agree post-rename:
- **`getApi().getWindowLabel()`** — a host IPC call (`InstancePanel.tsx:81`), i.e. host-resolved
  live. **Verify** the host `get_window_label` command resolves via a browser→label reverse lookup so
  it returns the **new** label after re-key (expected, but confirm — if it caches, fix it).
- **URL `?windowLabel=`** — fixed by §5.2.
Audit that no reader caches the label at load in a way that survives the rename; if any does, it must
re-read after `pool:pane-promote`.

### 5.4 Counter — no change needed
`isFloatingPaneLabel` already accepts `floating-<uuid>` and rejects `floating-pool-*`. Once the
snapshot returns the new label, seed/reconcile counts it. **Do not** loosen the pool exclusion (warm
unpromoted pool panes must still read 0).

---

## 6. Explicitly NOT affected (do not over-engineer)
- **Launcher (agentmux-launcher):** tracks **zero** floating panes today — `windows`, `pool`,
  `instance_registry`, `backend_window_ids`, `just_promoted_labels` never contain `floating-*`. A
  rename touches no launcher state, and **no host→launcher rename message is needed** (none exists;
  `Event::WorkspaceRenamed`/`TabRenamed` carry workspace/tab ids, not window labels). Do **not** add
  launcher reporting for floaters — the snapshot path already counts them.
- **srv (agentmux-srv):** never stores window labels (`WindowRecord`/`Window` WaveObject key on
  `window_id`/`oid`; label-bearing IPC is discarded at `extract_version`). Zero srv impact.

---

## 7. Loose-ends checklist (label-immutability assumptions to satisfy)
- [ ] `browsers` re-key is remove+reinsert (dup-key reject) and updates `BrowserHandle.label`.
- [ ] `emit_event_to_window` for `pool:pane-promote` fires **after** the re-key, under the new label
      (else `get_browser` misses).
- [ ] `window_hwnds` forward + `label_for_hwnd` reverse stay consistent (re-key atomically).
- [ ] `window_meta.label` field updated, not just the map key.
- [ ] `pane_window_states` / `window_opacities` re-keyed **iff** an entry exists.
- [ ] `ACTIVE_FLOATER_HWNDS` registered with the new label.
- [ ] Renderer URL `windowLabel` rewritten (§5.2); `get_window_label` returns the new label (§5.3).
- [ ] Non-Windows promote block updated to match (browsers/meta/payload/return-label).
- [ ] Return value of `open_floating_pane_window` (`OpenFloatingPaneResponse.window_label`) is the new
      label, so any caller that stored it is correct.

---

## 8. Test plan
- **Unit (host reducer):** `RelabelBrowser` — moves the entry, updates the `.label` field, errors on
  missing-old / existing-new, re-keys `window_meta` and conditionally `pane_window_states` /
  `window_opacities`; leaves unrelated labels untouched.
- **Unit (frontend):** `awaitPanePoolPromote` rewrites `windowLabel` from the payload; the
  `pool:pane-promote` type includes it.
- **Manual (Windows, the reported repro):** open a floating pane (hits the pool fast path) → status-bar
  InstancePanel counts **1** floating pane (after reconcile); open a second → **2**; close → decrements;
  a warm, un-opened pool pane still counts **0**.
- **Manual regression:** after promotion, exercise the floater's label-addressed paths — move/resize,
  maximize toggle, focus, redock, close — confirm none break (they'd break if any store or the URL
  label were left stale). This is the real risk surface.

---

## 9. Risks / caveats
- **Partial re-key = dangling reference.** The whole risk is missing one keyed-by-label store; §4 +
  §7 enumerate them. The `emit`-after-re-key ordering (§4.5 step 7) is the subtlest.
- **Renderer label desync** (§5.2/5.3) is the highest-impact loose end: get it wrong and the floater's
  IPC silently targets a dead label. Cover it in the manual regression.
- **This needs the app running to fully verify** — the unit tests prove the re-key mechanics, but the
  renderer-label consistency and the count are runtime properties. Treat like other runtime-gated
  changes: verify in a live instance before merge, don't rely on bots alone.
- Scope discipline: resist the temptation to "also fix" the snapshot/reconcile timing lag or to wire
  floaters into launcher reporting — both are out of scope; the rename alone fixes the reported bug.

---

## 10. Sources
- Host: `commands/floating_pane.rs:100,130,219,272`, `commands/window_pool.rs:1517,1527-1613,1593,1600`,
  `commands/window/meta.rs:102-127`, `state.rs:219,641,859,1091`, `reducer/mod.rs:96,130,142`,
  `commands/events.rs:91`, `reducer/pane_pool.rs:63-77`.
- Frontend: `store/launcher-event/types.ts:144-158`, `store/launcher-event-reducer.ts:69-115,187-229,297`,
  `store/init/pool.ts:107-143`, `statusbar/InstancePanel.tsx:81,576`, `block/blockframe.tsx:245`,
  `view/browser/browser-view.tsx:61,114,376`.
- Precedent: window-pool `promote_pool_window` (`window_pool.rs:654,829-859`), `isInstanceLabel`
  (`types.ts:144`).
- Investigation: this session's full label-lifecycle map (host/launcher/srv/frontend).
