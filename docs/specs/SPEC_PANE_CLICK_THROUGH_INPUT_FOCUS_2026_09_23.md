# Clicking an input in an unselected pane should focus that input — in one click

**Status:** implemented on branch `agent1/pane-click-through-input-focus`
(Agent1). Spec authored by Agent2. See §8 for where the implementation departs
from or extends §4.
**Date:** 2026-09-23.
**Severity:** Medium — nothing is lost, but every first interaction with a
secondary control in an unselected pane takes two clicks: one that is
silently thrown away, and one that works. On the starter "My Agents" picker
that is the first thing a user types into.
**Requested by:** repo owner (asafebgi).
**Regression of:** `SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md` (PR #3519,
`b3f84dbe6`). That spec's own §5 constraint 1 is "Never steal focus from a
pane the user is already typing in" — the implementation honors it for a
pane that was *already* selected, but not for the click that *selects* it.
**Related:** `SPEC_PANE_OPEN_FOCUS_ROUTING_2026_09_16.md` (the shared
`giveFocus()` contract).

---

## 1. What we want

In a pane that is not the selected pane, a single click on any text-entry
control (`<input>`, `<textarea>`, `<select>`, `contenteditable`) must both
select the pane **and** leave the caret in the control that was clicked.

The reported case: the starter agent pane's picker ("My Agents" page).
With another pane selected, click the **Filter agents...** box. Today the pane
becomes selected but the box does not get the caret; a second click is needed.
When the agent pane has a live session, clicking its chat input works in one
click — see §2c for why that is a coincidence, not a working path.

## 2. Current state, confirmed against `main` (`af3123b23`)

### 2a. The causal chain

One mousedown on `Filter agents...` in an unselected pane:

1. The browser moves DOM focus to the `<input>`
   (`frontend/app/view/agent/components/AgentPickerFilterBar.tsx:55-63`).
2. `focusin` bubbles to the block frame
   (`frontend/app/block/blockframe.tsx:1313`) →
   `handleChildFocus` (`frontend/app/block/block.tsx:168-171`) sees
   `!isFocused()` and calls `nodeModel.focusNode()`.
3. That dispatches `LayoutTreeActionType.FocusNode`, which sets
   `shouldRequestFocus = true` (`frontend/layout/lib/layoutModel.ts:646-648`);
   once state commits, `treeReducer` calls `focusManager.requestNodeFocus()`
   (`layoutModel.ts:694-696`). This hook was added by #3519. Before that,
   `requestNodeFocus()` did nothing.
4. `requestNodeFocus()` → `refocusNode()`
   (`frontend/app/store/focusManager.ts:45-60`) calls the view's
   `giveFocus()`. The agent view's `giveFocus()`
   (`frontend/app/view/agent/agent-model.ts:934-939`) returns `false` when the
   footer textarea isn't mounted, and it isn't mounted on the picker page.
   `refocusNode()` then falls back to
   `document.getElementById(`${blockId}-dummy-focus`).focus()`.
5. **The caret moves from the Filter box to the invisible dummy input.** The
   pane is now selected, so the second click has no selection side effects
   and simply focuses the input.

`handleBlockClick` (`block.tsx:223-232`) then runs on `click`. By that point
`focusedBlockId()` already resolves to this block (the dummy lives inside
it), so it does not move focus again. It is not the culprit, but see §4.3.

### 2b. Why `refocusNode()` does not notice

`refocusNode()` (and its twin, `refocusNode(blockId)` in
`frontend/app/store/block-component-registry.ts:169-184`) never looks at
`document.activeElement`. "The user just put the caret somewhere in this
pane" and "this pane was selected by keyboard / tab switch / creation" look
the same to it.

The browser pane is the one view that already guards against this, inside its
own `giveFocus()` (`frontend/app/view/browser/browser-model.ts:738-759`):
when a non-dummy `INPUT`/`TEXTAREA` in the block is already focused (the URL
bar), it keeps it. That is why the URL bar has always worked in one click. The
guard is in the right spirit, but it lives in a single view instead of the
shared contract.

### 2c. Why "the active agent pane works"

In a live agent pane, the control people click is the chat textarea, and that
textarea is exactly what `giveFocus()` focuses. The caret is taken from the
textarea and handed back to the same textarea, so nothing visibly changes. Any
**other** text control in the same pane gets its caret taken and moved to the
chat textarea (see §3, rows A2-A5).

## 3. Everywhere else this happens

