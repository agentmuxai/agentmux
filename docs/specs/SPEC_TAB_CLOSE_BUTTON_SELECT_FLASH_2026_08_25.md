# Tab close (X) button — spurious select flash

**Status:** §§2-3 (click-bubble race), §5 (double round trip), §6
(unbatched RPC-response application), and §7 (unbatched WS-push
application — the reason the flash survived §§5-6) are all implemented on
branch `fix/tab-close-select-flash` (PR #2811, open — not yet merged to
`main`, so no release build contains any of this yet).
**Owner:** unassigned
**Date:** 2026-08-25
**Scope:** `frontend/app/tab/tab.tsx`, `frontend/app/tab/tabbar.tsx`,
`frontend/app/tab/droppable-tab.tsx`, `agentmux-srv/src/reducer/tab.rs`
(`handle_set_active_tab`, `handle_delete_tab`)
**Related:** `SPEC_TAB_BAR_FIRST_PRINCIPLES_2026_04_25.md` (tab-bar layout
invariants), `SPEC_TAB_UI_REFINEMENTS_2026_06_20.md` (close-confirm modal /
`tab:skipcloseconfirm`), `SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md`
(a different flicker class — remount-driven, not this one)

---

## 1. Symptom

Clicking the "✕" close button on a **background (non-active) tab** in the
top window tab strip produces a brief, visually jarring flash: another tab
(often, but not reliably, the tab that was just clicked) appears to become
selected/highlighted for an instant, then immediately deselects/reverts.
The tab being closed does disappear correctly in the end, but the
in-between flash reads as broken/flaky UX. It is intermittent — it does not
reproduce on every close, which is itself a clue (see §2.3).

Closing the tab via keyboard (Ctrl+W) does not reproduce this — only mouse
clicks on the per-tab "✕" button are affected.

## 2. Root cause

### 2.1 The click event is not stopped from bubbling past the close button

`frontend/app/tab/tab.tsx:251-267` renders each tab as:

```tsx
<div class={clsx("tab", { active: props.active, ... })}
     onClick={props.onSelect}          // line 263 — selects this tab
     ...>
  <div class="tab-inner">
    ...
    <Button
        onClick={props.onClose}        // line 289 — closes this tab
        onMouseDown={handleMouseDownOnClose}   // line 290 — stops mousedown only
        ...
    />
  </div>
</div>
```

`handleMouseDownOnClose` (`tab.tsx:228-230`) calls
`event.stopPropagation()`, but only on the `mousedown` event. The `Button`'s
`onClick` (which fires `props.onClose`) does **not** stop propagation. A
native `click` on the close button therefore:

1. Fires the button's own `onClick` → `props.onClose` → (via
   `DroppableTab`, `tabbar.tsx:169`) `requestClose(tabId)` — starts the
   close flow for the clicked tab.
2. Continues bubbling up the DOM (nothing stopped it) to the outer `.tab`
   div's `onClick` → `props.onSelect` → (`tabbar.tsx:168`)
   `handleSelect(tabId)` — **selects the very same tab that step 1 just
   started closing.**

`handleSelect` (`tabbar.tsx:43-46`) only guards against re-selecting a tab
that is *already* active:

```ts
const handleSelect = (tabId: string) => {
    if (tabId === activeTabId()) return;
    setActiveTab(tabId);
};
```

For a background tab (the common case — you're closing a tab you're not
currently looking at), `tabId !== activeTabId()`, so this guard does
nothing and `setActiveTab(tabId)` fires for real, issuing a
`WorkspaceService.SetActiveTab` call for the tab that is simultaneously
being torn down.

(Closing the *currently active* tab happens not to trigger the bubble's
side effect, because `handleClose` — see §2.2 — synchronously kicks off its
own `setActiveTab(nextTab)` first, so by the time the bubbled `onSelect`
fires, the guard's `tabId === activeTabId()` is still true against the old
active id and short-circuits. The bug is specific to closing a **background**
tab.)

### 2.2 Two independent, unordered HTTP requests race on the backend

