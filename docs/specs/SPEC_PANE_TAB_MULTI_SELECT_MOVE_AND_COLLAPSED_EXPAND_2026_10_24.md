# Spec: Moving pane tabs between panes — a faster, longer drop flash; Ctrl/Cmd+click to select several tabs and drag them together; selecting a tab on a collapsed pane expands it

**Status:** active — approved by the operator on 2026-10-24 (through AgentY): "ok, thats fine, follow wcag, as fast as possible following the standard, lets proceed with recommendations". D1 is settled in §3 and §9; D2 to D9 take the recommendations. Build: #2242.
**Date:** 2026-10-24
**Author:** Camper@narko
**Task:** Spec #2241; the build is #2242 and starts after this is approved
**Requested by:** the operator, through AgentY, with two additions from AgentX (Cmd on macOS and Ctrl elsewhere; multi-select stays out of page content and clear of existing Ctrl+click bindings)
**Verified against:** `agentmux` `main` @ `9e0c15807` (2026-10-24), by reading the code named below. Nothing here has been run.
**Builds on:** [SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md](SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md) (the single-tab drag this extends), [SPEC_DROP_TARGET_LANDS_INVERTED_2026_10_21.md](SPEC_DROP_TARGET_LANDS_INVERTED_2026_10_21.md) (the flash this speeds up), [SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md](SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md) (landing bounce, and a pane emptied by a move closes), [SPEC_DRAG_MINIMIZED_PANES_2026_10_20.md](SPEC_DRAG_MINIMIZED_PANES_2026_10_20.md)

## 1. Ask

1. The flash when a tab is dragged over another pane's tab bar: **twice as fast and twice as long.**
2. **Ctrl+click selects several pane tabs, even in different panes.** Dragging any selected tab moves all of them into one target pane.
3. **Selecting a tab on a collapsed pane expands that pane.**

"Pane tab" is the pill in a pane's header strip (one `block_stack` member), as in the 2026-09-19 spec. The Window Tab bar and the document tabs of the Editor and Media panes are not in scope.

## 2. What happens today

### 2.1 The flash

Dragging a pill over another pane marks that pane's header row `.pane-header--foreign-hover` (or `.pane-tab-strip--foreign-hover` when the strip is its own drop zone). Both use `drop-target-hover($hold: true)` from `frontend/app/drop-feedback.scss`:

- **The blink.** `tab-drop-invert-hold 400ms steps(1, end) forwards`: the row is negative for 200 ms, normal for 200 ms, then negative and held until the hover ends. That is **one** blink (one flash) in the first 400 ms.
- **The pulse.** `tab-drop-pulse 0.8s … infinite alternate`: the 2 px outline fades between two colours for as long as the hover lasts.
- **The counter-invert.** The pill strip inside the row runs the same blink (`drop-target-strobe-exempt($hold: true)`), so the tabs stay un-inverted. The two must always share one definition.
- **Motion policy.** Both sit inside `mixins.when-motion-reduced(feedback)`: under the reduced treatment there is no blink; the negative is just on. The operator's setup (OS animations off, app motion on) gets the full blink, because `feedback` is not reduced by the OS setting (`frontend/app/motion/motion.ts`, `motion.test.ts:122`). The build adds no `prefers-reduced-motion` query.

The landing bounce on the dropped pill (`tab-bounce 300ms`, marker held 400 ms by `LANDING_BOUNCE_MS`) is a different effect and is not part of "the drag flash" (D2).

### 2.2 Moving a tab

- **Source.** Each pill is a pragmatic-dnd `draggable()` (`PaneTabStrip.tsx:829-867`). The payload is one `blockId`; the drag session (`drag-session.ts:27-55`, `pane-tab-drag.ts`) holds single scalars. `isDraggedPaneTab` dims the one dragged pill.
- **Targets.**
  - Another pane (header, strip, end icons or body) is one drop target (`foreignDropRootFor`, `PaneTabStrip.tsx:181-188, 401-429`). It accepts a pill from a *different* pane and appends it.
  - A pill in the same pane reorders.
  - Its own pane is not a target.
  - Outside every window tears the block off into a floating pane (`pane-tab-tearoff.ts`, one block).
  - The Window Tab bar and other windows accept nothing.