Rule: every text-entry control inside a pane body or pane chrome whose view's
`giveFocus()` focuses **something else**, or returns `false` (which falls back
to the dummy input), loses its first click in an unselected pane. Views
without a `giveFocus()` always fall back to the dummy input.

"Confirmed" means the §2a chain was traced in code for that control. "Same
path" means the control is a plain focusable input rendered inside the block
frame, with a view whose `giveFocus()` target is different, so the chain
necessarily applies. Nothing below has been clicked through in a running
build yet (§6 is the QA pass for that).

| # | View / surface | Control(s) | `giveFocus()` target | Status |
|---|---|---|---|---|
| **P1** | Agent picker (starter "My Agents") | **Filter agents...** (`AgentPickerFilterBar.tsx:55`) | none on picker → dummy | **Reported, confirmed** |
| P2 | Agent picker | Sort `<select>` (`AgentPickerFilterBar.tsx:77`) | dummy | Same path. A native select popup may also close when focus is taken during mousedown; verify in QA |
| P3 | Agent picker | Any input in `MyAgentsList.tsx` | dummy | Same path |
| A1 | Agent pane (live session) | Chat textarea (`AgentFooter.tsx`) | the same textarea | **Works**, coincidentally (§2c) |
| A2 | Agent pane | Transcript search bar, Ctrl+F (`AgentSearchBar.tsx`) | chat textarea | Same path. Usually opened by keyboard in an already-selected pane, but a click after selecting another pane loses the caret |
| A3 | Agent pane | Question / decision answer inputs (`AgentQuestionPanel.tsx`, `AgentDecisionPanel.tsx`) | chat textarea | Same path. **Highest-impact after P1**: a user answering an agent's question from another pane types into the chat box instead |
| A4 | Agent pane | Auth / login panels (`InAppLoginPanel.tsx`, `PreLaunchAuthPanel.tsx`, `AgentIdentityPanel.tsx`) | chat textarea or dummy | Same path |
| A5 | Agent pane | `NativeMemoryHistoryPanel.tsx` input | chat textarea or dummy | Same path |
| E1 | Editor pane | Tab-strip rename input (`editor-tab-strip.tsx`), file-tree input (`file-tree.tsx`) | CodeMirror content | Same path |
| E2 | Editor pane | CodeMirror's own search/replace panel inputs (inside the CM DOM) | `cm.focus()`, the content DOM | Same path. Clicking Find while another pane is selected moves the caret into the document |
| B1 | Browser pane | URL bar (`browser-nav-bar.tsx`) | guarded (§2b) | **Works**, and is the precedent for the fix |
| T1 | Terminal pane | Search box | `giveFocus()` returns `true` early when search is open (`termViewModel.ts:513-516`) | Works for the search box; nothing else to click |
| L1 | Launcher pane | Launcher input (`launcher.tsx`) | the same input | Works, coincidentally |
| M1 | Native Memory manager | **Filter agents...** (`MemoryAgentFilterBar.tsx:59`), `MemoryFileCard.tsx`, `native-memory-manager.tsx` | no `giveFocus()` → dummy | Same path. Same label as P1, so users will expect the same fix |
| S1 | Settings pane | Search bar (`settings-search-bar.tsx`) and every text/select control in `settings-controls.tsx` and `sections/*` | dummy | Same path |
| X1 | Bundle / Global Bundle / MCP / Skill managers | Inputs in `bundle-manager.tsx`, `Bundle*Section.tsx`, `global-bundle-manager.tsx`, `mcp-manager.tsx`, `skill-manager.tsx` | dummy | Same path |
| X2 | Identity / Accounts | `identity-account-form.tsx`, `OAuthConnectPanel.tsx` | `identity-model.ts:589` returns `false` → dummy | Same path |
| X3 | Drone, Swarm, Toolchain, Warden Supervisor | `drone-view.tsx`, `swarm-fleet-toolbar.tsx`, `swarm-view.tsx`, `toolchain-view.tsx`, `warden-supervisor-manager.tsx` | dummy | Same path |
| P4 | Agent picker | **Any input or composer in ANY pane**, taken by the default template card's own `focus()` (`AgentCard.tsx:83-85`) | n/a — bypasses all three guarded paths | **Confirmed.** Runs on mount and on every `agents:changed` refetch, since `<For>` remounted every card. Also scrolls the picker to "New Agent" (no `preventScroll`). See `REPORT_AGENT_PANE_SIDE_BY_SIDE_SCROLL_AND_FOCUS_QUIRKS_2026_09_23.md` §2 |
| C1 | Pane chrome | Click-to-rename view-type label (`blockframe.tsx:219`), pane title rename (`titlebar.tsx:98`), header `Input` decls (`blockutil.tsx:207`) | view's target / dummy | **Likely worse than a lost click.** These inputs commit on `onBlur`, so taking focus right after the rename input mounts could commit and close the rename immediately. Verify in QA |

