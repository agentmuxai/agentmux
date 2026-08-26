# Tab close (X) button — spurious select flash

**Status:** §§2-3 (click-bubble race) fixed and shipped (PR #2811). §5 below
is a follow-up: a second, independent flash reported after §2-3 landed —
fixed in the same branch.
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