- **The move.** `moveBlockInStack` (`layoutStack.ts:278-342`) moves **one block per call**: a local edit, then the `pane.moveTab` RPC, then `LayoutStackMove` in srv (`crates/srv/src/reducer/layout.rs:448-493`, `crates/srv/src/backend/layout/mod.rs:266-317`). A pane whose last tab leaves is removed as a move (not a close: no prompt, no close guard). The moved tab becomes active in the target and the target pane takes focus. The source pane's active tab, if it moved, passes to the right neighbour (or the new last tab). Cross-pane moves always append, though srv can already insert before/after.
- **No undo** exists for a tab move.
- **Landing.** `landedTab` is one `{id, paneKey}` slot and one 400 ms timer.

### 2.3 Clicks on a pill

- A pill has `onMouseDown` (middle button closes), `onClick` (activates a pill that is not already active) and `onDblClick` (rename). **None reads a modifier key.** Pills have no `tabindex`, no ARIA and no key handler (`PaneTabStrip.tsx:869-886, 938-959`).
- The click bubbles to the pane root, which focuses the pane (`PaneChrome.tsx:557-558`).
- The whole-pane drag excludes pills (`PANE_DRAG_EXCLUDED = ".pane-tab, .pane-tab-strip-add"`, `drag-preview-size.ts:184`).
- Nothing else in the pill, strip, header, document-tab strip or Window Tab bar uses Ctrl+click, and none of them has a multi-select.

### 2.4 Collapsed panes

A collapsed pane is a leaf with `minimized: true`. It keeps its **full header, pill strip included, live and clickable**; only the body is hidden (`TileLayout.core.tsx:577`, `tilelayout.scss:112-114`, `layoutMinimize.ts`). Clicking a pill on it focuses the pane and switches its active tab, and **never expands it** (`PaneChrome.tsx:430-437`, `layoutStack.ts:171-191`). Clicking the already-active pill does nothing at all (`PaneTabStrip.tsx:880`). The only things that expand a chip today are its Restore button, `showBlockInPane` (Swarm and the editor-reuse path, `reveal-block.ts:70-82`) and an automatic restore after a close. `SPEC_RESIZE_THROUGH_COLLAPSED_PANES` R6 says a chip "keeps its click (restore)"; no such click exists in the code, so that sentence is stale and the build corrects it.

## 3. The flash: twice as fast, twice as long

**Which flash.** The drop-target blink on the destination pane's header (§2.1), and its outline pulse. Not the landing bounce (D2), not the Window Tab bar's or the document tabs' flash (they keep their settle-back strobe; one word each if the operator wants them too).

**Current numbers.** Blink: one 400 ms cycle (200 ms negative, 200 ms normal), then held negative. Pulse: 0.8 s per direction.

**Asked-for numbers.** Twice as fast: a 200 ms cycle (100 ms negative, 100 ms normal). Twice as long: the blinking phase lasts 800 ms instead of 400 ms. Together that is **four blinks in 800 ms: five flashes per second.**

**The conflict.** The repo's rule (WCAG 2.3.1, written into `drop-feedback.scss` and the 2026-10-21 spec) is no more than three flashes in any one second. Four blinks in 800 ms breaks it. A pane header is a small area, below WCAG's general-flash area threshold, so the content standard would not forbid it, but the repo's rule is stricter on purpose and this is the operator's call (D1):

| Option | Cycle | Blinks | Blinking phase | Flashes in the worst second |
|---|---|---|---|---|
| **A (chosen)** | 200 ms | 3 | 600 ms | 3 (at the limit) |
| B (not chosen) | 200 ms | 4 | 800 ms | 4 in 0.8 s |