**Out of scope:** in-pane modals such as `AgentLaunchModal` and
`AgentCreateFromTemplateModal`. If they are registered with `modalsModel`,
`refocusNode()` already stops early (`focusManager.ts:48`). QA should confirm
each one is registered. Any that isn't falls under this spec.

## 4. Proposed fix

### 4.1 One guard in the shared contract

Add a single helper, and have **both** `refocusNode` implementations and
`claimFocusOnMount()` stop early when it returns true:

```ts
// frontend/util/focusutil.ts
/** True when the caret is already in a real text-entry control inside
 *  `blockId` — i.e. the user (or the view) chose where to type, and
 *  pane selection must not override that choice. */
export function userCaretInBlock(blockId: string): boolean {
    const el = document.activeElement as HTMLElement | null;
    if (el == null || el.classList.contains("dummy-focus")) return false;
    if (findBlockId(el) !== blockId) return false;
    return (
        el.tagName === "INPUT" ||
        el.tagName === "TEXTAREA" ||
        el.tagName === "SELECT" ||
        el.isContentEditable
    );
}
```

```ts
// focusManager.refocusNode(), after the modal check and after
// layoutModel.focusNode(lnode.id):
if (userCaretInBlock(blockId)) return;
```

Apply the same check at the same point in `refocusNode(blockId)`
(`block-component-registry.ts:169`) and before `giveFocus()` in
`claimFocusOnMount()` (`focusManager.ts:75`). Otherwise a pane that mounts
late could still take the caret from a control the user clicked while it was
loading.

Why these three call sites and not the individual views:

- One fix covers every row in §3, including views that have no
  `giveFocus()` at all.
- Every pane-selection path goes through one of these three (the #3519
  design), so no future view can reintroduce the bug by forgetting a guard.
- `isContentEditable` covers CodeMirror. Its content DOM is contenteditable,
  so in E2 the guard sees "already in CM" and does nothing, which is correct:
  CM's own panels handle their own focus. Clicking into the editor body, or
  selecting the pane by keyboard, still reaches `cm.focus()` as before.

### 4.2 Keep #3519's behavior everywhere else

The guard only fires when the caret is **already** in a text control **in
the pane being selected**. So:

- Keyboard selection (arrow-nav, Cmd+1..9), tab switch, and pane creation are
  unchanged. At that moment the caret is in another pane or nowhere, so
  `giveFocus()` runs as it does today.
- Clicking non-text areas of an unselected pane (a transcript message, a
  header, an agent card `<button>`) is unchanged. `BUTTON` is deliberately not
  in the list, so clicking a toolbar button in an agent pane still puts the
  caret in the chat box, which is what #3519 intended.

### 4.3 Clean-ups that go with it

- `browser-model.ts:746-754`: the view-local `isMainInput` check becomes
  redundant for the "keep caret" decision. Keep only its `main_window_focus`
  IPC side effect, or drop it if `handleChildFocus` (`block.tsx:190`) already
  covers that path. Remove it in the same PR so there is one guard, not two.
- `handleBlockClick` (`block.tsx:223-232`) should use the same
  `userCaretInBlock()` test instead of the looser `focusedBlockId() == blockId`.
  Today it is saved only because the dummy happens to live inside the block.
  Once the dummy is no longer where the caret lands, the looser check would
  still work, but the two paths should give the same answer for the same
  reason.

## 5. Tests

Vitest, next to the existing focus tests:

1. `refocusNode()` with `document.activeElement` = an `<input>` inside
   `[data-blockid=X]`, with X the focused node: `giveFocus` is **not** called
   and `activeElement` does not change.