Both calls go through `WOS.callBackendService` (`frontend/app/store/wos.ts:95-143`),
which issues a plain `fetch()` POST to `/agentmux/service` per call
(`wos.ts:122-127`) — **there is no shared ordering guarantee between two
calls fired back-to-back on the client.** The close flow
(`tabbar.tsx:48-60`) ends up firing:

- `WorkspaceService.CloseTab(wsId, tabId)` — from step 1 above.
- `WorkspaceService.SetActiveTab(wsId, tabId)` — from step 2's bubbled
  `onSelect` (via `setActiveTab` in `frontend/app/store/tab-actions.ts:66-112`).

Whichever HTTP request the backend happens to service first decides what
the user sees, because the reducer (`agentmux-srv/src/reducer/tab.rs`)
processes commands strictly in receipt order:

- **`handle_set_active_tab`** (`tab.rs:177-213`) — if the tab is still in
  `workspace.tab_ids`, sets `active_tab_id` to it and emits
  `Event::ActiveTabChanged`, which the frontend applies immediately (the
  tab bar re-renders with that tab highlighted).
- **`handle_delete_tab`** (`tab.rs:87-172`) — if the deleted tab **is**
  the current `active_tab_id`, it reassigns active to
  `tab_ids.get(pos)` (the tab that slides into the deleted one's old
  index) or the previous one, and emits a second
  `Event::ActiveTabChanged`.

Two orderings are possible:

- **`CloseTab` wins the race:** `DeleteTab` runs while `active_tab_id` is
  still the real original active tab, so no `ActiveTabChanged` fires from
  the delete. The stray `SetActiveTab` then arrives for a tab id that's
  already gone (`handle_set_active_tab`'s membership check at `tab.rs:191`
  fails) and is rejected as a harmless `Event::Error`. **No visible flash**
  — this is the case where the bug doesn't reproduce.
