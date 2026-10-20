# Spec: Selecting a pane from Swarm — the lost click, and one state machine for "show this pane"

**Status:** active — decisions taken 2026-10-20 (section 9); P0 (the trace) is in #5193; P1 (stable rows, press to select, keyboard, quiet poll) is in the next PR; P2 to P4 not started. Verified 2026-10-20.
**Date:** 2026-10-18
**Author:** AgentY
**Requested by:** the owner, 2026-10-18: "sometimes, but not all the time, if I click on an agent in swarm, nothing happens, I need to click a 2nd time. it is not always reproducible, likely has to do with some order of selection which makes me think perhaps we need a proper architecture like state machine. take a closer look, write spec to file."
**Verified against:** `agentmux` `main` @ `f2b8ad56d` (2026-10-18), by reading code, plus one throwaway test (section 3.4). I have **not** reproduced the lost click in a running build.
**Builds on:** `SPEC_REVEAL_BLOCK_ONE_PATH_2026_09_27.md` (the reveal path), `SPEC_SWARM_ROW_AGENT_COLOR_AND_SELECT_TO_FOCUS_2026_09_25.md` (the click handler), `SPEC_FOCUS_FOLLOWS_SELECTION_2026_10_08.md` (the caret), `SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md` (layout focus intents).

---

## 0. Summary

1. **There is a confirmed mechanism that loses clicks, and it matches "sometimes, not reproducible".** Swarm rebuilds every row's DOM whenever anything it reads changes, and also every 12 seconds regardless. A click is a press and a release. If the row under the pointer is replaced between the two, Chromium sends no `click` at all, so the row's handler never runs. The second click works because no rebuild landed inside it. It happens more when agents are busy, because busy agents change the data constantly. (Section 3.)
2. **Separately, the reveal path is an unguarded async script**: several awaits and a poll, no supersession, no post-condition check, silent on failure, and the "selected" state lives in five places written by different code at different times. That can produce more "nothing happened" cases, but I could not confirm which occur in practice from reading alone. (Section 4.)
3. **Recommendation, in this order:**
   - **P0, a day:** a trace of every select and every Swarm row rebuild, so the intermittent class becomes data.
   - **P1:** fix the confirmed cause. Rows keep their DOM across updates; the 12 s poll stops announcing "changed" when nothing changed; the row activates on press, not release.
   - **P2:** a **selection state machine** that owns "show this pane" from intent to a verified result, replaces `revealBlock`'s script, supersedes older intents, and reports failure instead of doing nothing. (Section 6.)
4. The state-machine instinct is right for the second problem and not needed for the first. The lost click is a list-rendering defect that a state machine would not have prevented, so do P1 first.

---

## 1. What the owner sees

Click an agent row in Swarm. Normally its pane is selected, its window tab or pane tab is shown, and the caret goes there. Sometimes nothing visibly happens. A second click works. It does not reproduce on demand.

## 2. What happens on a click today

| Step | Where | What |
|---|---|---|
| Press | `swarm-view.tsx` agent card `onMouseDown` | `preventDefault()`, so the Swarm pane never takes DOM focus (and flashes selected) before the click. |
| Release/click | `swarm-view.tsx` card `onClick` | `stopPropagation()`, then `void revealBlock(node.blockId)`. |
| Reveal | `util/reveal-block.ts:101` `revealBlockLocally` | find the window tab holding the block (`findTabHolding`, `:46`), `await setActiveTab`, poll up to 10 x 50 ms for the tab's layout, `showBlockInPane`, `model.focusNode`, `giveBlockFocus` (caret). Returns `false` silently if the block is not found; `revealBlock` (`:139`) then asks srv to reveal it in another window. |
| After the tab switch | `store/tab-actions.ts:354` | two animation frames after the `SetActiveTab` RPC, `focusManager.refocusNode()` runs on the active tab's focused node. |

