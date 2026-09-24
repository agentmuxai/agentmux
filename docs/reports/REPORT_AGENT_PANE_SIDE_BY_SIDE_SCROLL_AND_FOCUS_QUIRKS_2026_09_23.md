# Report: Agent pane side-by-side quirks — spontaneous scroll to "New Agent", and double-click to focus "Filter agents..."

**Status:** analysis — root causes for the fixes in PR #3621.
**Date:** 2026-09-23
**Base:** `main` @ `61231d41c`
**Method:** static analysis of the frontend focus/layout code paths. Not yet
reproduced under instrumentation in the running app — see §5 for the
verification plan.

> **Issue 2 is already covered by `SPEC_PANE_CLICK_THROUGH_INPUT_FOCUS_2026_09_23.md`**
> (agent2, branch `agent2/spec-pane-click-through-input-focus`). That spec is
> the authoritative fix for it: a shared `userCaretInBlock()` guard at all three
> focus call sites, plus a full inventory of every affected surface. §3–§4 below
> reached the same root cause independently and are kept only as corroboration.
> **Issue 1 (the scroll) is NOT in that spec** and is still open here. Fix B in §4
> depends on that spec's `claimFocusOnMount()` guard.
>
> **Update:** both issues and all of §6 except the backfill tracker shipped
> in #3621. The spec's §3 row P4 and §8 describe exactly what shipped.
>
> **Update 2 (2026-09-24):** two more fixes shipped on branch
> `agent1/two-pane-agent-quirks`:
> - The §6 backfill tracker is now per pane. A backfilling pane's rows are
>   held until it settles; every other pane's Activity Dock keeps updating.
> - The Ctrl+F focus-source mismatch (see the codebase map) is fixed: the
>   pane the key came from, else the selected pane, never the text selection.

Scenario: two agent panes side by side, both showing the **AgentPicker**
(pre-launch view: "Filter agents..." bar → My Agents list → "New Agent"
header → template cards).

---

## 1. Summary