- **`SetActiveTab` wins the race:** `active_tab_id` flips to the
  closing tab and the frontend highlights it (**flash #1 — "another tab
  selected"**). `DeleteTab` then runs, sees the deleted tab is (now)
  active, and reassigns to whatever neighbor `tab_ids.get(pos)` resolves
  to, firing a second `ActiveTabChanged` (**flash #2 — the highlight jumps
  again, reading as "deselected"**).

### 2.3 Why it's intermittent

Because both requests are independent `fetch()` calls with no ordering
contract, which one the backend services first depends on real-world
network/event-loop scheduling (client-side fetch dispatch timing, HTTP
connection reuse, backend thread/task scheduling) — not application logic.
That non-determinism is exactly why the flash reproduces "sometimes" rather
than every time, which matches the reported symptom.

### 2.4 A second, non-cosmetic bug riding on the same cause

In the "`SetActiveTab` wins" ordering, `handle_delete_tab`'s neighbor-promotion
picks `tab_ids.get(pos)` / `pos - 1` relative to the **closed tab's own
position**, not relative to whatever tab the user actually had open before
they clicked. If the user was on tab A and closed background tab X, the
race can leave tab **Y** (X's neighbor) active instead of A — i.e. beyond
the visual flash, the user's real active tab can silently change as a side
effect of closing an unrelated tab.

## 3. Fix

Stop the close button's `click` from ever reaching the tab's own `onClick`,
the same way `mousedown` is already stopped. This removes the spurious
`onSelect` call at the source, so no race between `SetActiveTab` and
`CloseTab` is ever created — no backend-ordering fix is needed.

`frontend/app/tab/tab.tsx`:

```tsx
const handleMouseDownOnClose = (event: MouseEvent) => {
    event.stopPropagation();
};

const handleCloseClick = (event: MouseEvent) => {
    event.stopPropagation();
    props.onClose(event);
};
```

and wire the button to it:

```tsx
<Button
    className="ghost grey close"
    onClick={handleCloseClick}
    onMouseDown={handleMouseDownOnClose}
    ...
```

(`props.onClose` itself is left untouched — `DroppableTab` and `TabBar`
already pass a zero-arg closure, so the extra event argument is simply
unused there, matching today's call shape.)

No backend change is required. §2.4's neighbor-promotion logic in
`handle_delete_tab` is correct as written *given* the precondition that
`active_tab_id` isn't mutated by an unrelated close click — fixing §2.1 at
the source restores that precondition rather than papering over it
downstream.

## 4. Test plan

- Unit/component test on `Tab`/`DroppableTab`: simulate a `click` on the
  close button of a non-active tab and assert `onSelect` is never called
  (only `onClose`).
- Manual: open 3+ tabs, click away from the active one, click that
  background tab's "✕" repeatedly (including rapid repeated clicks) —
  confirm the previously-active tab's highlight never blinks and the
  correct tab stays active throughout.
- Regression check: closing the *active* tab (via ✕ or Ctrl+W) still moves
  selection to its neighbor exactly once, with no double `ActiveTabChanged`
  round trip.

## 5. Follow-up — flash when closing the *active* tab via the confirm modal

After §2-3 shipped (PR #2811), a second, independent flash was reported:
closing the **currently active** tab through the close-confirm modal
(`tab:skipcloseconfirm` unset/false, the default) still shows a brief flash
right after clicking "Close tab" in the modal — even though the click-bubble
race from §2 is gone.

### 5.1 Root cause — redundant client-side pre-select turns one atomic update into two

`handleClose` (`tabbar.tsx`, pre-fix) did this when closing the active tab:

```ts
if (tabId === activeTabId()) {
    const idx = allTabs.indexOf(tabId);
    const nextTab = allTabs[idx + 1] ?? allTabs[idx - 1];
    if (nextTab) await setActiveTab(nextTab);   // RPC #1 — round trip, paints
}
await WorkspaceService.CloseTab(props.workspace.oid, tabId);  // RPC #2 — round trip, paints
```

This is a *sequential* two-RPC dance: first move the highlight to the
neighbor (a full `SetActiveTab` round trip that lands and paints on its
own), **then** remove the now-still-visible-but-deselected closing tab (a
second round trip). The user-visible sequence is: `[X active]` → `[X
deselected, neighbor active, X still sitting in the strip]` → `[X gone]` —
the middle frame is the reported flash: the tab you just confirmed closing
visibly un-highlights before it disappears, instead of just vanishing.

But per §2.4/the backend code already cited in this spec,
`agentmux-srv/src/reducer/tab.rs::handle_delete_tab` (`tab.rs:127-137`)
**already** reassigns `active_tab_id` to the correct neighbor *atomically*,
in the same reducer dispatch as the removal, whenever the deleted tab was
active — confirmed by the saga (`agentmux-srv/src/sagas/delete_tab.rs`)
dispatching exactly one `Command::DeleteTab` and by the reducer's own test
`delete_active_tab_promotes_neighbor` (`tab.rs:616-654`). `TabDeleted` and
`ActiveTabChanged` are emitted together from that one call, so the frontend
receives ONE update reflecting the final state (tab gone, correct neighbor
active) from a single `CloseTab` response. The client-side pre-select was
always redundant for correctness — it just happened to also be the thing
splitting one atomic transition into two paintable steps.

The pre-select wasn't pure dead weight, though: `setActiveTab`
(`tab-actions.ts`) also holds the tab-content **reveal gate**
(`holdRevealGate()`/`scheduleRevealLift()`, `SPEC_TAB_CONTENT_REVEAL_GATE.md`)
so the destination tab's panes don't mount piecemeal. Simply deleting the
pre-select without replacing that would reintroduce pane-content flicker on
every active-tab close.

### 5.2 Fix

Drop the separate `setActiveTab` RPC; let `CloseTab`'s own atomic backend
transition handle both the removal and the reassignment in one round trip.
Keep holding the reveal gate — just around the single `CloseTab` call
instead of around a now-removed extra RPC:

```ts
const closingActiveTab = tabId === activeTabId();
fireAndForget(async () => {
    if (closingActiveTab) holdRevealGate();
    try {
        await WorkspaceService.CloseTab(props.workspace.oid, tabId);
        deleteLayoutModelForTab(tabId);
    } finally {
        if (closingActiveTab) scheduleRevealLift();
    }
});
```

No backend change needed — this only removes a redundant frontend round
trip and relies on behavior the reducer already had and already tests.

### 5.3 Test plan (follow-up)

- Manual: with the default close-confirm modal enabled, close the active
  tab via ✕ → confirm — the tab should disappear directly with the neighbor
  already active, no intermediate deselected-but-still-visible frame.
- Regression: pane content on the newly-active tab still mounts atomically
  (no piecemeal reveal) — the reveal gate is still exercised, just around
  the single `CloseTab` call.
- Full `vitest` suite + `tsc --noEmit` — no regressions expected; this is a
  subtractive change (removes a call), not new branching logic.

## 6. Follow-up #2 — the closed tab itself flashes blank in place before disappearing

After §5 shipped, the repo owner reported the flash was still there and
pinned down the precise symptom: **after clicking "Close tab" in the
confirm modal, the closing tab itself visibly flashes (goes blank) in its
own position in the strip for a frame, then disappears** — not a
neighboring tab's highlight; the tab being closed, in place.

### 6.1 Root cause — two unbatched signal writes from one atomic server response

`CloseTab`'s HTTP handler (`agentmux-srv/src/server/service/tab_lifecycle.rs::handle_close_tab`,
`tab_lifecycle.rs:258-279`) already does the right thing server-side: it
runs the `delete_tab` saga (one atomic reducer transition — see §5.1) and
then returns **both** resulting object changes in a single response:

```rust
let mut updates = vec![WaveObjUpdate {
    updatetype: "delete", otype: OTYPE_TAB, oid: tab_id.clone(), obj: None,
}];
if let Ok(ws) = store.must_get::<Workspace>(&ws_id) {
    updates.push(WaveObjUpdate {
        updatetype: "update", otype: OTYPE_WORKSPACE, oid: ws_id.clone(),
        obj: Some(wave_obj_to_value(&ws)),
    });
}
```

Note the order: the **tab delete comes first**, the **workspace update
second**. The frontend applies this array via `updateWaveObjects`
(`frontend/app/store/wos.ts`, pre-fix):

```ts
function updateWaveObjects(vals: WaveObjUpdate[]) {
    for (const val of vals) {
        updateWaveObject(val);   // wov.setData(...) — a raw Solid signal write
    }
}
```

Each `updateWaveObject` call is a bare `setData(...)` on that object's own
Solid signal, with **no `batch()` wrapper**. Solid propagates every signal
write's dependent effects synchronously and independently unless the
writes are batched together. So processing this specific two-item array
does, in order:

1. **Delete the Tab object** — the closing tab's own `useWaveObjectValue<Tab>`
   signal (subscribed by its still-mounted `<Tab>` component, `tab.tsx:118`)
   goes to `{ value: null }`. `tabData()` becomes `null`, so
   `tabData()?.name` (`tab.tsx:285`) renders empty. The tab's DOM node is
   **still mounted** at this point — nothing has told the parent `<For
   each={tabIds()}>` (`tabbar.tsx`) to remove it yet, because `tabIds()` is
   derived from the **workspace** object, not the tab object. Net visible
   effect: the tab's own slot in the strip goes blank, in place, while
   still fully sized and present.
2. **Update the Workspace object** — *now* `workspace()`/`tabIds()`
   recompute, and `<For>` finally unmounts the DroppableTab/Tab for the
   removed id.

Step 1 and step 2 are two separate, independently-propagated Solid
updates from ONE atomic server response — exactly the blank-flash-then-
vanish sequence reported. (This is the same root cause class as §5 —
one atomic backend transition being fragmented into multiple visible
frontend steps — just one layer further down, at the generic WaveObject
update-application layer rather than at the RPC-call layer.)

### 6.2 Fix

`updateWaveObjects` is a shared primitive used by every backend RPC
response that returns multiple related `updates` — not just `CloseTab`.
Wrap its loop in Solid's `batch()` so all updates from one response apply
as a single atomic reactive flush, regardless of how many objects changed
or what order the backend lists them in:

```ts
import { batch, createSignal, onCleanup } from "solid-js";

function updateWaveObjects(vals: WaveObjUpdate[]) {
    batch(() => {
        for (const val of vals) {
            updateWaveObject(val);
        }
    });
}
```

This is the general fix (root-caused at the shared update-application
layer, not special-cased per RPC), and incidentally also closes off the
same class of bug for any other handler that already returns, or might in
future return, more than one `WaveObjUpdate` per call.

### 6.3 Test plan (follow-up #2)

- Manual: close the active tab via the confirm modal — the tab should
  vanish directly, with no intermediate blank/empty frame in its old slot.
- Full `vitest` suite + `tsc --noEmit` — no regressions expected (a `for`
  loop wrapped in `batch()` preserves the same update order and end state,
  only changes when Solid flushes the resulting DOM writes).

## 7. Follow-up #3 — the flash survives §6 because the WS push path applies the same pair unbatched, and it always paints first

After §6, the repo owner reported the flash was STILL there, right after
the close-confirm modal dismisses. §6's `batch()` was correct but fixed
only one of the delivery paths — and, as it turns out, the one that never
actually paints.

### 7.1 Root cause — three delivery paths, two of them unbatched, both faster than the one §6 fixed

A `CloseTab`'s `[delete tab, update workspace]` pair reaches the calling
renderer THREE separate ways:

1. **The wave-obj bridge** (`agentmux-srv/src/server/wave_obj_bridge.rs`,
   `Event::TabDeleted` arm): broadcast as **two separate WS frames** —
   tab delete first, then (after an async `spawn_blocking` SQLite fetch)
   the workspace update. Fired the moment the reducer publishes the event,
   i.e. *while the HTTP handler is still running*.
2. **The response-broadcast loop** (`run_service_call`,
   `agentmux-srv/src/server/service/mod.rs`): pre-fix, one WS frame **per
   update**, in the handler's array order — tab delete first. Fired after
   the handler returns but before the HTTP response body is serialized to
   the caller; the comment said "for everybody else on the event bus," but
   `broadcast_event` sends to every connection, including the caller.
3. **The HTTP response body** (`respData.updates` in `wos.ts`
   `callBackendService`) — the only path §6 batched. Arrives last.

Server-side, `forward_event` (`server/websocket.rs`) wraps each raw
`waveobj:update` frame as an `eventrecv` RPC message, so each frame lands
in `handleWaveEvent` → the scope-less `WaveObjUpdate` subscription in
`initGlobalEventSubs` (`global.ts`) → a **bare, per-frame
`WOS.updateWaveObject()` call** — no `batch()` anywhere on this path.

So the actual paint sequence was: WS frame "delete tab:X" → the
still-mounted `<Tab>` blanks in place (the §6 flash, verbatim) → WS frame
"update workspace" → tab unmounts → HTTP body's batched pair arrives and
is a no-op (`updateWaveObject`'s version guard skips the same-version
workspace update; the tab delete is already applied). §6's `batch()` never
had a chance to win the paint — the flash was deterministic, not even a
race.

### 7.2 Fix — batch the response broadcast; order the bridge parent-first

Two changes, mirroring §6's "fix it at the shared layer" principle:

- **`EventBus::broadcast_wave_obj_updates`** (`backend/eventbus.rs`): a
  response's whole `updates` array now goes out as ONE
  `waveobj:batchedupdates` WS frame (order preserved). The frontend
  subscribes to it in `initGlobalEventSubs` and applies via the
  already-batched `updateWaveObjects`. All five multi-update broadcast
  sites switched to it: `run_service_call`, `app_api/pane.rs`,
  `app_api/mod.rs`, `app_api/agent_open.rs`, `service/tear_off.rs`.
  Single-update emitters (blockcontroller, setmeta, the bridge) keep the
  plain `waveobj:update` frame — nothing to batch.
- **Bridge delete ordering** (`wave_obj_bridge.rs`): the `TabDeleted`,
  `BlockDeleted`, and `SrvWindowClosed` arms now emit the **parent update
  BEFORE the child delete** (the mirror of the create arms' child-first
  order). The bridge's two emissions are genuinely separate frames (one
  requires an async fetch), so they can't be batched into one — but
  parent-first, the child unmounts with its data still intact and the late
  delete lands on an unsubscribed signal: nothing paints. General rule
  worth keeping: **create child-first, delete parent-first.**

With both in place, every arrival order is flash-free: bridge frames are
parent-first, the response broadcast is atomic, the response body is
atomic (§6).

### 7.3 Test plan (follow-up #3)

- `wave_obj_bridge.rs::tests` — `TabDeleted` broadcasts the workspace
  update before the tab delete; `BlockDeleted` broadcasts the tab update
  before the block delete (asserted on a real `EventBus` connection's
  receive order).
- `eventbus.rs::tests` — `broadcast_wave_obj_updates` emits exactly one
  `waveobj:batchedupdates` frame with the array order preserved, and no
  frame for an empty slice.
- `frontend/app/tab/tab.test.tsx` — §4's long-owed component test: a click
  on a background tab's close button fires `onClose` only (never
  `onSelect`); a click on the tab body still selects.
- Manual: close the active tab via the confirm modal — the tab vanishes
  with the neighbor already active; no blank frame, no highlight blink.

## 8. Follow-up #4 — optimistic removal: make the flash structurally impossible

§§2-7 each closed one specific ordering hole, and after all four the flash
was STILL reproducible on the repo owner's machine ("after I click ok on
the modal, the closed tab will flash in its place before disappearing").
Two lessons:

1. **Verification gap:** none of §§5-7 was ever verified against a build
   actually containing all of them at once (v0.55.25 predates the merge;
   the dev instance used for testing had only §§3-6). Some sightings may
   have been of incomplete builds. But regardless —
2. **Wrong fix class:** every fix so far tries to *win an ordering race*
   between the HTTP response, individual WS frames, reveal-gate timing,
   and Solid's flush boundaries. Each new transport, event arm, or
   scheduler change can reopen the class. The strip's rendering of the
   closing tab should not *depend* on backend update ordering at all.

### 8.1 The directive (repo owner, verbatim intent)

> "the closed tab should leave right when the modal is open, if the user
> cancels, put the tab back"

That is optimistic UI, and it is ordering-immune by construction: a tab
that is not rendered cannot flash, whatever order the backend's updates
arrive in.

### 8.2 Design (`frontend/app/tab/tabbar.tsx`)

- `pendingHiddenTabIds: Signal<ReadonlySet<string>>` — ids hidden from the
  strip while a close is pending. A Set so overlapping skip-confirm closes
  of different tabs each stay hidden.
- `allTabIds()` — the raw workspace list (logic/guards);
  `tabIds()` — the rendered list, `allTabIds()` minus hidden ids.
- **Hide points:** modal path — `requestClose` hides the tab the moment it
  opens the modal (the directive); skip-confirm path — `handleClose` hides
  before firing the RPC.
- **Restore points:** modal `onCancel` unhides; `handleClose`'s `finally`
  unhides — on success the RPC response has already applied the workspace
  update synchronously (the id is gone from `allTabIds()`, unhide is a
  no-op), on failure the tab visibly returns rather than leaking as
  invisibly-alive.
- **Guards use the raw list:** `requestClose`/`handleClose` check
  `allTabIds().length <= 1`. Checking the filtered list would make a
  2-tab workspace read as 1 tab after the modal hid one, and refuse every
  confirmed close.
- **`displayActiveTabId()`** — while the real active tab is hidden
  mid-close, the strip highlights the neighbor the backend is about to
  promote (next in list, else previous — mirroring `handle_delete_tab`'s
  `tab_ids.get(pos) ?? pos-1`). The strip shows the final post-close state
  from the first frame; the backend's update then changes nothing visibly.
  `activeIndex`/`isActive`/`isBeforeActive` all key off it.
- Re-entry guard: `requestClose` no-ops for a tab already pending close.

The reveal gate (§5) is unchanged and still covers the *content* region on
active-tab close; this section covers the strip.

### 8.3 What §§3-7 still buy us

Optimistic hiding makes the strip immune, but §§3-7 remain correct and
load-bearing: §3 (click containment) prevents a spurious SetActiveTab RPC
entirely; §5 keeps the close a single atomic backend transition; §6/§7
(batched application + parent-first delete ordering) protect every OTHER
multi-object update path (block close, window close, tab create) that has
no optimistic layer in front of it.

### 8.4 Test plan

- Modal path: ✕ on active tab → tab disappears from strip immediately,
  neighbor highlighted, modal open → Cancel → tab reappears in place, its
  highlight state restored.
- Modal path: ✕ → Confirm → no visible change in the strip at all (it
  already showed the final state); content area switches atomically under
  the reveal gate.
- Skip-confirm path: rapid ✕ clicks on several background tabs — each
  vanishes on click, none reappears, no flash.
- Failure injection: CloseTab RPC rejects → tab returns to the strip.
- 2-tab workspace: modal open on one tab → Confirm still closes (raw-list
  guard); the surviving tab cannot be closed (✕ hidden / guard).
- `tsc --noEmit` + full `vitest` — no regressions.

## 9. Follow-up #5 — the promoted neighbor's PANE flashes after §8

With §8 in a verified build, the repo owner confirmed the strip flash is
gone but reported: "after the modal, when the tab is gone, the next tab's
entire pane flashes."

### 9.1 Root cause — the reveal gate blanks the SOURCE for the whole round trip

`workspace.tsx` keeps every tab's content mounted (`display:none` when
inactive) and applies the reveal gate as `visibility:hidden`/`opacity:0`
to whichever tab is **currently active** while `tabSwitching` is up
(`tid === tabId() && tabSwitching()`). On close-promotion:

1. Confirm → `handleClose` → `holdRevealGate()`. `tabId()` is still the
   CLOSING tab X → **X's fully-rendered pane blanks instantly.**
2. CloseTab RPC round-trips (~tens of ms). Content region shows blank.
3. Update lands: X unmounts, neighbor Y flips `display:none → flex` and —
   `tid === tabId()` now matching Y — stays hidden under the gate.
4. `scheduleRevealLift()`'s settle detector waits for 80ms of
   long-task-free frames. Tab teardown (block/terminal disposal, layout
   model deletion) emits long tasks that keep resetting that clock, so
   the blank stretches toward the 800ms hard cap before Y fades in over
   120ms.

Net effect: blank-from-confirm → (long settle) → fade-in = "the next
tab's entire pane flashes." The same source-blanking happens on ordinary
tab switches (the gate has always keyed on the current active tab), but
the switch case is shorter (no teardown long-tasks) and so was never
reported.

### 9.2 Fix — destination-targeted gate

The holder usually KNOWS the destination tab. Let it say so:

- `tab-reveal.ts`: `holdRevealGate(targetTabId?: string|null)` records a
  `gateTargetTabId` signal; every lift path (settle, hard cap, safety
  net) clears it. Untargeted holds keep the legacy behavior.
- `workspace.tsx`: hide only when
  `tid === tabId() && tabSwitching() && (target == null || target === tid)`.