2. The same with `<select>`, `<textarea>`, and a `contenteditable` div.
3. `activeElement` = `X-dummy-focus`, or an element in block **Y**:
   `giveFocus` **is** called (no regression of #3519).
4. `activeElement` = a `<button>` inside X: `giveFocus` **is** called (§4.2).
5. `claimFocusOnMount(X, fn)` with the caret in X's filter input: `fn` is not
   called.
6. Component test for `AgentPickerFilterBar` inside a mounted `Block`: with a
   second block focused, one `mousedown`+`focus`+`click` on the filter input
   leaves `activeElement` on the filter input and the block focused.

## 6. Manual QA matrix (`task dev`, Windows first)

With another pane selected each time, click once and type:

- [ ] P1 picker Filter agents: text appears in the filter
- [ ] P2 picker Sort select: popup opens and stays open
- [ ] A2/A3 agent pane search bar and a pending question input: text goes to that control, not the chat box
- [ ] A1 agent chat textarea: still one click (no regression)
- [ ] E1/E2 editor tab rename, file-tree input, Ctrl+F panel
- [ ] M1 Native Memory Filter agents
- [ ] S1 Settings search
- [ ] C1 pane title rename: stays in edit mode after the click
- [ ] B1 browser URL bar: still one click
- [ ] Keyboard pane nav, Ctrl+Tab, and new pane: caret still lands in the pane's main input (#3519 behavior)
- [ ] Any in-pane modal: confirm it is registered with `modalsModel` (§3 out-of-scope note)

## 7. Open questions

1. **Buttons and links.** Should clicking a focusable non-text control (a
   `<button>`) in an unselected agent pane leave focus on the button (better
   for keyboard and screen-reader users) or move the caret to the chat box
   (#3519's intent)? This spec picks the #3519 behavior. Flipping it means
   adding `BUTTON`/`A` to the guard.
2. **Mouse vs. programmatic focus.** The guard does not care *how* the caret
   got into the control. A view that calls `.focus()` on its own input during
   mount, before pane selection runs, will also keep it. That is almost
   certainly desired, but it is a behavior change for any view that relied on
   `giveFocus()` overriding its own autofocus.

## 8. Implementation notes (added by Agent1 with the implementation)

**As specified (§4.1):** `userCaretInBlock()` lives in `frontend/util/focusutil.ts`.
`refocusNode()`, `refocusNode(blockId)` and `claimFocusOnMount()` all consult it.

**DRY beyond §4.1.** The three "focus this pane" bodies were near-copies that
had already drifted: only `block.tsx` had a "focus already within" check, and
only `block.tsx` used `preventScroll`. They now all call one
`giveBlockFocus(blockId)` in `focusManager.ts`, so the guard lives in exactly
one place. The dummy-input fallback always uses `preventScroll: true`.

**Refinement: non-text `<input>` types.** Types that don't take typed text are
treated like `BUTTON` per §4.2: `checkbox`, `radio`, `button`, `submit`,
`reset`, `range`, `color`, `file`, `image` and `hidden`. They do not count as
"the user chose where to type".

**Departure from §4.3, second bullet.** `handleBlockClick` keeps its looser
`focusedBlockId() == blockId` test. Switching it to `userCaretInBlock()` would
refocus the composer on every click inside an *already-selected* pane that
isn't on a text control. Focusing the textarea collapses any transcript text
selection the user just made, which would break click-drag-copy. The
reducer-driven path, the one that caused this bug, gets the strict guard
through `giveBlockFocus()`.

**Not done: §4.3, first bullet** (browser-model's `isMainInput`). Its
`main_window_focus` IPC side effect is still needed when `giveFocus()` is
reached through `claimFocusOnMount()`. It is now redundant for the "keep the
caret" decision on the reducer path, but harmless there. Left for a
browser-pane follow-up rather than widening this PR.

**P4 (AgentCard) fix:**
- The card focuses with `{ preventScroll: true }`.
- It goes through `focusManager.claimFocusOnMount(blockId, …)`, so it gets the
  selected-pane check and the §4.1 guard.
- `useAgentDefinitions()` now keeps the previous object for any definition
  whose content didn't change (`reuseUnchangedById`), so an `agents:changed`
  refetch no longer remounts cards.

**Same-PR companion: pane-scoped document listeners.** Of the same
"two panes side by side" class, but about keyboard, not caret, ownership. A
shared `eventBelongsToPaneOf(e, el)` in `focusutil.ts` replaces two inline
`.closest(".agent-view")` checks (AgentDecisionPanel, AgentQuestionPanel). It
is also added to four listeners that had no check:
- `SlashCommandPicker`: arrows, letters and Enter typed in pane B used to drive
  pane A's open picker.
- `SlashHelpPanel` and `BtwOverlay`: Escape in pane B used to close pane A's
  panel. `BtwOverlay`'s outside-click close is now scoped to its own pane, so
  clicking into the neighbouring pane leaves it open.
- `MyAgentsList`: Escape in any pane used to close every pane's row menu.

An event whose target is outside every pane (e.g. `body`) belongs to the
*selected* pane only.
