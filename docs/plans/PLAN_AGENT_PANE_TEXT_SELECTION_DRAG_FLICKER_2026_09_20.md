# PLAN — Agent pane text selection drops/flickers when dragging left across collapsible header rows

- **Status:** Implemented (PR pending)
- **Date:** 2026-09-20
- **Reported by:** user (this machine)
- **Scope:** Originally agent-pane document rendering (`frontend/app/view/agent/**`); widened
  during implementation to the shared `element/tooltip.tsx` `Tooltip` primitive as well (see §8) —
  the same hover-triggered-Portal mechanism, used across many more surfaces app-wide (pane tab
  strips, widget-bar buttons, etc.), not just agent-pane message rows. Does not touch typing-
  latency/WS-fairness work or the status-bar "Instance" field — both tracked separately.

## 1. Symptom

In an agent pane, start a text-selection drag inside message/markdown content, then move the
mouse left (toward or past the pane's left margin) and sweep it vertically while still holding
the mouse button down. The selection intermittently collapses/flickers instead of staying
selected and continuously extending.

**Desired behavior**, per the user: match an ordinary browser text input — e.g. Chrome's address
bar. There, once you start a drag-select and keep the button down, you can move the mouse
anywhere, including far outside the original element/window bounds, and the selection just keeps
extending (or at minimum never spontaneously drops) until mouseup. Today's agent-pane behavior
does not match that.

## 2. Confirmed facts

- Many collapsible header/summary rows in conversation content (`.agent-message-summary`,
  `.agent-jekt-summary` in `frontend/app/view/agent/styles/_document-nodes.scss`) are
  `user-select: none`.
- Those same rows attach `onMouseEnter`/`onMouseLeave` handlers (`handlePeekEnter`/
  `handlePeekLeave`, sourced from `useNodePeek()`) that drive a conditionally-rendered
  `<PeekOverlay>` (`frontend/app/view/agent/virtualization/DocumentRow.tsx`, ~lines 202-581).
- SolidJS's keyed `<Show>`/`<For>` reconciliation replaces/re-parents real DOM nodes when their
  controlling signal flips — inherent framework behavior, not a bug by itself. Replacing a DOM
  node that is inside, or is an endpoint of, an active browser `Selection` Range collapses that
  Range — standard DOM/Selection spec behavior, not AgentMux-specific.
- The document list is hybrid-virtualized: older nodes are absolute-positioned and re-measured
  via `ResizeObserver`; the most recent ~50 nodes (the "streaming buffer") are unvirtualized, in
  normal flow.

## 3. Disproven

- An isolated repro (`repro2/selection-repro.html` + Playwright) mirroring the real
  `.agent-message-summary` (`user-select: none`) / `.agent-markdown-block`
  (`user-select: text`) structure, with **no** hover/peek logic attached: dragging a selection
  across multiple rows extended continuously with no collapse. This rules out "`user-select: none`
  sibling rows, by themselves" as a sufficient cause.

## 4. Leading hypothesis (unconfirmed — live CDP verification was inconclusive)

Mouse movement during the drag crosses a header/summary row's bounding box →
`handlePeekEnter`/`handlePeekLeave` fires → the `isPeeking()` signal flips → the `<Show>` gating
`<PeekOverlay>` mounts/unmounts → SolidJS inserts/removes/reorders DOM nodes in that vicinity →
if the current Selection's anchor or focus node is inside, adjacent to, or positionally
dependent on a re-parented node (or the overlay briefly intercepts hit-testing under the
cursor), the browser's native selection-extend logic collapses the range, or silently drops the
extend for that `mousemove` tick. Swept vertically, the cursor crosses several header-row
boundaries, so it flickers repeatedly. It's worse to the left of the text because that's where
each header row's interactive chrome (icons, label, timestamp, expand affordance) lives —
dragging within the markdown-text column itself mostly avoids those bounding boxes.

This lines up with the user's Chrome-address-bar framing: a plain `<input>` never mutates its own
DOM mid-drag, so nothing can invalidate the Range. The agent pane does — hover-driven conditional
DOM churn is actively happening under the cursor during exactly the gesture that fails.

**Not yet confirmed live.** CDP-driven drag simulation against the running app was inconclusive:
`document.elementFromPoint` at the computed coordinates returned an unrelated overlay element
(`agent-tool-overlay-log`), likely a coordinate-mapping issue specific to this multi-pane/tab/zoom
CEF embedder, not a refutation of the hypothesis itself.

## 5. Recommended fix approach (ranked)

1. **Suppress hover/peek activation while a text-selection drag is in progress.** Track whether
   the primary mouse button is currently held (e.g. a module-level or context signal set on
   `pointerdown`/cleared on `pointerup`, or check `event.buttons` inside the handler), and make
   `handlePeekEnter` a no-op while it's held. Directly targets the mechanism in the leading
   hypothesis without redesigning peek or virtualization. Small diff, low risk, no visible
   behavior change outside an active drag-select.
2. If (1) doesn't fully resolve it, also suppress `handlePeekLeave`-driven unmount while
   dragging — freeze the overlay's last state until mouseup, in case unmount-during-drag (not
   mount) is the actual trigger.
3. If neither resolves it, broaden the audit to other mouseenter/mouseleave-driven conditional
   renders in the row tree beyond `useNodePeek` (grep `DocumentRow.tsx` and siblings for other
   `<Show>`s gated by hover signals).
