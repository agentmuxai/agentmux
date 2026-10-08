# Focus follows the selection: after any close, you can type right away

**Status:** active — R1, R2, R4 and R5's `closeNode` fix built in #4479 (§8). R3 is dropped: the owner wants agent-tab
closing unchanged apart from focus (§9). R5's Windows browser reclaim is not built yet.
**Date:** 2026-10-08.
**Requested by:** repo owner (asafebgi): "if I type exit or close a pane tab, or entire pane, and the next pane that is
selected/border highlight, i should be able to type right away. this is for anything with an input, include terminal,
agent pane, editor, and anything else."
**Author:** AgentO.
**Supersedes:** `SPEC_PANE_CLOSE_FOCUS_HANDOFF_2026_10_03.md` (#4299, proposed, never built). That spec covered closing a
whole pane and said closing one pane tab already worked; on current `main` it does not for most cases (§2b), and it did
not cover panes without a `giveFocus` or the agent shell drawer.
**Builds on:** `SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md` (#3519: selecting a pane focuses its input),
`SPEC_PANE_CLICK_THROUGH_INPUT_FOCUS_2026_09_23.md` (never take a caret the user placed).

Line numbers are against `main` at `bf00c9103`, verified by reading the code. Rows marked *inference* need a live check.

---

## 1. The rule

> **Whenever the highlighted pane, its active pane tab, or the view inside it changes for any reason other than the
> user placing the caret somewhere, the keyboard caret follows it.** It lands in that pane's input (terminal prompt,
> agent composer, editor text, browser page, search box, …); a pane with no input gets its focusable root, so its
> shortcuts work. Never `<body>`.

Checked the way the user notices it: after the change, typed keys reach the highlighted pane with no click.

## 2. What happens today

### 2a. Closing a whole pane: the caret is lost

Every whole-pane close ends in the layout's `DeleteNode`, either synchronously (`closeNode`, `layoutMagnify.ts:156`:
×, Cmd+W `keymodel-nav.ts:33-41`, "Close Pane", the last tab's ×) or through srv's queued delete applied in
`layoutPersistence.ts:239-272` (`exit` with close-on-exit `lifecycle.rs:1680` → `delivery.rs:260-280`; `/quit`
`quit.ts:77`; `QuitSelf`; MCP `ClosePane`; every agent pane, via `close-with-log.ts:23` → `close_pane.rs:199-207`; a
pane's last tab via `removeLeafEmptiedByMove`, `layoutMagnify.ts:53-63`).

- The survivor is chosen well: `deleteNode` clears `focusedNodeId` (`layoutTree.ts:420-421`) and `validateFocusedNode`
  picks the most recently focused pane (`layoutFocus.ts:15-47`). Keep that.
- Nothing focuses it. `treeReducer` requests focus only for insert, `FocusNode` and magnify (`layoutModel.ts:618-670`),
  only when `setState` is true (`:711-713`); the backend batch commits once with `setState` false
  (`layoutPersistence.ts:207-210`). The survivor keeps its node id (`layoutNode.ts:244-248`), so it doesn't remount and
  its mount-time claim doesn't run again.
- Closing a magnified pane first re-focuses the pane being closed (`layoutMagnify.ts:148-150`, `focused` defaults to
  true at `:27`).

### 2b. Closing one pane tab: lost in most cases

Path: × (`PaneTabStrip.tsx:826-836`) → `PaneChrome.tsx:339-342` → `closeBlockInStack` (`layoutStack.ts:190-241`). The
only focus trigger is the tab-visibility effect (`block.tsx:502-513`), which calls the newly active tab's
`giveFocus()` when it becomes visible.

| Closing / landing on | Result |
|---|---|
| An **agent** tab | Lost until srv finishes tearing the agent down. `beforeNodeDelete` returns false for agents (`tabcontent.tsx:147-149`), the tab stays up under the shutdown overlay, and the neighbour only becomes active when srv's delete arrives (`layoutPersistence.ts:264-265`). |
| The **last tab of an agent pane** | The pane stays and shows a My Agents picker tab (`close-agent-tab.ts:58-80`). The picker has no composer, `giveFocus` returns false (`agent-model.ts:981-985`), and `block.tsx:511` has no dummy-input fallback: `<body>`. |
| A neighbour that **mounts fresh** (launcher, help, sysinfo, Swarm, settings, memory, …) | The effect fires as soon as the view model exists (`block.tsx:446-452`), before the view renders: the launcher's input ref is still null, the others have no `giveFocus`. No fallback: `<body>`. |
| A neighbour that is an agent tab showing the picker or history | Same as the last-tab case. |
| A neighbour that is a terminal, editor, browser, files list or agent with a composer (kept mounted) | Focused by the visibility effect. *Inference: needs a live check.* |

### 2c. Other ways the selected thing changes and the caret is lost

- **`exit` in an agent pane's shell drawer.** The drawer collapses (`shell-exit-collapse.ts:58-60`), `<Show>` unmounts
  the focused xterm (`AgentShellDrawer.tsx:43`), and nothing refocuses the composer. No pane closes.
- **Closing a window tab.** srv promotes a neighbour (`tabbar.tsx:163-208`); no focus call is made. *Inference:* the
  visibility effect may focus the promoted tab's pane if it was already laid out, not if it is still held hidden behind
  the reveal gate (`workspace.tsx:407`).
- **A confirm modal in the way** ("processes are still running"). The modal restores focus to its opener, which is gone
  after the close (`modal.tsx:323-324`).

### 2d. Which panes can take the caret

`giveFocus` is wired only when a pane type supplies `focus` (`pane-tab-host.tsx:73`).

| Pane | Today | Target |
|---|---|---|
| Terminal | xterm (`termViewModel.ts:376-389`); false outside `term` mode | unchanged |
| Agent, composer view | composer (`agent-model.ts:981-985`) | unchanged |
| Agent, My Agents picker | none (returns false) | the picker's filter box (`AgentPickerFilterBar.tsx:55`) |
| Agent, history view | none | the history view's search/filter field if present, else the pane root |
| Editor | CodeMirror (`editor-model.ts:1430-1435`) | unchanged |
| Browser | native page focus (`browser-model.ts:747-761`) | unchanged |
| Files | file list (`files.tsx:45-48`) | unchanged |
| Launcher | search input (`launcher.tsx:100-106`), no mount claim | add the mount claim |
| Settings | none | search box (`settings-search-bar.tsx`) |
| Remotes | none | its filter input (`remotes-view.tsx`) |
| Swarm, Toolchain, Drone | none | the pane root. Their search fields sit behind a toolbar toggle or only exist in some modes, so there is no input that is always there to type into. |
| Identity | returns false, not wired (`identity-model.ts:729`) | the pane root: the account form is a secondary screen, not the pane's landing state |
| Help, Sysinfo, Media, Memory, Connectors, Warden, Armory | none | the pane root (dummy input), so shortcuts and arrow keys work |

## 3. Design

### R1. One reconciler, called wherever the selection can change

Add `focusManager.ensureSelectionFocused(reason)`. It is the only thing that moves the caret on these events, and it is
idempotent:

1. If the user's caret is in an editable element outside every pane (tab rename, command palette, a settings field), a
   modal is open, or the caret is already inside the selected pane: do nothing.
2. Otherwise `giveBlockFocus(selected pane's active tab)`: the view's `giveFocus()`, else the block's dummy input. Never
   `<body>`.
3. If the view isn't ready yet (`giveFocus()` false *and* the view registered that it will claim on mount), leave it to
   the mount claim (`claimFocusOnMount`), which already checks "still the active tab's selected pane".
4. Only for the active window tab, and only with DOM focus: never raise or activate the window (an `exit` while the user
   is in another app sets the caret for when they come back).

Called from:

| Event | Where |
|---|---|
| A layout commit removed the focused pane (whole-pane close, any route) | `treeReducer`'s post-commit point (`layoutModel.ts:711`) and after the backend batch commit (`layoutPersistence.ts:207-210`), when the `focusedNodeId` captured before the change is no longer in the tree |
| A pane's active tab changed while that pane is selected | `closeBlockInStack` / `removeMemberFromStack` / `setActiveMember`, replacing the direct `vm.giveFocus()` in `block.tsx:511` (which gains the guards and the dummy fallback this way) |
| A block's view changed while selected (agent composer ↔ picker ↔ history; shell drawer collapsed) | the agent view's mode switch and `shell-exit-collapse.ts` |
| A window tab became active after a close promotion | `workspace.tsx`'s reveal, once the tab is visible (not while held hidden) |
| A modal closed and its opener is gone | `modal.tsx`'s restore fallback |

### R2. A safety net for paths nobody wired

A document-level `focusout` listener: when focus leaves an element inside a pane and, on the next frame,
`document.activeElement` is `<body>` and `document.hasFocus()` is true, call `ensureSelectionFocused("orphan")`. Skip it
if the user pressed a pointer outside every pane in the last 300 ms (they clicked somewhere on purpose). This catches the
next close path someone adds without wiring R1, and logs `[focus] orphan → <blockId> (<reason>)` so the gap is visible
in the log and can be wired properly.

Chromium fires `focusout` when the focused element is removed from the document, which is exactly the close case. It is
not relied on alone: R1's explicit calls are the primary mechanism; R2 only covers omissions.

### R3. Agent tabs close in the UI at once (dropped, §9)

Closing an agent tab activates the neighbour immediately (the closing tab is taken out of the strip and its content
hidden, with the shutdown overlay shown on the tab pill or in the status line instead), so R1 hands the caret over at
once. srv's teardown continues; when its delete arrives the tab is already gone from the strip, and
`layoutPersistence`'s delete is a no-op for the active member. The last agent tab of a pane swaps to the picker
immediately, as today, and the picker now takes the caret (§2d).

### R4. Every pane type with an input exposes it

Add `focus` for the types in §2d's Target column. The launcher also claims focus on mount. A pane type without an input
needs nothing: R1 falls back to its dummy input.

### R5. Smaller fixes

- `closeNode` un-magnifies with `magnifyNodeToggle(model, nodeId, true, false)`, so nothing focuses the pane being
  closed first.
- Windows, closing a browser pane when the survivor is a browser pane: the host's post-destroy `MainFocusReclaimTask`
  (`browser_panes/close.rs:94, :212`) must not pull native focus back from a browser pane focused after the close began
  (skip when the root's last intentional focus is a live browser pane). From the 10-03 spec, unchanged.

## 4. Acceptance

Each row: do the action with the caret in the closing pane, then type without clicking; the keys must reach the
highlighted pane. Run with each landing type: terminal, agent (composer), agent (picker), editor, browser, launcher,
settings, Swarm.

| # | Action | Expect |
|---|---|---|
| 1 | `exit` in a terminal (single tab) | Survivor (most recently focused pane) has the caret |
| 2 | `exit` in a terminal tab of a multi-tab pane | The pane's next tab has the caret |
| 3 | × on the active pane tab, every pane type | The next tab has the caret, immediately, including when an agent tab closes |
| 4 | × on the last tab of an agent pane | The My Agents picker's filter box has the caret |
| 5 | Cmd+W, the pane's ×, "Close Pane" from either menu | Survivor has the caret |
| 6 | `/quit`; the agent calls `QuitSelf`; MCP `ClosePane` on the focused pane | Survivor has the caret |
| 7 | `exit` in an agent pane's shell drawer | The composer has the caret |
| 8 | Close a magnified pane | Survivor has the caret; nothing focuses the closing pane first |
| 9 | Close a pane with running processes; confirm | After the modal, the survivor has the caret |
| 10 | Close a window tab | The promoted tab's selected pane has the caret once it is shown |
| 11 | A background pane or a pane in another window tab closes | Nothing moves; the caret stays where it was |
| 12 | Caret in the tab-rename field or the command palette while a pane closes | It stays there |
| 13 | `exit`, then switch to another app before the pane closes | AgentMux does not come to the front; on return, the survivor has the caret |
| 14 | Last pane in the window tab closes | No error |
| 15 | Windows: close a browser pane next to a browser pane | The survivor's page keeps keyboard focus after the host's reclaim |

## 5. Tests

- **Unit:** `ensureSelectionFocused` guard table (outside editable, modal, caret already in pane, background tab, view
  not ready → mount claim); the post-commit check fires once when the focused node is removed (sync and backend batch)
  and never for an unfocused one; `closeBlockInStack` asks for focus for the new active member when the pane is
  selected; agent tab close activates the neighbour without waiting for srv; `closeNode` doesn't focus the magnified
  pane; `modal` falls back when the opener is disconnected; the R2 net ignores a deliberate pointer press.
- **Live (CDP):** `scripts/ui-screenshots/focus-after-close.mjs` against a dev build: builds each acceptance row's
  layout, performs the action through the real UI (keys and clicks), then types a marker string with
  `Input.insertText` and asserts it arrived in the highlighted pane (terminal buffer, composer value, editor document,
  search box value). Runs all rows × landing types; any row where the marker lands nowhere fails.

## 6. Rollout

One PR for R1, R2, R4 and R5 (they are small and only useful together), with the CDP script. R3 (agent tabs leave the
strip at once) in a second PR, because it changes when an agent tab disappears and needs its own review of the shutdown
overlay. Both go through the live script before merge.

## 7. Open questions

1. **R3's overlay.** While a closed agent tab shuts down in the background, show its progress on the neighbouring tab
   pill, in the pane's status line, or nowhere? Recommended: a small "closing Claude #2…" note in the status line until
   srv confirms, so a slow shutdown is still visible.
2. **R2's logging.** Keep the `[focus] orphan` line in production builds? Recommended: yes, rate-limited; it is how the
   next unwired path gets found.
3. **Terminal not in `term` mode** (exit banner shown with close-on-exit off): today `giveFocus` returns false. With R1
   the dummy input gets the caret, so pane shortcuts work. Recommended: keep it that way; there is nothing to type into.

## 8. As built (#4479)

The design above called `ensureSelectionFocused` from each place the selection can change. Building it showed one
observer covers all of them, so that is what was built:

- **R1, one observer per layout model.** `LayoutModel` watches a memo of the selected node's id and block id
  (`layoutModel.ts`, next to `focusedNode`). When it changes for any reason (a pane deleted by `closeNode` or by srv's
  backend batch, a pane tab closed or switched, a view swapped inside the pane), it calls
  `focusManager.ensureSelectionFocused("selection", model)` on the next frame, after the DOM has updated. A background
  tab's model is ignored. No close path calls it by hand, so a new close path is covered without wiring.
- **R1, waiting for a view that is still mounting.** Instead of each view registering a mount claim, `giveBlockFocus`
  retries every frame for up to `FOCUS_RETRY_FRAMES` (60, about a second) while the block is still the selection, the
  caret is still parked (on `<body>` or the block's dummy input) and no modal is open. This covers the launcher, the
  picker and any pane that mounts fresh.
- **R1, window tabs.** `installFocusFollowsSelection()` (called from `app-init.ts`) watches `atoms.activeTabId` and
  asks on the next frame; the retry covers a tab still held hidden behind the reveal gate.
- **R1, modal.** `modal.tsx` falls back to `ensureSelectionFocused("modal-closed")` when the opener is gone.
- **R2** in `installFocusFollowsSelection()`, skipped after *any* pointer press in the last 300 ms, not only one outside
  the panes: a press on agent output blurs the composer to `<body>` too, and refocusing it would collapse the selection
  being dragged. It also covers `exit` in an agent's shell drawer: the xterm
  unmounts, focus falls to `<body>`, and the net hands it to the composer.
- **R4, a `data-pane-focus` attribute** instead of a `focus` method per pane type. `giveBlockFocus` tries the view's
  `giveFocus()`, then the first visible, enabled `[data-pane-focus]` inside the block, then the dummy input. Marked:
  the My Agents picker's filter box, Settings' search box, Remotes' filter. The picker's first card still takes focus
  (Enter launches it); a printable key typed on a card goes into the filter box (`AgentCard.tsx`).
- **R5:** `closeNode` un-magnifies without requesting focus.
- **Guards** (all in `ensureSelectionFocused` / `giveBlockFocus`): the window not having focus (a browser pane's
  `giveFocus()` moves native focus and could raise the window; when the window regains focus with the caret on
  `<body>`, the reconcile runs then, which is acceptance row 13); a modal over the pane, whether opened through
  `modalsModel` or a declarative `<Modal>` on the modal stack (`modalCovers`, `modal-stack.ts`; a pane modal in another
  pane doesn't block); a caret in an editable element outside
  every pane (`caretInEditableOutsidePanes`, `focusutil.ts`); a caret the user put in an input inside the selected pane
  (`userCaretInBlock`, unchanged).

### Consolidated (DRY)

| Before | After |
|---|---|
| `block.tsx`'s tab-visibility effect called the view's `giveFocus()` directly, with no caret guard, no fallback and no retry | it calls `giveBlockFocus`, the routine every other "focus this pane" path already used |
| Each close path would have needed its own focus call (the 10-03 spec listed six) | one selection observer per layout model |
| A focus method per pane type with an input | one `data-pane-focus` attribute, found by `giveBlockFocus` |
| Each late-mounting view claims focus on mount | one retry in `giveBlockFocus` |
| `userCaretInBlock` and the new outside-panes check each had their own "is this a text entry" test | one `isTextEntry` in `focusutil.ts` |

### Not built yet

- **R3**: dropped (§9). Closing an agent tab hands the caret over when srv's delete arrives (the observer sees the
  selection change then), not immediately.
- **R5, Windows browser reclaim.**
- **The CDP script** in §5 is not checked in yet. For this PR, a scratch version drove rows 1, 2, 3 and 5 on a dev
  build, landing on a terminal, an agent composer, the launcher and Settings: on `main` five of the six cases left the
  caret on `<body>`; with this change all six land in the highlighted pane's input.

## 9. Later: possible improvements, not planned

Found while building this; recorded so they can be picked up separately. None changes behaviour in #4479.

1. **Agent tabs hand over the caret late.** Closing an agent pane tab keeps the tab, under its shutdown overlay, until
   srv finishes the teardown (`SHUTDOWN_GRACE` 5 s, up to about 9 s in the worst case). Only then does the neighbour
   become active and take the caret. `beforeNodeDelete` returns false for agents (`tabcontent.tsx:147-150`), so
   `closeBlockInStack` never removes the member itself. R3 would remove the tab at once. The owner chose to keep today's
   behaviour (2026-10-08). If it is revisited:
   - srv's later `delete` for a block no longer in the stack is a safe no-op (`layoutPersistence.ts:264-270`), but it
     logs a `console.error`. Close with `ClosePane([id])` without `frontendWaits`, as terminal tabs do, to avoid it.
   - The in-pane shutdown overlay (progress, "Couldn't shut down", Try again / Keep open) unmounts with the tab, so
     progress and failure need another home.
   - A failed shutdown would leave an agent block in no pane until the orphan reaper runs (2 min); a failure before
     the teardown starts would leave a *live* agent. Re-adding the tab on failure avoids both.
2. **The "last agent tab returns to My Agents" fallback is dead code.** #3780 made closing the last agent tab of a pane
   show the picker. #3784's shutdown log made `beforeNodeDelete` start the shutdown and return false, so
   `closeAgentTab` returns at `close-agent-tab.ts:63` and srv closes the whole pane. The owner chose that the pane
   closes (2026-10-08). Remove the picker fallback (`close-agent-tab.ts:67-83`); `close-agent-tab.test.ts` mocks
   `beforeNodeDelete` as true, so it tests a path that no longer runs for agents.
3. **A second × on a closing agent tab** probably starts a second probe and a second `ClosePane`: the in-flight guard in
   `close-agent-tab.ts:46-52` clears as soon as `beforeNodeDelete` returns, while the tab is still visible. Not
   verified against srv.
4. **No test of `DeleteNode` for a block id no longer in the tree** (`layoutPersistence.test.ts`).
5. **`block.test.tsx`'s first test** takes about 1.3 s alone (most of it the cold import of `block.tsx`) and can pass
   the 5 s timeout when the whole suite runs in parallel on a loaded machine. Warm the import in a `beforeAll`.
6. **The CDP script** (§5) and **R5's Windows browser reclaim**, from §8.
