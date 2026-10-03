# When a pane closes, the pane that takes over gets the caret

**Status:** proposed.
**Date:** 2026-10-03.
**Severity:** Medium. No data is lost, but every close costs a mouse trip before the user can type again, and the most common closes (`exit` in a terminal, `/quit` in an agent pane) are keyboard actions where the user's hands are already on the keys.
**Requested by:** repo owner (asafebgi).
**Extends:** `SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md` (#3519). That spec made *selecting* a pane focus its input: click, arrow keys, Cmd+1..9, creation, tab switch. Pane close was not in its scope, and today it is the one way a pane becomes selected without its input getting focus.
**Related, separate bug:** focus lost mid-typing to a hidden pool window, fixed in #4306 (its report, `docs/reports/REPORT_INPUT_FOCUS_STOLEN_BY_POOL_REFILL_2026_10_03.md`, lands with that PR). Different cause, similar symptom; not repeated here.

---

## 1. What we want

> "If I type `exit` in a terminal (or `/quit` in an agent pane), if I land on either a terminal, editor, agent pane, or anything with an input, I am able to type right away. So if a pane closes, the next pane that is selected gets its input going."

Close the pane you are working in, by any route, and the pane that becomes selected already has the caret: the terminal's prompt, the agent's composer, the editor's text, a browser pane's page. No click needed.

Today the next pane *looks* selected (it gets the `block-focused` border), but `document.activeElement` is `<body>`. Keystrokes go nowhere until the user clicks.

## 2. Current state, confirmed against `main` (`9d901f5f3`)

### 2a. Every close path ends in the layout's `DeleteNode`

| How the user closes | Path | Where the tree changes |
|---|---|---|
| `exit` in a terminal | srv close-on-exit (`blockcontroller/shell/lifecycle.rs:1599-1646` → `bootstrap/delivery.rs:243-263` → `sagas/delete_block.rs`) queues a frontend `delete` action (`layout_helpers.rs:253`, `focused: false`) | `layoutPersistence.ts` `processPendingBackendActions` → `DeleteNode` case (:239) → `treeReducer(DeleteNode, false)`, then one batch commit (:207-210) |
| `/quit`, `/exit`, or the agent's own `QuitSelf` tool | `quit.ts:63-85` → `ObjectService.QuitAgent` → `sagas/self_quit.rs` → `delete_block` | same backend-action path as above |
| × button, header or body menu "Close Pane", Cmd+W, on a terminal, editor or browser pane | `nodeModel.onClose` / `genericClose` → `closeNode` (`layoutMagnify.ts:110-163`) | `treeReducer(DeleteNode)` synchronously (:156), with `setState` true |
| The same on an **agent** pane | `closeNode` → `beforeNodeDelete` returns false (`pane-close-guard.ts:58`) → `closeWithShutdownLog` → `ClosePane(ids, true)` (`close_pane.rs:200-206`) | backend-action path, after srv teardown (asynchronous) |
| MCP `ClosePane` tool | `close_pane.rs` | backend-action path |
| × on one tab of a multi-tab pane | `closeBlockInStack` → `removeMemberFromStack` (`stackMembers.ts:25-38`) | no tree change; **already works**, see 2d |

### 2b. The next pane is chosen well

`deleteNode` clears `focusedNodeId` when the deleted node was the focused one (`layoutTree.ts:420-422`). Then `validateFocusedNode` (`layoutFocus.ts:15-47`) picks the **most recently focused surviving pane** from `focusedNodeIdStack` (pushed on every `FocusNode`, `layoutModel.ts:700-701`), or the top-left leaf if the stack is empty. This spec keeps that rule.

### 2c. Nothing gives the chosen pane focus

- `treeReducer` requests focus (`focusManager.requestNodeFocus()`, which since #3519 delegates to `refocusNode()` → `giveBlockFocus()` → the view's `giveFocus()`) only for `InsertNode`, `InsertNodeAtIndex` (when `focused`), `FocusNode` and `MagnifyNodeToggle` (`layoutModel.ts:618-670`). **The `DeleteNode` case (:630-634) never does.**
- The request also only fires when `setState` is true (:711-713). The backend-action path runs every action with `setState=false` and commits once at the end of the batch, so even a `DeleteNode` that did request focus would be dropped there.
- The surviving pane does not remount: `balanceNode` keeps the survivor's node id when a container collapses (`layoutNode.ts:245-248`), and leaves are keyed by node id. So its `claimFocusOnMount` (#3519's mount-time claim) never runs again.
- No effect on `focusedNode()` calls `giveFocus()`; no window-focus listener refocuses.

### 2d. The one case that works: closing one tab of a multi-tab pane

`removeMemberFromStack` makes the right-hand tab active, and the tab-visibility effect (`block.tsx:502-513`) calls `giveFocus()` on the newly visible tab. Keep this. (`SPEC_AGENT_PANE_HOVER_CLOSE_FOCUS_REFINEMENTS_2026_09_23.md` around line 452 says this case doesn't move focus; that is out of date.)

### 2e. Closing a magnified pane focuses the pane being closed

`closeNode` un-magnifies first with `magnifyNodeToggle(model, nodeId)` (`layoutMagnify.ts:148-150`), which defaults to `focused=true` and requests focus for the node that is about to be deleted.

### 2f. Native focus (Windows)

- Closing a DOM pane (terminal, agent, editor): the main window's render widget keeps Win32 focus. Only DOM focus is lost, so a DOM `focus()` call is enough.
- Closing a browser pane: the host posts `MainFocusReclaimTask` (`browser_panes/close.rs:94`, :212 → `reclaim_focus_after_pane_destroy`), which `SetFocus`es the main render widget. It restores Win32 focus only, and it runs after the pane is destroyed, so it could undo a focus hand-off to a *surviving browser pane* made before it ran (§3, point 5).

## 3. Design

One rule, applied wherever the tree commits:

> **If a commit removed the active tab's focused pane, request focus for the pane that is now focused.**

1. **Detect "the focused pane was removed", not "a delete happened".** Capture `focusedNodeId` before the reducer runs. After the commit, if that id is no longer in the tree, call `focusManager.requestNodeFocus()`. Doing it at commit time, not in the `DeleteNode` case, covers both paths with one check:
   - the synchronous `closeNode` path (`treeReducer` with `setState` true): check at the existing post-commit point (:711);
   - the backend-action path: check after the batch commit in `processPendingBackendActions` (:207-210), comparing against the focused id captured before the batch.
   A close of a pane that was *not* focused (background pane, a shell exiting in a pane the user isn't in) leaves `focusedNodeId` unchanged and requests nothing.
2. **Reuse the selection contract unchanged.** `requestNodeFocus()` → `refocusNode()` → `giveBlockFocus()` → `viewModel.giveFocus()`, with the dummy-input fallback. Every guard #3519 added applies as is: modal open, search panel open, caret already in the block.
3. **Only for the active tab.** Each tab has its own layout model. A pane closing in a background tab (a background terminal's shell exits, an agent in another tab quits itself) must not move the caret in the tab the user is looking at. Gate on "this layout model belongs to the active tab"; `claimFocusOnMount` already has this check and is the precedent.
4. **Un-magnify without focusing the closing pane.** In `closeNode`, call `magnifyNodeToggle(model, nodeId, true, false)` (signature `(model, nodeId, setState = true, focused = true)`), so the request in point 1 is the only one and it targets the survivor.
5. **Browser survivor (Windows).** `giveFocus()` on a browser pane sends `browser_pane_focus`, which focuses the page natively. When the pane that closed was itself a browser pane, the host's post-destroy `MainFocusReclaimTask` must not then pull focus back to the main render widget. Make the reclaim skip when the same root's `LAST_FOCUSED_BY_ROOT` entry is a live browser pane recorded after the close began (an intentional focus, per `record_intentional_focus`), or have the frontend re-assert its hand-off after the close completes. The first is simpler; see Q2.
6. **A confirm modal in between.** When closing asks for confirmation ("processes are still running"), the modal's focus-restore targets its opener, which is gone after the close (`element/modal.tsx:322-326`, `isConnected` check). When the opener is disconnected, fall back to `focusManager.refocusNode()` instead of leaving focus on `<body>`. `refocusNode` skips while a modal is open, so the order works out: the close commits, the modal unmounts, focus goes to the survivor.
7. **Never raise or activate the window.** If AgentMux isn't the foreground app when the close happens (a shell exiting while the user is in another app), the DOM `focus()` still runs. That sets the caret for when the user comes back, without stealing the OS foreground. Do not call any window-activation API from this path.

## 4. Scope

In: every close route in §2a, all pane types with a `giveFocus()` (terminal, agent composer, editor, browser page, files list, launcher input). Windows, macOS and Linux for the DOM part; point 5 is Windows-only because the reclaim is.

## 5. Constraints: when NOT to take focus

The five constraints of `SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md` §5 carry over. Specific to close:

1. Only when the closed pane was the focused pane of the active tab (§3, points 1 and 3).
2. Not while the user's caret is in an editable element outside every pane (tab rename, command palette, a settings field). The close didn't take their caret, so don't move it. `giveBlockFocus` already returns early when the caret is in the block; add the same check for "caret in an input outside any block".
3. Not while a modal is open. `refocusNode` already skips; point 6 covers the moment it closes.
4. Restoring a saved layout is not a close; it never runs `DeleteNode` on a focused pane, so no change is needed.

## 6. Acceptance

| # | Do this | Expect |
|---|---|---|
| 1 | Two panes, terminal A focused. Type `exit` in A. | B is selected and its input has the caret. Typing goes into B with no click. Repeat with B a terminal, an agent pane, an editor, and a browser pane. |
| 2 | Agent pane focused. Type `/quit`. | Same as 1. |
| 3 | Agent pane focused; the agent calls its `QuitSelf` tool. | Same as 1. |
| 4 | Focused pane; press Cmd+W, or click ×, or use "Close Pane" from either menu. | Same as 1, for every pane type being closed, including agent panes (the asynchronous path). |
| 5 | Three panes; focus C, then B, then A. Close A. | B gets the caret (most recently focused), not the top-left pane. |
| 6 | Pane A focused; a shell exits in background pane B. | Nothing moves. A keeps the caret. |
| 7 | Tab 1 active. A pane in tab 2 closes (shell exit, `QuitSelf`). | Nothing moves in tab 1. Switching to tab 2 later focuses its newly selected pane, as #3519 already does. |
| 8 | Magnified pane focused; close it. | The survivor gets the caret. Nothing focuses the closing pane first. |
| 9 | Close a pane whose processes are still running; confirm. | After the modal closes, the survivor has the caret. |
| 10 | Close a browser pane when the survivor is a browser pane (Windows). | The survivor's page has keyboard focus and stays focused after the host's reclaim. |
| 11 | Type `exit` in the focused terminal, then switch to another app before it closes. | AgentMux does not come to the front. On returning, the survivor has the caret. |
| 12 | Last pane in the tab closes. | No error. Nothing to focus. |

## 7. Tests

- `frontend/layout/tests/layoutModel.test.ts`: `DeleteNode` of the focused node requests focus for the MRU survivor after commit; `DeleteNode` of an unfocused node requests nothing; a batch from `processPendingBackendActions` that removes the focused node requests focus once, after the batch commit.
- Active-tab gating: a background tab's layout model removing its focused node requests nothing.
- `layoutMagnify`: closing the magnified node does not request focus for it.
- `element/modal`: when the opener is disconnected, focus restore calls `refocusNode`.
- `tools/tests/pane-focus-smoke.ps1`: add a close scenario (`exit` in a terminal next to an agent pane, then send keystrokes and assert they reach the composer).

## 8. Files likely to change

- `frontend/layout/lib/layoutModel.ts` (`treeReducer` post-commit check)
- `frontend/layout/lib/layoutPersistence.ts` (`processPendingBackendActions` post-batch check)
- `frontend/layout/lib/layoutMagnify.ts` (`closeNode` un-magnify without focus)
- `frontend/app/store/focusManager.ts` (active-tab gate; "caret outside any block" guard)
- `frontend/app/element/modal.tsx` (restore fallback)
- `crates/cef/src/browser_panes/close.rs` / `ui_tasks/window.rs` (`MainFocusReclaimTask` skips an intentional browser-pane focus)

## 9. Open questions

- **Q1. Views whose `giveFocus()` returns false.** The agent pane's picker and history views have no composer; an identity pane has no `giveFocus`. They fall back to the dummy input, as selection does today. Should the agent picker focus its search box instead? Recommendation: a follow-up, since it affects selection equally.
- **Q2. Browser survivor ordering (§3 point 5).** Skip the reclaim when an intentional pane focus was recorded after the close began, or have the frontend re-assert after the close completes? Recommendation: the skip; it keeps one owner of the decision.
- **Q3. Terminal not in `term` mode** (e.g. showing an exit banner with close-on-exit off): `giveFocus` returns false today. Probably correct, since there's nothing to type into.

## 10. Non-goals

- Changing which pane is chosen after a close (the MRU rule stays).
- Focus after closing a whole tab or window.
- The pool-refill focus steal (#4306).