4. **Do not touch `user-select` CSS** — disproven as the cause; adjusting it only relocates where
   dragging is/isn't possible, it doesn't fix DOM-churn-invalidates-Range.
5. **Do not manually re-derive/restore the Selection Range** after every mutation (e.g.
   recomputing text offsets post-patch) — fragile and unnecessary if the actual trigger
   (hover-driven remount during drag) is suppressed at the source.

## 6. Verification plan

- **Manual:** `task dev`, open an agent pane with several collapsible header rows and streamed
  markdown content. Start a drag-select in markdown text, drag left past the header rows'
  bounding boxes, sweep vertically through several header rows while holding the button —
  selection should extend continuously, matching the Chrome-address-bar behavior described above.
- **Automated (new):** a Playwright/CDP test against the real running app (not the isolated
  repro) that starts a selection in a markdown block, dispatches a `mousemove` sequence sweeping
  through a header row's bounding box with the button held, and asserts
  `window.getSelection().toString()` length is monotonically non-decreasing (never drops
  mid-drag). Before asserting on selection state, first confirm via `elementFromPoint`/
  `getBoundingClientRect` that dispatched coordinates land on the intended elements — this closes
  the gap the earlier live-CDP attempts hit (coordinate/element mismatches in this embedder).
- **Regression check:** confirm hover-peek still activates normally on a plain hover (no button
  held) after the fix — the gate must be conditioned specifically on "primary button held," not
  disable peek generally.

## 7. Explicitly out of scope

- Typing-latency / cross-pane WS-fairness work (tracked separately; user has not yet chosen a
  starting point between bench-first and backend-fairness-first).
- The status-bar "Instance" field (tracked separately, awaiting user's answer).
- Any change to `useNodePeek()`'s hover-intent timing/debounce beyond the button-held gate.

## 8. What was actually implemented

Fix approach #1+#2 (combined, not staged) plus a proactive #3, done in one pass rather than
incrementally, since the mechanism (mount/unmount racing an active drag) is identical everywhere
it appears:

- **`frontend/app/util/pointer-drag-state.ts`** (new) — a framework-agnostic, module-level
  singleton (mirroring the existing `frontend/app/util/submenu-hover.ts` pattern) exposing
  `isPrimaryButtonDown()`. Tracked via `window`-level `pointerdown`/`pointerup`/`pointercancel`
  listeners registered with `capture: true` (so they observe every button transition regardless of
  which element the event lands on or whether it stops propagation), plus a `blur` listener to
  reset state if the mouse is released outside the window.
- **`frontend/app/view/agent/hooks/useNodePeek.ts`** — only `handlePeekEnter` (mount) no-ops
  while `isPrimaryButtonDown()` is true; `handlePeekLeave` (unmount) is never gated. Original
  version gated both ("freeze whatever was showing") — **reagentx P1 on PR #3470** caught that
  this left a peek stuck open indefinitely whenever its row's mouseleave fired mid-drag but the
  button was released somewhere else afterward, since no further mouseenter/mouseleave would ever
  fire for a row the cursor had already left. Unmounting doesn't reintroduce the hit-testing
  problem this fix targets (removing an obstruction from under the cursor can't break selection
  the way adding one does), so ungating leave entirely closes the stuck-open hole with no
  trade-off against the original bug. Also re-checks `isPrimaryButtonDown()` again when the
  enter-delay timer fires, not just at the initial call, since the button can go down during that
  window.
- **`frontend/app/element/tooltip.tsx`** (`Tooltip`, the shared hover-triggered Portal-rendered
  tooltip primitive used across the app — pane tab strips, widget-bar buttons, etc., not just
  agent-pane rows) — same enter-only gate, same reasoning, same reagentx finding (identical
  stuck-open defect, fixed the same way). Added in response to the user separately reporting the
  identical flicker on the browser pane's address-bar input; investigation did not find a
  hover-driven Portal mechanism specifically wired to that `<input>` itself (it is a plain native
  text field with no nearby overlay logic in `browser-nav-bar.tsx`), but `Tooltip` is a
  widely-reused primitive that likely sits near enough to many text inputs app-wide (including via
  its use on nearby toolbar buttons) that gating it the same way generalizes the fix broadly. If
  the address-bar case persists after this fix, it has a different root cause — see the follow-up
  note below.
- Tests: `frontend/app/util/pointer-drag-state.test.ts` (new), `useNodePeek.test.ts` (4 new cases:
  gated enter, leave still closes while held, enter-delay button-down race, resumes after
  release), `frontend/app/element/tooltip.test.tsx` (new, 4 cases covering the same behaviors plus
  the ungated baseline).

**Follow-up, not yet done:** if the browser-pane address-bar flicker is still reproducible after
this change, it needs its own targeted investigation — the concrete mechanism there (if any)
was not identified in this pass, unlike the agent-pane/`PeekOverlay` case where the causal chain
is well-evidenced. Candidates not yet ruled out: OS-level pane-HWND compositing/mouse-capture
interference near pane boundaries (this app's CEF panes are separate native HWNDs), or something
triggered by the `main_window_focus` IPC call `browser-nav-bar.tsx`'s address-bar `onMouseDown`
fires on every click.