Selection state is spread over: the window's `activeTabId` (with an optimistic "intent" while the RPC is out), the target tab's layout `focusedNodeId`, the pane stack's active member, the DOM caret, and Swarm's own `focusedBlockId` memo.

---

## 3. Finding 1 (confirmed): Swarm rebuilds the row under the pointer

### 3.1 The chain

1. `swarm-view.tsx:110`: `const tree = createMemo(() => model.buildTree());`
2. `swarm-model.ts:1994` `buildTree()` returns, for each agent, a **new object literal** (the `return { blockId, agentName, ... }` at about `:2069`). Only the workflow-dispatch sub-objects are identity-stabilised (`stabilizeGroupIdentity`); the row nodes themselves are not.
3. `swarm-view.tsx:183`: `<For each={tree()}>{(node) => <AgentRow node={node} .../>}</For>`. Solid's `For` is keyed by object reference, so a list of fresh objects **destroys and re-creates every row's DOM**.
4. `AgentRow` (`:333`) takes `{ node, focusedBlockId, model }` destructured, so it is not written to follow changes in place. It relies on being re-created. That is why the design works at all, and why it cannot simply be switched to a keyed list without a change to the row.

### 3.2 What triggers a rebuild

`buildTree()` reads, for every agent: tracked block ids, subagents, dispatches, shells, crons, background tasks, statuses, progress, retired keys, and **each block's meta** (`MOS.getMuxObjectAtom`). Any of them changing recomputes the memo, and so rebuilds every row.

- **Unconditionally every 12 s**: `swarm-model.ts:1155` (`TRACKED_BLOCKS_POLL_MS = 12_000`) and `:1300` call `loadTrackedBlocks()`, which does `this.setTrackedBlockIds(ids)` (`:1345`) with a freshly parsed array, so the signal fires even when the set is identical. (The code just below it compares the set to skip the status re-subscription, but not before the set call.)
- **On every event while agents work**: status changes, progress/todo updates, block-meta writes (activity summary, context tokens), subagent and dispatch reloads.

### 3.3 Why a rebuild loses the click