A is twice as fast, three times as many blinks and one and a half times as long; B is the literal ask and needs the rule relaxed for this one case.

**Pulse.** 0.8 s becomes 0.4 s per direction (a 1.25 Hz outline colour fade on a 2 px outline, no luminance flash, so no conflict). It runs for the whole hover, so "twice as long" does not apply to it.

**Design.**
- `drop-feedback.scss`: replace the literal in `$drop-hold` with two variables, `$drop-blink-ms: 200ms` and `$drop-blink-count: 3` (4 for B), giving `tab-drop-invert-hold $drop-blink-ms steps(1, end) $drop-blink-count forwards`; `$drop-pulse` and `$drop-pulse-reduced` go to `0.4s`. The keyframes `tab-drop-invert-hold` do not change (0% negative, 50% normal, 100% negative): each iteration is one blink, and `forwards` holds the last keyframe, negative. The header and the counter-inverting pill strip use the same variable, so the two stay in step on the same frame.
- The reduced-motion treatment is untouched: no blink, the negative on for the whole hover.
- **Re-entry budget (new, needed for the rule to hold).** Today a pointer that leaves and re-enters replays the burst at once, so a quick wobble over two panes could put six flashes in a second. The build keeps one blink clock per window: a blink burst starts only when at least 1000 ms have passed since the previous burst began, anywhere in the window. Inside that second the target lands directly in the held negative (the reduced treatment's look, no blink). That makes "at most three flashes in any one-second window" true for the whole window, not only for one hover.
- `frontend/app/drop-feedback.test.ts` pins the literal `400ms steps(1, end) forwards` in four places; the build updates it to the new variable values and adds an assertion that the blink count times the cycle never puts more than three flashes in one second (with A: 3 blinks at 200 ms).

## 4. Multi-select

### 4.1 Gestures

All with the **platform modifier**: Cmd on macOS, Ctrl elsewhere, via the existing `isModKey(e)` (`frontend/util/platformutil.ts:24`). On macOS Ctrl+click is the right-click, and it already fires the pane menu (`blockframe.tsx:765`), which is why Cmd is not optional there.

| Gesture | Result |
|---|---|
| Mod+click a pill | Toggles that pill in the selection. It does **not** activate the tab, focus the pane or expand a collapsed pane (D4), so the layout holds still while the operator gathers tabs. |
| Mod+click a selected pill | Removes it from the selection. |
| Plain click on any pill | Clears the selection, then activates as today. |
| Escape | Clears the selection (§4.6). |
| Press on a selected pill and drag | Moves the whole selection (§4.3). |
| Press on an unselected pill and drag | Moves only that pill; the selection is left as it is. |
| Mod+double-click | Does not start a rename (two quick toggles must not open the editor). |
| Closing a selected pill (x or middle-click) | Removes it from the selection and closes only that tab. |
| Switching Window Tab, or a selected block leaving the layout | Clears (or prunes) the selection. |

The selection is explicit: the active tab of a pane is **not** implicitly part of it. It is per OS window, held in memory only (a restart clears it), and covers pane tabs of the current Window Tab's layout (a move across Window Tabs does not exist today either). Shift+click ranges and "select all in this pane" are not part of this build.

### 4.2 Looks

A selected pill is distinguished from the other pill states without colour alone and without reusing a channel they already own (active: inset bottom underline; dragging: opacity 0.4; drop before/after: inset left/right; activity flash: `::before`):

- `.pane-tab--selected`: a 2 px accent outline, offset inward (`outline-offset: -2px`), a faint accent background tint, and a small leading check mark. The mark is the non-colour cue.
- A selected pill that is also active keeps its underline.
- While a selection exists, the grabbed pill, once the drag starts, shows a count ("3") and every selected pill dims, like the single-tab dimming.
- Wiring: `selected` (and the count) are props from `PaneChrome` through `PaneHeaderTabStrip` to `PaneTabStripItem`. The strip stays dumb; the state lives in a new module, `pane-tab-selection.ts` (§10).

### 4.3 The drag and the drop

- **Session.** `DragSource` gets `blockIds: string[]` (the grabbed block plus the rest of the selection, in layout order); the pragmatic `data` and `PaneTabDragPayload` carry the same list. `isDraggedPaneTab` becomes a membership test. With one block the list has one entry and nothing changes.
- **Targets.** For a multi-block drag:
  - The pane-level foreign target (header, strip, end icons, body) accepts when **at least one** dragged block is not already in that pane. It may be the grabbed pill's own pane: gathering tabs into the pane you started from is the obvious gesture (D5).
  - The per-pill reorder targets reject a multi-block drag, so the drop is never ambiguous between "reorder" and "gather".
  - A pane that already holds every dragged block does not accept the drag (no hover flash).
- **What moves.** Every dragged block not already in the target. Blocks already there stay where they are and are not reordered. Membership is resolved **by block id**, not by the strip's `paneKey`: a fork pill shown in one pane's strip but belonging to another (`extraTabs`) is moved from the pane it really lives in.
- **Order of arrival.** Appended after the target's existing tabs in **layout order**: panes in tree order (the same order Cmd+1..9 uses), tabs in strip order within each pane. Click order is invisible to the operator, so it is not used (D6).
- **Active tab and focus.** The **grabbed** tab becomes active in the target, and the target pane takes focus, as a single-tab move does today. The other moved tabs arrive inactive.
- **Source panes.** Each source pane whose active tab left passes it to its right neighbour (or the new last tab), per the existing rule. A pane emptied by the move is **removed as a move, not closed** (same as the single-tab rule: no prompt, no close guard, `removeLeafEmptiedByMove`). A collapsed pane emptied this way loses its chip. If removing a source would leave only collapsed panes, the existing restore-nearest rule applies. The last pane in a window never closes; a move that would empty it cannot happen, since the target is another pane.
- **A collapsed target** expands on drop (D7): the moved tabs are selected on it by the drop, and a drop into a chip that stays a chip would hide where they went.
- **One command.** The move is one atomic operation, not N calls (§5).
- **After the drop** the selection is cleared and every moved pill plays the landing bounce.
- **Cancel.** Escape during the drag (or a drop on nothing) cancels the drag and keeps the selection, so the operator can try again.
- **Tear-off.** Dropping a multi-block drag outside every window is **cancelled** in this build (nothing moves, selection kept); it does not tear off only the grabbed tab and leave the rest (D8). Tearing off several tabs into one floating pane is a follow-up.
- **Not supported, as for a single tab:** dropping on the Window Tab bar, on another OS window, or across Window Tabs.

### 4.4 Different kinds of tabs together

Agent, terminal, browser and editor tabs can be selected and moved together; no view type is excluded from a pane-tab move today. Each moved block is unmounted in its source pane and mounted in the target's keep-alive stack (`pane-leaf-chrome.tsx`), adopting its ViewModel by block id. Inactive keep-alive members stay mounted but hidden, and a browser's native page is collapsed while its tab is dormant. Two things to check by eye in `task dev` (the 2026-09-24 spec left the first open too): that a moved agent's stream keeps rendering in the target; and what moving, say, eight terminals at once costs (eight remounts), measured before the build is called done. Document tabs of Editor and Media panes keep their own drag and are not selectable. Ephemeral panes' tabs cannot be moved, as today.

### 4.5 Undo

There is none for a single-tab move, and this build adds none. A multi-move scatters more, so a one-step "Undo move" toast is the follow-up worth making (it needs the previous layout kept for one step); it is named, not built (D9).

### 4.6 Escape, keyboard and accessibility

- **Escape order.** One Escape does one thing, in this priority: an open modal, an active search, then (new) clear the selection, then un-magnify the pane (`tilelayout-shared.tsx:123-131`, which has no `stopPropagation` and would otherwise fire on the same key). The selection handler does nothing when a drag is in flight (the drag owns Escape: `PaneTabStrip.tsx:727-731`), when the target is an editable field (rename, a view's inputs), or when the event is already handled. The build proves the order with a test, since three listeners are involved.
- **Keyboard.** Pills are not focusable and have no key handling today, and tab moves are mouse-only today (keyboard reorder is a named non-goal of the 2026-09-19 spec). This build therefore does **not** add a keyboard path, and does not make anything worse. It adds `aria-selected` on selected pills and a visible count; making pills focusable with a "move selected tabs to…" command is a follow-up spec of its own.
- **Not colour alone:** outline plus a check mark plus `aria-selected`.

### 4.7 Where Ctrl+click can and cannot collide

- **Pills and strip.** The handler is on the pill's own `onClick` (and its mousedown for the double-click guard), not on `document`. Nothing else handles a modifier there (§2.3). The click is not allowed to bubble to the pane root for a toggle, so selecting does not move pane focus.
- **Browser panes.** Page content is a native CEF view above the DOM; a click in it never becomes a DOM event in the app (`pane-overlay.ts`, `SPEC_BROWSER_PANE_CLICK_TO_SELECT`). The host's `browser-pane-clicked` signal carries no modifier and no button, and the build does not use it for selection. So a document-level listener could not even see page clicks, and this handler is not one. Ctrl+click on a link in a page is decided by the page and CEF (`NEW_BACKGROUND_TAB` → a new browser pane beside the opener, subject to the popup rules in `popup_route.rs`); the app's multi-select never sees that click. The pills of a *browser pane's own* strip are app chrome and take the selection like any other pill. While a tab is being dragged over a browser pane, the drag catcher already covers the page (`browser-view.tsx:59-117`).
- **macOS.** Ctrl+click = right-click there (the pane menu); the modifier is Cmd (`isModKey`).
- **Windows and Linux.** Ctrl held while an HTML5 drag starts can turn the drop cursor into "copy". The build forces the drag effect to "move" for pane-tab drags and tests it.
- **Wheel.** Ctrl+wheel zoom (`app.tsx`, `tab-reorder.ts:398`) is a wheel event, unaffected.
- **Doc-tab strips.** They reuse `PaneTabStrip` through `docDrag`; selection is switched off there.

## 5. The move command

N sequential `pane.moveTab` calls would be N optimistic edits, N RPCs and N version bumps, with a window in which the layout is half-moved, and a hazard if a failed call lands between "emptied" removals. Instead:

- **Frontend.** `moveBlocksInStack(model, blockIds, targetBlockId, activateBlockId)` in `layoutStack.ts`, looping `moveMemberAcrossStacks` (`stackMembers.ts:77-94`) inside **one** `updateTree` and one `localTreeStateAtom` write, emitting **one** `pane.moveTabs` command; emptied sources are removed once at the end with `removeLeafEmptiedByMove`.
- **Backend.** `pane.moveTabs` (`app_api` next to `pane.moveTab`, request type next to `CommandPaneMoveTabData` in `rpc_types/block.rs`) → `Command::LayoutStackMoveMany` (`crates/common/src/ipc.rs`, next to `LayoutStackMove` at `:592`) → one `stack_tree_edit` that applies `move_stack_member` (`backend/layout/mod.rs:266`) for each block under a single borrow. One `LayoutTreeReplaced` covers the common case; when sources empty it reconciles focus and magnify as `LayoutStackMove` already does (`reducer/layout.rs:478-492`). Other windows mirror it as a `stackmove` list (`layoutPersistence.ts:213-250`). The launcher's reducer lists the command next to `LayoutStackMove` (`crates/launcher/src/reducer/mod.rs:385`, `ipc/server.rs:1079`) and gets the same arm.
- A single-block move keeps using `pane.moveTab`; the new command is used when the list has more than one block, so the shipped path is untouched.
- **Failure** is as for a single move: the window shows srv's layout.

## 6. A collapsed pane expands when a tab on it is selected

- **Rule.** A **plain click on any pill** (the active one included) of a collapsed pane **expands that pane** (D3). The expansion is the existing layout intent `Restore` through `minimizeNodeToggle(nodeId)` (`layoutMinimize.ts:87-119`), the same call `showBlockInPane` makes; it is instant (the layout animation time is 0), is allowed even when it is the only pane, and is authoritative in srv.
- **Where.** A new `handleSelect(blockId)` in `PaneChrome`, called by the pill's `onClick` for **every** plain click (today the click is swallowed when the pill is already active, which is the common case on a chip, `PaneTabStrip.tsx:880`). It restores first, then runs the existing activation. It is placed **before** `model()?.onActivate`, which agent panes use to return early (`PaneChrome.tsx:434`). Order is restore, then activate, as in `showBlockInPane`; the two are separate commands, not one atomic write.
- **Other cases.**
  - A fork pill belonging to another pane keeps its present behaviour: it reveals the pane the block lives in, and that pane expands (already true through `revealBlockLocally`). The pane whose strip was clicked is not expanded for it.
  - A drop onto a chip expands it (§4.3, D7).
  - A Mod+click (multi-select) does not expand (D4).
  - Panes hidden by another pane's maximise have no clickable pills, so nothing changes there. Maximised-and-collapsed: `minimizeNodeToggle` restores the chip and leaves the maximise alone, as today.
  - Header and body clicks, and dragging the chip itself, are unchanged. Back/forward history navigation and Cmd+1..9 do not expand a chip today either; I have left them out as a different path (D3 asks).
- **Doc fix.** Correct the stale "restore on click" sentence in `SPEC_RESIZE_THROUGH_COLLAPSED_PANES` R6 in the build.

## 7. Edge cases

1. A selected block is closed from outside the strip (an agent exits, a pane closes): the selection prunes itself against the live layout.
2. A selected pill is in a pane that is then torn off or moved by a whole-pane drag: its block id is still in the layout and stays selected.
3. Select tabs in panes 1 and 2, drag one onto pane 2, where one already lives: that one stays, the others arrive.
4. Everything in the layout is selected and dropped on one pane: the other panes close in turn; the target remains.
5. A drag begins on a selected pill while another drag is somehow live: the existing single-drag guard (`beginDrag`) wins; one drag at a time.
6. Dropping a selection on a pane whose active tab is in the selection: the grabbed tab still becomes active.
7. Two windows: a selection in window A is invisible to window B and is not moved by it.
8. A plain click on a pill *after* a drag does not fire (HTML5 drag swallows it), so the selection cannot be cleared by the drop's own mouse-up.
9. The floating-pane header blocks pill drags (`floating-pane-workspace.tsx:394-404`); selection there still toggles, dragging stays off, as today.
10. Re-entry during a hover replays the blink (§3).

## 8. Tests and checks

- **Unit, pure:** the layout-order sort and the "already in target" filter; the selection store (toggle, clear, prune, scoping); the Escape priority; `moveMembersAcrossStacks` for mixed sources, an emptied source, and a block already in the target; `minimizeNodeToggle` call on a collapsed-pane click, active and inactive pill, agent pane.
- **Component:** `PaneTabStrip` Mod+click toggles and does not activate or focus; plain click clears; Mod+click on macOS is Cmd; Mod+double-click does not rename; `aria-selected` and the check mark; a multi-drag disables the per-pill reorder target and enables the own-pane target; every selected pill dims.
- **Stylesheet (`drop-feedback.test.ts`):** the new variables; header and counter-invert use the same; the per-second flash count is at most three (A); reduced-motion treatment unchanged.
- **srv:** `LayoutStackMoveMany` in `reducer/layout/tests.rs` beside the `LayoutStackMove` tests: order, active tab, one emptied source, several emptied sources, a source that is the root, the target already holding some, and a failed lookup leaving the tree untouched.
- **By eye in `task dev`:** the flash at the chosen speed; a multi-move of an agent, a terminal and a browser tab together; a collapsed pane expanding on a click and on a drop; macOS with Cmd; a page link Ctrl+click in a browser pane still opens its own pane.

## 9. Decisions for the operator (recommendations first)

- **D1 (flash) — decided: A.** The operator: follow WCAG 2.3.1 and go as fast as the standard allows. The standard caps the number of flashes in a second, not the speed of a burst, so the limit on speed is that each phase stays clearly visible: with a 200 ms cycle each phase is 100 ms, six frames at 60 Hz. A faster cycle (150 ms and below) would shorten the whole burst below the 400 ms of today and undo "twice as long", for a blink that is harder to see, so the build uses 200 ms × 3 = 600 ms, plus the re-entry budget in §3 so the three-per-second limit holds across re-entries. B is not used.
- **D2 — decided as recommended.** "The drag flash" means the blink and pulse on the destination pane's header. The landing bounce after the drop (300 ms) is left alone. Say if it should speed up too. **Recommend as written.**
- **D3 — decided as recommended.** Expand on a plain click of any pill, including the active one, and not on back/forward history or Cmd+1..9. **Recommend as written.**
- **D4 — decided as recommended.** Mod+click selects without activating, focusing or expanding. **Recommend as written.**
- **D5 — decided as recommended.** The grabbed pill's own pane may be the drop target for gathering. **Recommend yes.**
- **D6 — decided as recommended.** Moved tabs land in layout order, not click order. **Recommend layout order.**
- **D7 — decided as recommended.** A drop onto a collapsed pane expands it. (SPEC_DRAG_MINIMIZED_PANES D1 is about moving the chip itself, which stays a chip.) **Recommend yes.**
- **D8 — decided as recommended.** A multi-drag released outside every window is cancelled, not torn off partially. **Recommend cancel; tear-off of several is a follow-up.**
- **D9 — decided as recommended.** No undo in this build. **Recommend a follow-up "Undo move" toast.**

## 10. Files the build touches

- `frontend/app/drop-feedback.scss`, `frontend/app/drag/drop-indicators.scss` (comment only), `frontend/app/drop-feedback.test.ts`
- `frontend/app/element/PaneTabStrip.tsx`, `PaneTabStrip.scss`, `PaneChrome.tsx`, `PaneHeaderTabStrip.tsx` (and their tests)
- new `frontend/app/element/pane-tab-selection.ts` (+ test)
- `frontend/app/drag/drag-session.ts`, `pane-tab-drag.ts`, `pane-tab-tearoff.ts` (cancel a multi-drag), and the three cross-window monitors where they read the payload
- `frontend/layout/lib/layoutStack.ts`, `stackMembers.ts`, `layoutPersistence.ts`, `tilelayout-shared.tsx` (Escape order)
- `crates/common/src/ipc.rs`, `crates/srv/src/backend/layout/mod.rs`, `crates/srv/src/reducer/layout.rs` (+ tests), `crates/srv/src/backend/rpc_types/`, the `pane.moveTab` handler's neighbour, `crates/launcher/src/reducer/mod.rs`, `crates/launcher/src/ipc/server.rs`, generated TS types
- Docs: `SPEC_DROP_TARGET_LANDS_INVERTED` and `SPEC_PANE_TAB_DRAG_AND_DROP` (cite this), `SPEC_RESIZE_THROUGH_COLLAPSED_PANES` (R6 fix), the spec index
- One changeset (`minor`: a new feature) in the build PR. The spec ships in the build PR, not on its own.

## 11. Order of the build

1. The flash (D1), its test; small and independent.
2. Expand-on-select and the Escape/selection store with Mod+click and the looks (no move yet).
3. The drag payload, the batch command and the drop, then the tear-off cancel.

These can be one PR or three, as the reviewer prefers; the batch command is the part that needs the srv review.

Agent: Camper@narko