- `tabbar.tsx handleClose`: passes `displayActiveTabId()` (computed after
  the optimistic hide, so it resolves to the neighbor the backend is
  about to promote — §8's same promotion mirror).
- `tab-actions.ts setActiveTab`: passes its destination `tabId` — regular
  tab switches get the same improvement (source keeps painting through
  the RPC; only the destination is FOUC-gated, from the flip until
  settle).
- `createTab` stays untargeted: its destination id doesn't exist until
  the RPC returns, and hiding the current tab during creation is the
  established behavior.

What the user now sees on close-promotion: X's content stays on screen
through the RPC; at the flip, Y is hidden only for its own settle window
(teardown tasks overlap it), then fades in. The blank no longer starts at
confirm-click and no longer covers the round trip.

### 9.3 Test plan

- `tab-reveal.test.ts`: targeted hold records the target; untargeted hold
  resets a stale target; every lift path (schedule fallback, MAX_GATE
  safety net) clears it.
- Manual: close the active tab via the modal — the closing pane stays
  visible until the moment of the switch; the neighbor appears with at
  most its own brief settle, no full-region blank from the confirm click.
- Manual regression: ordinary tab switch — source no longer blanks during
  the RPC; destination still reveals atomically (no piecemeal paint).
  Startup reveal (app-init's untargeted `scheduleRevealLift`) unchanged.