A click is delivered to the nearest common ancestor of the press target and the release target. If the pressed element is removed in between, Chromium (and Safari) send `mousedown` and `mouseup` but **no `click`**; Firefox is more forgiving. This is long-standing and documented in the UI Events and Pointer Events discussions:
[w3c/uievents #141](https://github.com/w3c/uievents/issues/141),
[w3c/pointerevents #492](https://github.com/w3c/pointerevents/issues/492).
The row's `onClick` is on the row, so it never runs. The user sees nothing, and clicks again.

### 3.4 Evidence

A throwaway test (not committed): a `createMemo` that returns fresh objects, rendered with `For`; after an unrelated signal changed, the row element was a different node and the old one was disconnected. That is the Solid behaviour step 3 depends on. What I could **not** do from reading is count how often rebuilds happen in a real fleet, or watch a press straddle one. P0 below does that.

Rough odds, to be replaced by the P0 numbers: the 12 s poll alone hits a 100 ms press about 1 time in 120 (under 1 %). Event-driven rebuilds in a busy fleet are far more frequent; if they come every second or so, one click in ten is plausible. That fits "sometimes", "not reproducible", and "more when agents are working".

### 3.5 Side effects of the same defect (worth fixing for free)

Every rebuild resets the row's hover, any in-progress text selection, the expanded state of children that is not model-backed, and scroll anchoring, and costs a full re-render of the list on each tick. The same `For`-over-fresh-objects shape may exist elsewhere (the Tower detail list and the token popover also call `revealBlock`); audit them in P1.

---

## 4. Finding 2 (from reading, needs the trace to rank): the reveal path

These are weaknesses of `revealBlockLocally` / `setActiveTab` / `focusNode`. Each is verified in code; whether each *happens in practice* is not yet known. The labels say how sure I am.

| # | Weakness | Where | Effect | Confidence |
|---|---|---|---|---|
| R1 | **No supersession.** Two reveals overlap freely: the first can finish after the second and win. | `reveal-block.ts` (no token or queue) | Click A then B quickly: ends on A. | Verified: no guard. |
| R2 | **Layout wait is a fixed 500 ms poll that fails silently.** After a window tab switch, a cold tab can take longer. | `LAYOUT_POLL_TRIES = 10`, `LAYOUT_POLL_MS = 50` | Returns `false`; `revealBlock` falls through to the "other window" call, which shows nothing for a block in this window. | Verified. Frequency unknown. |
| R3 | **`findTabHolding` trusts cached `Tab.blockids`** and takes the first tab (pinned first) that lists the block. | `reveal-block.ts:46` | A block just created, forked or moved may not be listed yet, or be listed in two tabs: wrong tab or "not found". | Inference: depends on how current `Tab` rows are. |
| R4 | **`focusNode` returns early when `focusedNodeIdStack[0]` already equals the node**, with no check that the pane is actually the one shown. | `layout/lib/layoutFocus.ts:190` | A "select" that changes nothing visible when the layout's idea and the screen disagree. | Inference. |
| R5 | **A delayed second focus.** `setActiveTab` schedules `refocusNode()` two frames after its RPC, using the *active tab's* focused node at that moment. | `tab-actions.ts:354` | If anything changed selection in between (a second click, the user), this can pull focus back to something else. | Inference from ordering. |
| R6 | **The caret has its own deferral and retries** (waits for a drag press to end, retries 60 frames) that the caller cannot see. | `focusManager.ts:146` `giveBlockFocus` | "Selected" and "can type" can diverge for up to a second. | Verified. |
| R7 | **Failure is silent.** `revealBlock` returns `void`; a miss is a `console.warn` at best. | `reveal-block.ts:139` | Every cause above looks identical to the user: nothing. | Verified. |

R7 is the important one for the owner's "order of selection" suspicion: with no result and no trace, a race looks like nothing happened, and we cannot tell the causes apart.

---

## 5. Goals and non-goals

**Goals**
- A click on a Swarm row never gets lost, however busy the fleet is.
- "Show this pane" has one owner, one definition of done, and always ends in a visible success or a visible, explained failure.
- The newest intent wins; an older one can never finish on top of it.
- Every select leaves a trace good enough to diagnose the next intermittent case without guessing.

**Non-goals**
- No change to layout persistence, the intent outbox, or in-tab keyboard navigation (arrow keys, Cmd+1..9) in v1.
- No new dependency. A small typed machine is enough; XState is not in the tree.
- Not redesigning Swarm's rows.

---

## 6. Design

### 6.1 Fix the cause first (P1)

1. **Rows keep their DOM.** `AgentRow` takes `node: Accessor<AgentTreeNode>` and reads it inside tracking scopes (about 40 `node.` uses in the row, all in one file). The list is keyed by `blockId` (a keyed `mapArray`, or `reconcile` over a store keyed by `blockId`), so an update changes text in place and never replaces the element.
2. **`setTrackedBlockIds` only fires on a real change** (compare the id set first). This alone removes the 12 s periodic rebuild.
3. **Activate on press, not release.** The card activates on `pointerdown` (primary button, not a drag) through one delegated listener on the stable list container, reading the block id from a `data-block-id` attribute. Reasons: it cannot be lost to anything that happens before the release; it is the native behaviour of lists and menus; and it feels faster. Keyboard: the card gets `role="button"`, `tabindex="0"`, Enter and Space call the same function. The activation is idempotent per press (a press and its click do not select twice).
4. `mousedown` keeps its `preventDefault` so the Swarm pane does not flash selected first.

### 6.2 One state machine for "show this pane" (P2)

**Shape.** A pure transition function plus an effect runner, in `frontend/app/selection/`:

- `selection-machine.ts`: `transition(state, event) -> { state, effects }`, no I/O, fully unit-testable.
- `selection-effects.ts`: runs effects (`setActiveTab`, wait for layout, `showBlockInPane`, `focusNode`, `giveBlockFocus`, ask another window) and feeds results back as events. Each effect carries the intent id and is dropped if that id is no longer current.
- `selection-trace.ts`: a ring buffer of the last 50 transitions with timestamps.
- `revealBlock(blockId, opts)` stays as a thin wrapper that calls `requestSelect` and returns a `SelectionResult` (`settled` or `failed` with a reason), so existing callers keep working.

**Intent.** `{ id, blockId, source: "swarm" | "tower" | "tokens" | "picker" | "remote-window", tabHint? }`. One current intent per window.

**States**

| State | Meaning | Leaves on | Deadline |
|---|---|---|---|
| `idle` | nothing in flight | `select` | none |
| `resolving` | find the window tab or window that holds the block | found here -> `switchingTab` or `awaitingLayout`; found elsewhere -> `handoff`; none -> `failed(not-found)` | 1 s |
| `switchingTab` | the window tab is changing (RPC out) | switched -> `awaitingLayout` | 3 s |
| `awaitingLayout` | the tab's layout must contain the block | present -> `showing`; deadline -> `failed(layout-timeout)` | 1.5 s (was a silent 0.5 s) |
| `showing` | restore if minimized, un-magnify others, make the pane's tab active | done -> `focusing` | 200 ms |
| `focusing` | set the layout focus, request the caret | done -> `verifying` | 200 ms |
| `verifying` | read the state back and compare with the intent | all hold -> `settled`; mismatch and no retry used -> `showing` (one retry); else `failed(mismatch: ...)` | 300 ms |
| `handoff` | another window owns the block; ask it | acknowledged or deadline -> `settled`/`failed(remote-timeout)` | 3 s |
| `settled` | done; the locate flash plays now | `select` | none |
| `failed(reason)` | not shown; the user is told | `select` | none |

**Events:** `select(intent)`, `resolved(where)`, `tabSwitched`, `layoutReady`, `layoutTimeout`, `shown`, `focused`, `verified(ok | mismatch)`, `remoteDone`, `deadline(state)`, `userMoved`, `blockGone`.

**Rules**
- **Latest wins.** A `select` in any state ends the current intent and starts the new one. Effects of an ended intent are discarded (R1).
- **Do not fight the user.** If the user does something that selects another pane, window tab or window while an intent is in flight (`userMoved`), the intent ends as `superseded`, not retried.
- **Read back, don't assume.** `verifying` checks: the window's active tab is the holder; the pane's active member is the block; the layout's focused node is the pane; the pane reports itself focused; the caret is in the block, or `giveBlockFocus` deferred it for a stated reason (a drag press, a modal, a covering overlay). One automatic retry from `showing`.
- **A select on the already-selected target** still runs `verifying` and re-focuses, so a stale click is repaired, and still plays the flash (it confirms the click).
- **A block that disappears** (`blockGone`) ends as `failed(gone)`.
- **Deadlines are explicit** and each produces a named reason, replacing silent `false`.

### 6.3 What the user sees

- `settled`: the locate flash (already built) is the confirmation. It fires from the machine, not from the reveal function.
- `failed`: a small non-blocking notice, "Couldn't show *Name*: <reason>", with the reason in plain words (still loading, moved, no longer open). No more silent nothing.
- `superseded`: nothing.

### 6.4 Trace

Each transition logs `[select] id=7 swarm blk=ab12 resolving->awaitingLayout +38ms`. The ring buffer is readable from the console. P0 adds the same style of line for Swarm: `[swarm] rows rebuilt (cause: progress|status|meta|poll|...)`, and a counter of presses whose `click` never arrived (pointerdown on a row with no matching click within 400 ms).

### 6.5 Cross-window

The window that owns the block runs its own machine for the same intent id. v1 keeps today's one-way `block:reveal` event but gives the sender a deadline and a trace line. v2 adds a `block:revealed` acknowledgement so the sender's machine can finish `settled` or `failed(remote-timeout)`.

### 6.6 What stays out

In-tab pane navigation, pane creation and close keep calling the layout directly. If the machine proves itself, they can move onto it later (section 9).

---

## 7. Phasing

| Phase | Content | Why this order |
|---|---|---|
| **P0** | Trace lines: selects, Swarm rebuild causes and counts, presses without a click. Small PR, no behaviour change. | Turns "sometimes" into numbers, and confirms or corrects section 3. |
| **P1** | Stable keyed rows; `AgentRow` on an accessor; id-set comparison before `setTrackedBlockIds`; press-to-activate with keyboard support; audit Tower and the token popover. | Removes the confirmed cause. |
| **P2** | The machine for the reveal flows, with the failure notice and the flash wired to `settled`. | Fixes the class of race (R1-R7) and gives one owner. |
| **P3** | Route the remaining `revealBlock` callers and the MCP/`block:reveal` entry through `requestSelect`. | Removes the duplicate paths. |
| **P4** | Acknowledgement for cross-window reveals. | Closes the last silent case. |

P0 can ship alone and be read for a day before P1/P2 are tuned.

---

## 8. Tests

- **Row identity (P1).** Render the real `SwarmView` rows; cause a rebuild (a status change, a poll tick); assert each row's element is the same node afterwards.
- **Press across a rebuild (P1).** Dispatch `pointerdown`, trigger a rebuild, dispatch `pointerup`/`click`; assert exactly one select happened, and that it also happens when the click never arrives.
- **Poll equality (P1).** Two polls returning the same ids do not notify.
- **Machine table (P2).** Table-driven over every state x event, including: select during each state; double select of the same target; select A then B (B wins even if A's effects finish last); tab closed mid-flight; layout never arrives; stale `Tab.blockids`; user selects elsewhere mid-flight; verify mismatch then retry then fail.
- **Effects (P2).** Fake effect layer with controllable completion order, so the interleavings that real timing produces once in a hundred are forced every time.
- **Result contract (P2).** `revealBlock` resolves with `settled` or a `failed` reason; the notice appears for each failure reason.
- **Trace (P0).** The Swarm counters and `[select]` lines appear with the right causes.
- **By hand (a `task dev` build, with 6+ busy agents):** click rows rapidly while they stream; before P1 count lost clicks over 100 presses, after P1 expect zero.

## 9. Decisions for the owner

Taken 2026-10-20: all four as recommended (press-to-activate with keyboard support; a quiet failure notice; reveal flows only; P0 ships first and is read for a day before P2 is tuned). P1 does not wait on that data: it removes the confirmed cause.

1. **Press or release?** I recommend activating on press (section 6.1), keeping keyboard activation. The cost is that a press that starts a drag-select elsewhere would still select; rows are not draggable, so I see none.
2. **Show a notice on failure?** I recommend yes, a quiet one. Without it, a failed reveal looks exactly like today's problem.
3. **Machine scope.** Reveal flows only (recommended), or all pane selection (arrow keys, Cmd+1..9, creation) later if it earns it.
4. **Ship P0 alone first?** I recommend yes: one release of trace data makes P2's deadlines and retry rules evidence-based.

## 10. References

- Code: `frontend/app/view/swarm/swarm-view.tsx` (`:110`, `:183`, `:333`), `swarm-model.ts` (`:1155`, `:1300`, `:1345`, `:1994`), `frontend/app/util/reveal-block.ts` (`:46`, `:101`, `:139`), `frontend/app/store/tab-actions.ts` (`:286`-`:356`), `frontend/layout/lib/layoutFocus.ts` (`:190`), `frontend/app/store/focusManager.ts` (`:66`, `:146`).
- Browser behaviour: [w3c/uievents #141, click not fired when the node is removed and re-added](https://github.com/w3c/uievents/issues/141); [w3c/pointerevents #492, mousedown/click target when the press target is removed](https://github.com/w3c/pointerevents/issues/492).

Agent: AgentY@narko