| # | Symptom | Root cause | Confidence |
|---|---------|------------|------------|
| 1 | Pane spontaneously scrolls down to the "New Agent" section | The first template card calls `el.focus()` (no `preventScroll`) in `onMount`, and the whole card list **remounts on every `agents:changed` event** | High |
| 2 | Clicking "Filter agents..." in the *other* pane needs two clicks | The first click focuses the input, which selects the pane, and selecting the pane (since #3519) immediately moves focus to the pane's "default focus target". The picker has none, so focus goes to the hidden `dummy-focus` input and the caret is taken away from the Filter box | High |

Both come from the same design gap: **several code paths move DOM focus by
themselves, and none of them checks whether the user already put focus
somewhere inside the target pane.** There are three near-copies of "focus a
block" logic, plus one component (AgentCard) that skips the shared guard
entirely. §4 proposes making it DRY.

---

## 2. Issue 1 — spontaneous scroll to "New Agent"

### Mechanism

`frontend/app/view/agent/components/AgentCard.tsx:83-85`

```ts
onMount(() => {
    if (props.defaultFocus && !props.disabled) cardEl?.focus();
});
```

`frontend/app/view/agent/components/AgentPicker.tsx:1016-1039` renders the
template cards with `defaultFocus={index() === 0}`. The first template card
sits directly under the **"New Agent"** header (`AgentPicker.tsx:1008-1010`).
`HTMLElement.focus()` without `{ preventScroll: true }` scrolls the nearest
scroll container (`.agent-picker`, `overflow-y: auto`,
`styles/_picker.scss:19`) so that the card is in view. That is exactly the
"jump to NEW AGENT" the user sees.

### Why it's "random": what remounts the card

`useAgentDefinitions()` (`AgentPicker.tsx:103-139`) refetches the full list
on **every** `agents:changed` event and calls `setAgents(result)` with brand-new
objects. `templates()` (`:929`) is a memo over that list, and Solid's `<For>`
is keyed by **object identity**. So every refetch unmounts and remounts **every**
`AgentCard`, and card 0's `onMount` calls `focus()` again.

`agents:changed` is broadcast by srv for any definition create, update,
delete, fork, hide/unhide, template create, reseed or App API
`agent.define` (`agentmux-srv/src/server/agent_handlers/core.rs:172,290,315,517,546,675`,
`template.rs:246,300,332,507,609`, `app_api/agent_define.rs`). In a fleet
where other agents define or fork agents, this happens at times the user
neither triggers nor sees. That matches "sometimes, not clear when."

### Side effects beyond the scroll

- **It steals keyboard focus across panes.** This `focus()` never checks
  which pane is selected. A refetch while the user is typing in pane A's
  filter box, pane B's composer, or even another tab (agent tabs stay
  mounted per #3391) moves the caret into whichever picker's card 0 mounted
  last.
- The steal then fires `handleChildFocus` (`block.tsx:168-171`), which
  **also switches the selected pane** to that picker.
- It fires on first mount too. That is intended (the "Enter launches the
  default" behaviour), but even that should not scroll, and should only
  happen when this pane is the active, selected one.

`focusManager.claimFocusOnMount()` (`frontend/app/store/focusManager.ts:75-81`)
exists for exactly this "focus on mount, but only if I'm the selected pane in
the active tab" case. AgentFooter, the editor and the terminal all use it;
AgentCard does not.

---

## 3. Issue 2 — "Filter agents..." needs two clicks in the unselected pane

### Event chain on the first click (pane B not selected, pane A selected)

1. **mousedown** → the browser focuses pane B's Filter `<input>`
   (`AgentPickerFilterBar.tsx:55-63`).
2. **focusin (capture)** → `handleChildFocus` (`frontend/app/block/block.tsx:168-171`):
   B is not selected, so `nodeModel.focusNode()` runs.
3. → `focusNode` (`frontend/layout/lib/layoutFocus.ts:152-170`) dispatches
   `LayoutTreeActionType.FocusNode`.
4. → `treeReducer` sets `shouldRequestFocus = true` for FocusNode
   (`frontend/layout/lib/layoutModel.ts:646-649`), commits state, then calls
   `focusManager.requestNodeFocus()` (`:694-696`).
5. → `requestNodeFocus()` → `refocusNode()` (`frontend/app/store/focusManager.ts:37-61`):
   `bcm.viewModel.giveFocus()`. `AgentViewModel.giveFocus()`
   (`frontend/app/view/agent/agent-model.ts:934-939`) returns `false` in picker
   mode because `focusTargetRef` (the composer textarea) doesn't exist yet.
   So it falls back to `document.getElementById(\`${blockId}-dummy-focus\`).focus()`.
   **The caret moves from the Filter input to the invisible 0×0 dummy input.**
6. **click** → `handleBlockClick` (`block.tsx:223-232`): focus is now inside
   B (on the dummy) and B is selected, so nothing else happens.

The user sees a click that "didn't take." On the **second click** B is
already selected, `handleChildFocus` skips `focusNode()`, and the input
keeps focus. The same happens in either direction (A↔B), as reported.

### Why it started recently

Before PR #3519 ("Auto-focus a pane's input on selection",
`SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md`), `requestNodeFocus()` was a no-op
(see the comment at `focusManager.ts:30-36`). #3519 made every FocusNode
dispatch move the caret, **including the one caused by the user's own
focus-in**. `handleBlockClick` already had a "don't refocus if focus is
already within this block" guard (`block.tsx:225-227`), but the new
reducer-level path does not.

### It's not picker-specific

For a **launched** agent pane, `giveFocus()` returns `true` and focuses the
composer. So the first click on *any* other focusable control in an
unselected agent pane, for example the Ctrl+F search input, a
question-panel text field, or a select, gets redirected to the composer.
Terminal and editor panes have the same exposure through their own
`giveFocus()`.

---

## 4. Three copies of "focus a block"

| Location | Guard: "focus already inside this block"? | Dummy fallback uses `preventScroll`? |
|---|---|---|
| `block.tsx:193-199` `setFocusTarget` (called from `handleBlockClick`) | **Yes** (`:225-227`) | **Yes** |
| `focusManager.ts:45-61` `refocusNode()` (called from every FocusNode / InsertNode(focused) / MagnifyNodeToggle via the reducer) | **No** | **No** |
| `block-component-registry.ts:169-185` `refocusNode(blockId)` (keyboard nav, picker "switch to existing", tabs, browser panes) | **No** | **No** |
| `AgentCard.tsx:83-85` (own `focus()`) | **No**, and it doesn't even check which pane is selected | **No** |

Each of these makes its own decision, which is how the reducer path ended up
without the guard `handleBlockClick` already had.

### Proposed fix

**A. One focus primitive, in `focusManager`.**

```ts
/** Move the caret into `blockId` — unless the user already put it there. */
focusBlock(blockId: string, opts: { force?: boolean } = {}): void {
    if (modalsModel.hasOpenModals()) return;
    const active = document.activeElement as HTMLElement | null;
    const activeBlock = active?.closest?.("[data-blockid]")?.getAttribute("data-blockid");
    const onDummy = active?.id === `${blockId}-dummy-focus`;
    if (!opts.force && activeBlock === blockId && !onDummy) return; // user's click already landed
    const ok = getBlockComponentModel(blockId)?.viewModel?.giveFocus?.();
    if (!ok) document.getElementById(`${blockId}-dummy-focus`)?.focus({ preventScroll: true });
}
```

- `focusManager.refocusNode()` becomes "resolve the focused node, then `focusBlock(id)`".
- `block-component-registry.refocusNode(blockId)` becomes "`layoutModel.focusNode(...)`, then `focusBlock(id)`". It is already reached through the reducer, so the explicit call may simply go away.
- `block.tsx` `setFocusTarget` plus the manual `focusWithin` check collapse into `focusManager.focusBlock(nodeModel.blockId)`.
- Use `document.activeElement`, **not** `focusedBlockId()`. `focusutil.ts:43-66` falls back to the *text-selection anchor*, which would wrongly treat "text selected in B, caret in A" as "focus is in B."

This fixes Issue 2 for picker, launched-agent, terminal and editor panes at
once. It keeps #3519's intent: clicking a non-focusable area of a pane, or
keyboard nav, still lands the caret in the composer.

**B. AgentCard default focus goes through the shared guard, without scrolling.**

```ts
onMount(() => {
    if (!props.defaultFocus || props.disabled) return;
    focusManager.claimFocusOnMount(props.blockId, () => {
        cardEl?.focus({ preventScroll: true });
        return true;
    });
});
```

Also, **stop the remount on every refetch.** Either key the list by
`agent.id` (for example `reconcile(result, { key: "id" })` into a store in
`useAgentDefinitions`, which also saves each picker re-rendering every card
on unrelated definition edits), or have the picker own a one-shot
`didAutoFocus` flag so the default-focus only ever fires once per picker
mount. The reconcile route is better because it also removes the render
churn.

**C. (Optional, same spec spirit.)** Give the picker a real focus target.
Point `AgentViewModel.focusTargetRef` at the Filter input while the picker is
shown, so selecting a picker pane lands the caret in "Filter agents..."
instead of the dummy input. This keeps the fix consistent with #3519's
"selecting a pane focuses its input" intent.

---

## 5. Verification plan

1. **Instrument first** (per the systematic-debugging skill): log
   `document.activeElement` in a `focusin` listener on `document`, plus
   `agents:changed` receipts. Then confirm in the running app:
   - Issue 1: each scroll jump coincides with an `agents:changed` event and a
     focusin on `.agent-card` inside the picker. It can be triggered on
     demand by hiding/unhiding a template or running `agent.define` from
     another agent.
   - Issue 2: the first click shows focusin on `.agent-picker-filter-input`
     immediately followed by focusin on `#<blockId>-dummy-focus`.
2. **Regression tests (vitest):**
   - `focusManager.focusBlock` doesn't move focus when `activeElement` is
     already a non-dummy element inside the block, and does when focus is in
     another block or on the dummy.
   - AgentPicker: firing `agents:changed` doesn't change `activeElement`
     when focus is elsewhere, and never calls `focus` without
     `preventScroll`.
   - Two-pane test: focusing the Filter input of the unselected pane leaves
     it as `activeElement` after the FocusNode reducer runs.
3. **Manual:** two picker panes side by side; one click on either Filter box
   takes the caret. Scroll one picker to the top, then hide/unhide a template
   from the other; neither pane scrolls or loses the caret.

---

## 6. Related findings (same two-pane class, not part of the reported symptoms)

Found during the same sweep. Worth fixing in the same pass because they're
the same "global listener or focus with no pane scoping" pattern:

- `SlashCommandPicker.tsx:47-88`: a document-level capture keydown with no
  pane check. With the picker open in A, typing in B drives A's picker, and
  Enter both commits A's choice and sends B's message
  (`AgentFooter.tsx:1045` doesn't check `defaultPrevented`).
- `SlashHelpPanel.tsx:72-80` and `BtwOverlay.tsx:156-177`: Escape (and, for
  BtwOverlay, any outside pointerdown, including a click into the other pane)
  closes pane A's overlay.
- `MyAgentsList.tsx:568-573`: document listeners attached at component
  setup. An Escape anywhere closes every pane's row menu.
- `activity/backfill-tracker.ts:41,73,122-135`: a backfill in one pane
  suppresses Activity Dock refreshes in **all** panes (20s fallback).
