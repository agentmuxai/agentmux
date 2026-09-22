# Selecting a pane should focus its input — not just opening one

**Status:** proposed — problem confirmed in code, design not yet agreed, nothing
implemented.
**Date:** 2026-09-22.
**Severity:** Medium — no data is lost, but every pane switch or pane open costs
the user a mouse trip before they can type, on the app's most-used, most
keyboard-driven surfaces (agent chat above all).
**Requested by:** repo owner (asafebgi).
**Extends:** `SPEC_PANE_OPEN_FOCUS_ROUTING_2026_09_16.md` — that spec covers
pane *creation* and leaves whether this also applies to *selecting an existing
pane* as an open question (its Q2). This spec answers Q2 (yes), reframes the
agent pane as the primary case rather than an incidental one, and adds a
fourth gap the prior spec didn't scope: **tab switch never invokes the focus
contract at all.**
**Related:** `SPEC_EDITOR_FIRST_KEYSTROKE_NOT_RENDERED_2026_09_15.md` — a
different, already-fixed bug (#3260) with a similar-looking symptom. See §6 of
the prior spec; that analysis is not repeated here.

---

## 1. What we want

Select an agent pane — by clicking it, switching tabs to it, tabbing to it
with the keyboard, or opening it fresh — and the input box already has the
caret. Same for editor and terminal panes. No click into the text area
required, ever, as long as the pane is the one now selected.

Today: selecting a pane only ever changes which pane *looks* active. Whether
the caret follows is inconsistent — and for the agent pane, one of the two
most-used surfaces in the app, it never does.

## 2. Current state, confirmed against `main` (`14926be7f`)

The prior spec (§2) already established that every focus hand-off funnels
through the same duplicated pattern:

```ts
const ok = bcm?.viewModel?.giveFocus?.();
if (!ok) {
    const inputElem = document.getElementById(`${blockId}-dummy-focus`);
    inputElem?.focus();
}
```

and that this pattern is a **single synchronous attempt with no retry**,
appearing in three places: `focusManager.refocusNode()`
(`frontend/app/store/focusManager.ts:37-49`), `refocusNode(blockId)`
(`frontend/app/store/block-component-registry.ts:169-183` — a second,
differently-implemented function with the same name), and
`openOrFocusPaneByView()` (`block-component-registry.ts:191-208`, focus-only
branch at line 202; the create branch never calls `giveFocus()` at all).

Re-verified for this spec:

| `giveFocus()` implementation | Location | Status |
|---|---|---|
| Agent | `frontend/app/view/agent/agent-model.ts:890-892` | **stub — `return false`** |
| Editor | `frontend/app/view/editor/editor-model.ts:1258` | **stub — `return false`** |
| Terminal | `frontend/app/view/term/termViewModel.ts:499` | implemented, races mount (calls `termRef.current.terminal.focus()` only if the ref is already populated) |
| Browser | `frontend/app/view/browser/browser-model.ts:711` | real implementation — out of scope, works today |

The prior spec treated the agent-pane stub as something that "falls out for
free" from fixing the shared contract (its §2c, §4). **This spec elevates it
to the primary case**, since the feature request names the agent pane first
and it is the surface most users spend the most time in.

### 2a. New gap not covered by the prior spec: tab switch does zero focus work

`setActiveTab()` (`frontend/app/store/tab-actions.ts:92-135`) is the entry
point for every tab switch — tab-bar click, `Ctrl+Tab`/`Ctrl+1..9` (keymodel),
command palette, and the CEF test API. Read in full for this spec: it awaits
`WorkspaceService.SetActiveTab(ws.oid, tabId)`, manages a reveal gate and
tab-switch perf marks, and returns. **It never calls `focusManager`,
`giveFocus()`, or anything focus-related.** After a tab switch, whatever pane
was focused in the destination tab is *shown* as focused (layout state is
per-tab and persists), but the caret is wherever it happened to be left —
typically nowhere, since the dummy-focus fallback doesn't fire either.

This is a fourth, independent gap alongside the three the prior spec found:
editor stub, agent stub, terminal race, creation-path never-invoked — and now,
**tab-switch path never-invoked.**

### 2b. Within-tab pane selection: already wired, but to a no-op

Selecting a different pane *within* the same tab (click, arrow-key nav via
`switchNodeFocusInDirection`, `Cmd+1..9` via `switchNodeFocusByBlockNum` —
both in `frontend/layout/lib/layoutFocus.ts`) does dispatch through the
layout tree's `FocusNode` reducer case
(`frontend/layout/lib/layoutModel.ts:634-637`):

```ts
case LayoutTreeActionType.FocusNode:
    focusNode(this.treeState, action as LayoutTreeFocusNodeAction);
    focusManager.requestNodeFocus();
```

But `requestNodeFocus()` (`frontend/app/store/focusManager.ts:29-31`) is:

```ts
requestNodeFocus(): void {
    // no-op
}
```

So the mechanism to reach every pane-selection event already exists and is
already called at the right time — it just does nothing. This is good news
for the design: unlike tab switch (§2a, wired to nothing), within-tab pane
selection is wired to a stub, which is a smaller fix.

The one real implementation, `focusManager.refocusNode()`
(`focusManager.ts:37-49`), is currently dead code — `grep` finds no caller
except `setBlockFocus()`, which itself has no caller anywhere in the
codebase.

## 3. Design

**Recommendation: adopt the prior spec's Option B (retry-capable focus
contract) exactly as designed there, and wire it into three call sites, not
one:**

