# Agent pane scroll-follow: one owner, one intent-based state machine

**Status:** active — Phase 0 (the hotfix for both reported symptoms, §8) shipped with this spec in PR #3652; invariant I4 (follow-transition logging, §5.3) shipped ahead of Phase 1 as Phase 0b; Phases 1–5 not started. Tracking: issue #3655.
**Date:** 2026-09-24.
**Requested by:** repo owner (asafebgi): "rock solid scrollbar for agent panes … this may need an architecture rethink, perhaps a DRY round."
**Author:** Agent2.
**Supersedes, once implemented:** the follow and disengage logic in `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx` (`handleScrollNow`, `pendingProgrammaticScroll`, `userScrollInput`, `pinnedGeometry`, the first-overflow force-pin) and the ad-hoc follow code in `ToolOverlayLog.tsx` and `SystemToolInstallInline.tsx`.
**Related:** `docs/specs/SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md`, `docs/specs/SPEC_WORKING_STATE_AND_SCROLL_FOLLOW_HARDENING_2026_07_27.md`, `docs/specs/REPORT_AGENT_PANE_SCROLL_PIN_FLICKER_AUDIT_2026_07_30.md`, `docs/specs/SPEC_AGENT_PANE_FIRST_OVERFLOW_SCROLL_PIN_FIX_2026_08_29.md`, `docs/specs/SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md`, `docs/specs/SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md`.

All line numbers are against `main` at `be548a696`. "VL" is short for `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx`.

---

## 1. What the owner sees

- **A. A new pane doesn't follow at first.** When an agent pane starts, the transcript does not scroll with the conversation. The user has to scroll to the bottom by hand; after that it follows "for some time".
- **B. It gets stuck later.** After a while it stops following and sits on old content while new messages arrive below.

Neither symptom leaves a log line (see §2). That is why ~20 earlier passes (§3) fixed neighbors of these bugs but not these.

## 2. Root causes (confirmed by reading the code)

### 2.1 Bug A: our own pin triggers "load older history", which turns follow off

`handleScrollNow` (VL:969) runs for every scroll batch, including the one our own pin causes. After the follow checks it runs the older-history pagination block (VL:1110-1149). That block has no check for **who** scrolled:

```ts
if (props.onLoadOlder && isNearTop(scrollTop) && !loadingOlderInFlight && !(props.loadingOlder?.())) {
    ...
    if (anchor) props.viewState.captureHeadAnchor(anchor);   // VL:1148
```

`captureHeadAnchor` sets `stickToBottom` to `false` (`virtualization/state.ts:156-160`). `isNearTop` is `scrollTop < 50` (`anchor.ts:91`).

Here is the sequence in a new session:

1. `stickToBottom` starts `true` (`state.ts:144`). Nothing overflows yet, so no scroll events fire.
2. The first streamed flush that overflows adds δ px, usually one or two lines, so δ < 50.
3. The content ResizeObserver (VL:864-881) pins: `scrollTop = δ`. That fires a scroll event.
4. `handleScrollNow` sees a trusted pin batch near the bottom, so follow stays on. It then continues to the pagination block: `isNearTop(δ)` is true, and `onLoadOlder` is always passed in (`agent-view.tsx:2420`).
5. `captureHeadAnchor` runs, and **follow is now off**.
6. `loadOlder` returns immediately, because a new session has no older history (`hooks/useHistoryPagination.ts:161`, `if (currentOffset === 0) return;`). So nothing visible happens and nothing is logged.
7. Both ResizeObservers now skip the pin (`if (!stickToBottom()) return`, VL:815 and VL:870). The pane stays put.
8. When the user scrolls to the bottom, follow comes back on (VL:1072-1075). From then on `scrollTop` is well above 50, so this path cannot fire again. **That is the "it works for some time" part.**