1. **Pane creation** (prior spec §2d/§3) — `createBlock()`
   (`frontend/app/store/block-layout-actions.ts:68-86`) inserts a node with
   `focused: true` but never invokes the contract at all.
2. **Within-tab pane selection** (§2b above) — make
   `focusManager.requestNodeFocus()` do what `refocusNode()` already does,
   instead of being a no-op. This one is close to a one-line fix once Option
   B's retry mechanism exists, since the call site is already correctly
   placed in the `FocusNode` reducer case.
3. **Tab switch** (§2a above) — `setActiveTab()` needs an explicit call,
   after the destination tab's workspace state lands, to focus that tab's
   currently-focused node. This is new wiring the prior spec didn't scope,
   since it only looked at the three `giveFocus()` call sites and tab switch
   isn't one of them.

All three should resolve to the **same underlying call** (`giveFocus()` on
the focused block's view model, with the retry/readiness semantics Option B
adds), so there is still exactly one focus contract — just three legitimate
triggers for it instead of one.

### 3a. Fixing the agent pane's stub — a precedent already exists in-repo

`AgentFooter.tsx` already solves an adjacent problem: giving the
`AgentViewModel` a live handle into the mounted textarea, for voice
dictation. `agent-model.ts:84-88` declares
`voiceTargetRef: { current: PaneVoiceHandle | null }`; `AgentFooter.tsx`'s
`onMount` block (lines 727-780) constructs a handle wrapping `textareaRef`
and registers it via `vm.voiceTargetRef.current = handle`, clearing it in
`onCleanup`.

`giveFocus()` should follow the identical shape: register a focus handle (or
extend `PaneVoiceHandle`) the same way, so `AgentViewModel.giveFocus()` can
call `textareaRef.focus()` through it instead of returning `false`
unconditionally. This also naturally gives the "not mounted yet" signal
Option B's retry needs — the handle is `null` until `onMount` runs.

The editor's stub (`editor-model.ts:1258`) needs the same treatment:
`cmView` is currently a variable private to `editor-view.tsx`
(`editor-view.tsx:87`, assigned at line 415) and needs an equivalent
hand-off to the model before `giveFocus()` can call `cmView.focus()`.

## 4. Scope

- **In:** agent panes, editor panes, terminal panes — on creation (per prior
  spec), on within-tab selection (§2b, new), and on tab switch (§2a, new).
- **Primary case:** the agent pane. Named first in the request and the
  highest-traffic surface; treat its fix as required, not a side effect of
  the shared-contract change.
- **Out:** the browser pane (already implements the contract correctly);
  changing *which* pane gets selected by any given action — this is only
  about where the caret goes once selection has already happened.

## 5. Constraints — when NOT to take focus

All four constraints from the prior spec (§5) carry over unchanged and now
apply to tab switch and within-tab selection too, not only creation:

1. Never steal focus from a pane the user is already typing in.
2. Never steal focus from a modal or an open search panel — the terminal's
   `searchAtoms.isOpen()` guard must keep working under whatever mechanism
   replaces the stub/no-op calls.
3. Restoring a saved layout on startup is not the same as selecting a pane —
   don't have every restored pane fight over the caret.
4. Read-only or error states (editor load error, dead terminal) probably
   should not take the caret (Q3, still open).

One new constraint specific to this spec's added scope:

5. **A tab switch is always an explicit user action** (click, keyboard
   shortcut, palette) — unlike pane *creation*, which can happen in the
   background (e.g. an agent opening a file via `muxsh` in a tab the user
   isn't looking at). So tab switch should focus its destination pane
   unconditionally, with no "is this in the background" check — the whole
   point of switching to a tab is that it's no longer in the background. This
   is a simpler rule than creation's, not a stricter one.

## 6. Acceptance

Carries the prior spec's two creation criteria, plus new ones for selection
and tab switch, plus the agent pane made first-class:

- Open a new agent, editor, or terminal pane and type immediately —
  characters reach the composer / document / shell.
- Click from one pane to another within a tab (agent, editor, or terminal) —
  the destination's input has the caret with no extra click.
- Arrow-key or `Cmd+1..9` pane navigation within a tab moves the caret along
  with the visual focus.
- Switch tabs (tab-bar click, `Ctrl+Tab`, `Ctrl+1..9`, palette) to a tab whose
  currently-focused pane is agent, editor, or terminal — the caret is there
  immediately, no click.
- A pane opened or updated in the background while the user is typing in
  another pane does not move the caret.
- A modal or the editor's find panel open — selecting or switching panes
  behind it does not steal the caret from the modal/panel.
- Restoring a multi-pane layout on startup does not cause panes to fight over
  the caret.
- No pane silently leaves the caret on a `*-dummy-focus` element when its
  view is mountable and not in a read-only/error state.

## 7. Open questions

Carried over from the prior spec, still unresolved:

- **Q1.** Tri-state `giveFocus()` return vs. `focusWhenReady(): Promise<boolean>`
  for the retry mechanism.
- **Q3.** Should a pane in an error/read-only state take the caret at all?
- **Q4.** Is the `*-dummy-focus` fallback still needed once views focus
  reliably, or is it masking bugs it should surface?
- **Q5.** Retry bounded by attempts, by a timer, or by the block-component
  registry's existing mount signal (the registry already knows when a block
  component mounts — likely the cleanest bound).

Closed by this spec:

- ~~**Q2.** Does this apply to focusing an existing pane (tab switch)?~~ —
  **Yes.** §2a/§2b/§3 above; both within-tab selection and tab switch are now
  in scope, with tab switch identified as a fourth, previously-unscoped gap
  (setActiveTab calls no focus code at all today).

New, from the extended scope:

- **Q6.** If the app's own window doesn't have OS-level focus, should a tab
  switch still move the in-app caret (so it's ready the instant the window is
  focused), or wait? Likely wait / doesn't matter, but worth an explicit
  decision since tab switches can be triggered programmatically (palette,
  test API) while the window is backgrounded.
- **Q7.** Precedence if a tab switch and a background pane-creation land in
  the same tick (unlikely, but the constraint set in §5 should be unambiguous
  about which wins). Proposed answer given constraint 5: tab switch wins,
  since it's the explicit user action.

## 8. Files a future implementer will touch

Grounded in this investigation, not exhaustive:

- `frontend/app/store/focusManager.ts` — make `requestNodeFocus()` real
  (§2b); likely becomes the single shared entry point all three triggers call.
- `frontend/app/store/tab-actions.ts` (`setActiveTab`, line 92) — new call
  into the focus contract after the destination tab's state lands (§2a).
- `frontend/app/store/block-component-registry.ts` — reconcile with the
  second `refocusNode()` defined here (line 169) and `openOrFocusPaneByView`'s
  create branch (line ~205), per the prior spec's §2d.
- `frontend/app/store/block-layout-actions.ts` (`createBlock`, line 68) —
  creation-path wiring per the prior spec.
- `frontend/app/view/agent/agent-model.ts:890` +
  `frontend/app/view/agent/components/AgentFooter.tsx` — real `giveFocus()`
  for the agent pane, via a registered handle (§3a).
- `frontend/app/view/editor/editor-model.ts:1258` +
  `frontend/app/view/editor/editor-view.tsx` — real `giveFocus()` for the
  editor pane, needs `cmView` exposed to the model (§3a).
- `frontend/app/view/term/termViewModel.ts:499` — extend the existing
  implementation with the retry/readiness mechanism from Option A instead of
  a one-shot ref check.

## 9. Non-goals

- Changing which pane gets selected by any action (click target, tab-switch
  destination, keyboard-nav direction) — unchanged.
- The browser pane's focus behavior — already correct.
- Any change to voice dictation (`PaneVoiceHandle`) beyond reusing its
  registration pattern as precedent.