The same thing happens again whenever the scroll range drops back under 50 px and grows: `/clear` without a remount, or the documented whole-pane `scrollHeight → 0` collapse (issue #2648).

### 2.2 Bug B: anything that isn't provably our pin counts as the user scrolling away

The disengage rule (VL:1076-1106):

- follow turns **off** when the batch is "not near bottom", **unless** `wasProgrammatic && !hadUserInput`;
- so every other batch counts as the user scrolling away.

Two inputs feed that rule, and both are unreliable:

- **B1. A user-input flag that never expires.** `userScrollInput` (VL:195) is set by `wheel`, `touchstart`, `touchmove` and `pointerdown` on the scroller, in the capture phase, and by scroll keys typed anywhere in the document that isn't an input (VL:1208-1228). It is cleared **only** when the next scroll batch is handled (VL:980). Clicking into the transcript to focus it, selecting text, expanding a tool block, wheeling over a nested tool log, or pressing an arrow key in another pane leaves the flag set indefinitely. The next scroll batch, usually our own pin minutes later, is then treated as the user's. That batch reads **live** geometry (VL:986-990). The cross-pane stream scheduler (`frontend/app/view/agent/stream-scheduler.ts`) has already added the new content in the same frame, but the pin hasn't run yet. So the gap is "the new content's height". A tool output or code block easily exceeds the 200 px threshold, and follow turns off at VL:1105.
- **B2. Scrolls the browser makes on its own have no flag.** Examples: a shrink clamp, or a Chromium scroll-anchoring adjustment (`overflow-anchor: auto` on `.agent-document`, `styles/_document.scss:145`). These arrive with `wasProgrammatic = false` and fall into the disengage branch even though no user did anything. VL:1079-1084 says a batch without user input should keep following. The code only applies that to batches it can prove are its own pin.

Contributing factors (they make A and B more likely; they are not separate bugs):

- **Near-bottom threshold halving.** When the scroll range is under 200 px, the "near bottom" zone becomes half the range (`anchor.ts:78-80`), so a short, new conversation has a zone of a few px. That existed so a user could still escape a short range. With intent-based detach (§5) it is no longer needed.
- **Observers bound once at mount.** The content ResizeObserver observes `virtualContainerRef` and `streamingBufferRef` once in `onMount` (VL:878-879). The streaming buffer is inside `<Show when={partition()}>` (VL:1411). If that element is ever recreated, content growth is no longer observed, and follow degrades to viewport-resize pins only. This isn't shown to happen today, but nothing prevents it.

### 2.3 Why these bugs keep coming back

Nobody owns "are we following". The decision is spread across at least ten participants that each carry part of the rule:

| # | Participant | Where |
|---|---|---|
| 1 | `stickToBottom` / `headAnchor` store | `state.ts` |
| 2 | Near-bottom threshold + halving, near-top threshold | `anchor.ts` |
| 3 | `handleScrollNow` engage/disengage | VL:1072-1107 |
| 4 | `pendingProgrammaticScroll` one-shot flag | VL:180, 298, 309, 977 |
| 5 | `pinnedGeometry` trusted-geometry reuse | VL:190, 303, 985 |
| 6 | `userScrollInput` sticky flag | VL:195, 1208-1228 |
| 7 | First-overflow force-pin hack | VL:1036-1070 |
| 8 | Pagination anchor capture (disengages) | VL:1110-1149 |
| 9 | Two ResizeObservers (viewport, content) + no-RO fallback effect | VL:774-881 |
| 10 | `jumpToBottom` callers (typing, send, queued turn) | `agent-view.tsx:1482, 1672, 2706`; `AgentFooter.tsx` |
| 11 | `scrollToNode` (disengages) | VL:896-922 |
| 12 | CSS `overflow-anchor: auto` | `_document.scss:145` |

Each earlier fix added a participant or a flag to decide the same question from geometry, with more exceptions each time. §3 lists them.

## 3. History, briefly

About twenty passes since April. Highlights:

- **2026-04-13 (#348, #371):** jump to the bottom on typing and on send.
- **2026-04-15 (#400):** threshold 50 → 200 px, because growing content read as a user scroll.
- **2026-05-09 (#783/#784):** the store (`stickToBottom`/`headAnchor`) and the anchor helpers.
- **2026-06-03 (#1257):** re-pin when a hidden tab is shown again.
- **2026-06-12 (#1363):** threshold halving on short ranges.
- **2026-07-24 (#2292):** the pin tracks `totalSize`.
- **2026-07-27 (#2326):** re-pin on any viewport resize; the "input-gating race" was explicitly deferred.
- **2026-07-28 (#2349):** `pendingProgrammaticScroll`.
- **2026-07-30 (#2370):** content ResizeObserver.
- **2026-08-29 (#2834):** first-overflow force-pin, root cause unknown. It was very likely §2.1, which runs in the same batch.
- **2026-08-31 (#2887…):** the content-resize contract. It counts "nine passes" at this bug class; steps 4–6 are still pending.
- **2026-09-23 (#3599):** `pinnedGeometry` and `userScrollInput`. This is where B1 became reachable: the gate changed from `if (wasProgrammatic)` to `if (wasProgrammatic && !hadUserInput)`.
- **2026-09-23 (#3610, #3611):** head migration height handoff; turn-scoped tail.

Recurring themes: (1) programmatic or browser scrolls misread as the user; (2) height changes the pin didn't watch; (3) shrinks that can't be corrected after layout clamps them; (4) zero-size and first-layout transitions; (5) threshold edge cases; (6) bugs that only show up in a live app and that agents couldn't reproduce.

## 4. Best practice (external research)

- **stackblitz-labs/use-stick-to-bottom** (the basis of Vercel ai-elements `Conversation` and shadcn AI chat): a ResizeObserver on the content element; ignores scroll events that match its last written `scrollTop` or happen during a resize; only an **upward** user scroll escapes (`wheel deltaY < 0` escapes immediately); re-locks only near the bottom; escapes on text selection. <https://github.com/stackblitz-labs/use-stick-to-bottom/blob/main/src/useStickToBottom.ts>
- **assistant-ui ThreadViewport:** "user scrolled up" means `scrollTop` decreased **and** `scrollHeight` stayed the same, "to rule out content-driven shifts being misread as a user scroll up". Pending bottom intent is cancelled by `pointerdown` and by Enter/Space. <https://github.com/assistant-ui/assistant-ui/blob/main/packages/react/src/primitives/thread/useThreadViewportAutoScroll.ts>
- **xterm.js:** following is **model state** (`isUserScrolling`), not geometry. User input scrolls back to the bottom.
- **TanStack Virtual chat mode** (`anchorTo:'end'`, `followOnAppend`): pinned stays pinned across item growth. Only items entirely above the fold get size compensation (3.17.6).
- **Lody** replaced `use-stick-to-bottom` with explicit `follow` / `free` / `anchored` modes after geometry heuristics snapped users back.
- **Hermes** fixed a race by escaping *synchronously* on `wheel deltaY < 0`, rather than in a deferred scroll handler that a same-frame ResizeObserver pin could override.

Principles taken from this:

1. **Follow is intent, stored explicitly.** Geometry only answers "is the user at the bottom" when they might re-engage, and drives the "jump to latest" button.
2. **Only a user gesture can release follow.** A raw `scroll` event on its own never does.
3. **Content growth pins; it never releases.**
4. **Re-engage only at the true bottom**, with a small threshold, or on an explicit action.
5. **Pin inside the ResizeObserver callback** (after layout, before paint): no flash, no forced layout.
6. **Hidden or zero-size means suspended**, not "scrolled away".

## 5. Design

### 5.1 One controller, one primitive

A new module, `frontend/app/view/agent/scroll/follow-controller.ts`, has two layers:

- **A pure reducer.** `followReducer(state, event) → { state, effects }`. It has no DOM or Solid dependencies and is table-testable. It follows the reducer pattern `agent-pane-state/reducer.ts` already uses.
- **A thin DOM binding.** `createFollowScroll(scrollEl, contentEls, opts)` owns every listener and observer for one scroller. It turns DOM facts into reducer events and executes the effects (pin, emit a telemetry line, request an older-history load).

**Every place that follows content uses `createFollowScroll`:** the transcript, `ToolOverlayLog`, and `SystemToolInstallInline`. The latter two currently duplicate a 40 px, geometry-only follow rule (`ToolOverlayLog.tsx:139-143`, `SystemToolInstallInline.tsx:87-92`), which has the same misclassification problem in a simpler form. That is the DRY round.

### 5.2 States

| State | Meaning |
|---|---|
| `FOLLOWING` | Keep the true bottom in view. Every content or viewport resize pins. |
| `DETACHED` | The user is reading older content. Nothing moves it except the user or an explicit jump. |
| `SUSPENDED(prev)` | Hidden or zero-size (inactive tab via `content-visibility:hidden`, a minimized window, a collapsed `<details>`). All scroll and resize input is ignored; `prev` is restored when the element has size again. |

The initial state is `FOLLOWING`. `isOverflowing` stays a derived signal, updated from the same observations (it is not part of the follow decision).

### 5.3 Events and transitions

| From | Event | To / effect |
|---|---|---|
| FOLLOWING | `ContentResized` / `ViewportResized` (RO) | Pin, synchronously in the RO callback. Stay FOLLOWING. |
| FOLLOWING | `UserEscape` (§5.4) | → DETACHED. Cancel any pending pin in the same tick. |
| FOLLOWING | `Scroll` with no open user-intent window | Ignored. Record geometry only. |
| DETACHED | `ContentResized` / `ViewportResized` | Don't move. Update the "new below" indicator (§5.7). |
| DETACHED | `Scroll` inside a user-intent window, ending within `REATTACH_PX` of the true bottom | → FOLLOWING, and pin. |
| DETACHED | `Scroll` inside a user-intent window, within `NEAR_TOP_PX` of the top, **and** older history exists | Effect: request an older-history page. Capture the anchor, then restore it after the prepend. The state stays DETACHED. |
| any | `JumpToBottom` (typing, send, a queued turn starting, the "jump to latest" button) | → FOLLOWING, and pin. |
| any | `JumpToNode` (search, deep link) | → DETACHED, then scroll to the node. |
| any | `BecameZeroSize` (RO reports a 0-height viewport) | → SUSPENDED(current). |
| SUSPENDED(p) | `BecameSized` | → p. If p = FOLLOWING, pin. If DETACHED, leave `scrollTop` alone. |
| any | `Reset` (`/clear`, a new session in the same pane) | → FOLLOWING. |

Rules that are invariants, not just transitions:

- **I1.** Only `UserEscape`, `JumpToNode` and a user scroll that doesn't end at the bottom can leave FOLLOWING. No geometry reading, pagination step, anchor capture, clamp or anchoring adjustment can.
- **I2.** Pagination never changes the follow state, and it only runs from a user-attributed scroll. §2.1 is then impossible by construction.
- **I3.** There is exactly one function that writes `scrollTop` to the bottom, and one that restores a pagination anchor. Nothing else in the pane writes `scrollTop` on the transcript scroller.
- **I4.** Every state change emits one telemetry line: `[scroll-follow] pane=… FOLLOWING→DETACHED cause=wheel-up gap=…`. A stuck pane then always has a cause in `muxlog fe grep scroll-follow`. That closes the "only live, unreproducible" gap in §3.

### 5.4 What counts as the user

A **user-intent window** opens on one of the gestures below and closes `INTENT_WINDOW_MS` (proposed 250 ms) after the last gesture event. It is a timestamp, not a sticky boolean, so it cannot go stale the way `userScrollInput` does (§2.2 B1).

| Gesture | Opens a window | Escapes immediately |
|---|---|---|
| `wheel` with `deltaY < 0` whose target's nearest scrollable ancestor is this scroller | yes | **yes** (synchronously, as Hermes does) |
| `wheel` with `deltaY > 0` | yes | no (downward never escapes) |
| `touchmove` | yes | no (decided by the resulting scroll's direction) |
| `pointerdown` **on the scrollbar track or thumb** (hit-test: `offsetX > clientWidth`) | yes | no |
| Scroll keys (PageUp/Up/Home/Shift+Space) while focus is inside the transcript | yes | PageUp/Up/Home: yes |
| Text-selection drag inside the transcript (pointer held and a non-collapsed selection intersects the scroller) | — | yes, so auto-scroll never drags the selection away |

A `pointerdown` on content (focusing the pane, clicking a tool header, clicking a link) is **not** a gesture. It can't scroll this container. Treating it as one is §2.2 B1.

A `Scroll` event inside an open window becomes `UserScroll(prevTop, top, geometry)`:

- `top < prevTop` and `scrollHeight` unchanged since the last observation → `UserEscape` (if FOLLOWING).
- `distanceFromBottom ≤ REATTACH_PX` → re-attach (if DETACHED).

Outside a window, a scroll event only updates the last observed geometry.

### 5.5 Thresholds

- **`REATTACH_PX` = 24 px**, a single fixed value with no halving. It is small because re-attaching from DETACHED should mean "the user went to the bottom", and a large threshold re-locks a reader after a shrink (the Ensemblr finding). It is not 1 px because of the fractional scroll positions seen at non-100% zoom (use-stick-to-bottom PR #33).
- **The 200 px `STICK_TO_BOTTOM_THRESHOLD_PX` and the halving rule are deleted.** They only existed to guess detach from geometry, which the new design no longer does.
- **`NEAR_TOP_PX` stays at 50**, but it is only checked inside a user-intent window while DETACHED (I2).

### 5.6 Pinning and measurement

- **One `pinToBottom()`**, called only from ResizeObserver callbacks (content and viewport) and from `JumpToBottom`. It writes `scrollTop = scrollHeight - clientHeight`. It reads geometry only where layout is already clean, keeping the zero-forced-layout property #3599 bought and satisfying `tools/lint/check-input-handler-layout-reads.sh`.
- **The scroll handler reads live geometry only inside a user-intent window.** Otherwise it reuses the last RO observation. So an ordinary streaming frame costs the same as today's `pinnedGeometry` fast path, without the one-shot flag.
- **Observers attach through ref callbacks.** `observe` when an element mounts, `unobserve` in its cleanup, so a recreated streaming buffer or virtualizer can't escape observation (§2.2, second contributing factor).
- **`overflow-anchor`:** keep `auto` on the scroller for the DETACHED case (it keeps a reader's place when content above changes), and set `overflow-anchor: none` on the streaming buffer's last row while FOLLOWING, so anchoring never selects the growing row as the anchor. Anchoring adjustments are neutral anyway under I1. This only removes jitter.

### 5.7 "Jump to latest"

When DETACHED and content has grown below the viewport since detaching, show a small "↓ New activity" pill above the composer. Clicking it dispatches `JumpToBottom`. Every mainstream chat UI does this, and the ai-elements `Conversation` ships it by default. Visual design is out of scope for this spec. It must use the existing overlay and theme tokens.

## 6. DRY: what gets deleted or moved

| Today | After |
|---|---|
| `handleScrollNow`'s engage/disengage branches, `pendingProgrammaticScroll`, `userScrollInput`, `pinnedGeometry`, first-overflow force-pin (VL:969-1107) | `createFollowScroll` + reducer |
| Pagination block inside `handleScrollNow` (VL:1110-1201) | A reducer effect `LoadOlder`, emitted only per I2. The anchor restore stays in VL, called by the binding. |
| `captureHeadAnchor` turning `stickToBottom` off (`state.ts:156-160`) | Anchor capture no longer touches follow state. `stickToBottom` becomes a read-only projection of the controller's state, kept for existing readers. |
| `isNearBottom` + 200 px + halving (`anchor.ts:66-85`) | Deleted; replaced by `REATTACH_PX`. |
| Two ROs + no-RO fallback effect (VL:774-881) | The binding's two ROs. The no-RO fallback is deleted: CEF always has ResizeObserver, and jsdom tests use the existing RO shim. |
| `scrollToNode` disengage (VL:921) | `JumpToNode` event |
| `ToolOverlayLog.tsx:139-143` and `:217-227` follow logic | `createFollowScroll(logEl, [contentEl], { telemetry: false })` |
| `SystemToolInstallInline.tsx:87-92` follow logic | Same primitive. Its collapsed-`<details>` case is `SUSPENDED`. |

Out of scope: the content-resize contract's pending steps 4–6 (shrinks at the source), and the virtualization partition itself. Both stay as they are. With I1, a shrink or a migration can no longer *release* follow. Any flicker they cause is cosmetic and is tracked in their own specs.

## 7. Tests

1. **Reducer table tests** (`follow-controller.test.ts`): every row of §5.3, plus named regressions:
   - `A1`: FOLLOWING, then `ContentResized` leaving `scrollTop = 12`, then a `Scroll` near the top with no window. The result must be FOLLOWING and no `LoadOlder`.
   - `B1`: `pointerdown` on content, then 10 s pass, then a `Scroll` with a 400 px gap. The result must be FOLLOWING.
   - `B2`: a clamp scroll (`top` decreases, `scrollHeight` decreases, no window). The result must be FOLLOWING.
   - Wheel up by 1 px must give DETACHED. Wheel down never detaches.
   - Selection drag must give DETACHED.
   - Hidden → shown must restore the previous state and pin if it was FOLLOWING.
   - `/clear` must give FOLLOWING.
2. **Binding tests (jsdom)**, extending `AgentDocumentVirtualList.pin.test.tsx` and `.resize.test.tsx`: drive fake geometry and ResizeObserver callbacks. Assert exactly one writer of the bottom `scrollTop` and zero forced layout reads outside RO callbacks and user windows.
3. **Live soak (the missing piece in every earlier pass).** Extend `scripts/ui-screenshots/full-conversation-bench.mjs` with a `--follow-soak <minutes>` mode against a dev build over CDP:
   - Stream through the real pipeline into N panes. Interleave random non-scroll clicks in the transcript, tool expand/collapse, focus changes, tab switches, pane resizes, `/clear`, and a fresh session start.
   - Every second, assert that each pane still in FOLLOWING has `scrollHeight - clientHeight - scrollTop ≤ 1`.
   - Inject user wheel-ups to check that DETACHED holds and pagination fires only then.
   - Any `[scroll-follow]` transition whose cause isn't a synthetic user gesture fails the run.
   - Acceptance: 30 minutes with 3 panes and zero violations.

## 8. Rollout

1. **Phase 0: shipped in the same change as this spec.** All of it is in VL, plus a `hasOlderHistory` prop threaded from `agent-view.tsx` (`history.historyOffset() > 0`) through `AgentDocumentView`:
   - **§2.1 (bug A):** pagination runs only from a scroll inside the user-input window below (so never from our own pin, nor from a browser-made clamp or anchoring scroll), and only when `hasOlderHistory()` is true. This is invariant I2 applied inside today's code. The first version gated only on "not our own pin", which still let a browser-made near-top scroll in a pane with older history disengage (ReAgent P1 on #3652).
   - **§2.2 B1:** `userScrollInput` is replaced by a timestamped **user-input window** of `USER_INPUT_WINDOW_MS` = 250 ms. A `pointerdown` counts only when its target is the scroller element itself (the scrollbar; content clicks target a child), and a held scrollbar pointer keeps the window open until `pointerup`/`pointercancel`. Target identity is used instead of a `clientWidth` hit-test, so there is no layout read in the input handler.
   - **§2.2 B2:** a scroll batch with no user input never disengages, whatever its geometry. This goes further than the one-line gate first planned. It is invariant I1 applied inside today's code, and it is what lets the B1 window be short.

   Regression tests are in `AgentDocumentVirtualList.pin.test.tsx` ("scroll-follow Phase 0 regressions"). The six bug tests (A ×3, including the post-review "collapse + regrow with older history" case; B1 ×2; B2) fail against the code before them and pass with it, and the four still-works tests (a real user page-up, a scrollbar drag held to the top that still pages, a scrollbar drag, wheel away and back) pass on both. Two older tests in `.resize.test.tsx` that faked a user scroll-away with a bare `scroll` event now send a `wheel` gesture first, with a controlled clock.

   Not covered by Phase 0 (left to Phase 2): middle-click autoscroll mode (no gesture event while it scrolls), text-selection drags that scroll the transcript, and the threshold halving.
2. **Phase 0b: I4 telemetry, shipped ahead of Phase 1.**
   - **One line per change.** Every `stickToBottom` change logs one `[scroll-follow] pane=<7> <from>→<to> cause=<cause> [detail]` line, plus `mount <state>` when a pane mounts.
   - **Causes:** `user-scroll` (or `user-scroll:scrollbar`) with the gap, `user-scroll-to-bottom`, `reached-bottom`, `jump-to-bottom:<typing|sent|queued-turn>`, `jump-to-node`, `load-older`, `external`.
   - **Implementation:** every call site in VL goes through `transitionFollow()`. A safety-net effect logs any change that bypasses it as `cause=external`, so no disengage path can be silent again.
   - **Scroller attribute:** `.agent-document` carries `data-follow-state="following|detached"`.
   - **Tests:** "follow-state transition log (I4)" in `AgentDocumentVirtualList.pin.test.tsx`.
3. **Phase 1:** the pure reducer and its table tests. No behavior change.
4. **Phase 2:** `createFollowScroll` binding, wired into `AgentDocumentVirtualList` behind a setting, `agent:followcontroller` (default **on** in dev builds, off in release). Keep the old path for one release as a kill switch, the same pattern as `agent:turnscopedtail`.
5. **Phase 3:** move `ToolOverlayLog` and `SystemToolInstallInline` onto the primitive.
6. **Phase 4:** live soak passes; default on everywhere; delete the old path, `anchor.ts`'s threshold helpers, and the dead flags. The history catalog in §3 becomes this spec's status note.
7. **Phase 5:** the "↓ New activity" pill (§5.7).

## 9. Decisions (owner, 2026-09-24: "proceed" on the recommendations)

1. **Phase 0 first:** yes. It ships with this spec (§8).
2. **Turn anchoring (ChatGPT style, an `ANCHORED` state):** not in v1. It is a follow-up that fits the reducer.
3. **"New activity" pill:** later (Phase 5), not v1.
