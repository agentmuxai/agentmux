# Fix Plan: Agent Pane Loses Bottom-Pin on Pane Resize

**Date:** 2026-08-05
**Author:** AgentX (agent)
**Status:** Updated 2026-08-05 (same day) — H1 disproven and the scroll-pin
JS logic empirically validated via a new real-component test suite
(`AgentDocumentVirtualList.resize.test.tsx`, 5/5 passing against unmodified
`main`). See §3.5. No code fix has been made to `AgentDocumentVirtualList.tsx`
— the tests show that file's logic is already correct for every resize
ordering this plan could construct in jsdom. The remaining open question is
live-only (§7) and needs one manual repro in a running build, which this
agent cannot drive (no GUI automation tool available).

**Reported symptom:** Dragging a pane splitter (gutter resize) while the agent
pane is scrolled to the latest message causes it to lose its bottom-pinned
scroll position — the view no longer sits at the true bottom after the resize
settles.

**Related (read first — same bug class, twice before):**
- `docs/specs/SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md`
- `docs/specs/SPEC_WORKING_STATE_AND_SCROLL_FOLLOW_HARDENING_2026_07_27.md`
- `docs/specs/REPORT_AGENT_PANE_SCROLL_PIN_FLICKER_AUDIT_2026_07_30.md`
- `docs/specs/SPEC_SHIFT_DRAG_GROUP_RESIZE_2026_08_03.md` (most recent change to
  the resize hot path — 2 days before this report)

## 1. Current mechanism (verified against `main` @ `9692f8633`)

Three independent layers keep `.agent-document` (the scroll container,
`scrollRef` in `AgentDocumentVirtualList.tsx:130`) pinned to true bottom while
`viewState.stickToBottom()` is true:

1. **Content-signal pin effect** (`AgentDocumentVirtualList.tsx:435-454`) — re-scrolls
   when `nodes().length`, `layoutView().totalSize`, or `workingRowHeight()` change.
2. **RO #1 — viewport resize** (`AgentDocumentVirtualList.tsx:476-496`) — observes
   `scrollRef` itself; on any `clientHeight` change, if `stickToBottom()` is true,
   calls `scrollToTrueBottom()`. Added for the hidden-tab case, broadened on
   2026-07-27 specifically to cover *any* container clientHeight change including,
   per its own comment, "pane resize."
3. **RO #2 — content resize** (`AgentDocumentVirtualList.tsx:498-536`) — observes
   `virtualContainerRef` / `streamingBufferRef`; re-pins on any content-height
   change (e.g. late syntax-highlight reflow).

Pane-splitter drag is renderer-driven JS (not a host-native pointer-capture
loop — `docs/specs/SPEC_PANE_RESIZE_AND_FLOATER_DRAG_NATIVE_LOOP_2026_06_05.md`
§4 explicitly deferred moving gutter-resize to the host; it's still v1 JS).
`layoutResize.ts:157-260` (`onResizeMove`) runs on a `throttle(10, …)` pointer-move
handler (~100 Hz) and writes new sizes into a Solid signal via
`layoutGeometry.ts:236-241`'s `batch()`, which the leaf pane's wrapper div
consumes reactively for `width`/`height` (`tilelayout.scss:197` confirms no CSS
transition on width/height — the resize is synchronous per tick, only
`transform` is animated for drag-placeholder polish). This propagates through
plain flex layout (`blockframe` → `.agent-view` → `.agent-document-scroll-region
{flex:1; min-height:0}` → `.agent-document`), so `scrollRef`'s box genuinely
changes size on every tick, and RO #1 is a real `ResizeObserver` on that exact
element — **on paper this should already re-pin on every resize tick.**

**Conclusion of the static read:** the mechanism is not obviously broken.
Splitter-drag resize was never the *confirmed* repro for the two prior fixing
commits (`95fe278de` hidden-tab, `ba6da8b03` content-reflow) — it was
theorized to be covered by the same RO #1 broadening, but never explicitly
tested. This is a gap in verification, not a visible gap in the code, which is
why static analysis alone can't fully close this out — see §3.

## 2. Ranked hypotheses for where the theory breaks in practice

None of these are confirmed; §3 is the diagnostic step to pick between them
before writing a fix.

### H1 — Grow-clamp / re-pin ordering race — **DISPROVEN, see §3.5**

Original theory: when the agent pane's `clientHeight` **increases**, the
browser auto-clamps `scrollTop` down to the new, smaller
`scrollHeight - clientHeight` max as part of layout and fires an async native
`scroll` event for that clamp; if that clamp-triggered, rAF-coalesced
`handleScrollNow` call ran *before* RO #1's own `scrollToTrueBottom()` call for
the same tick, `isNearBottom()` might read false and disengage `stickToBottom`.

**This doesn't hold up under the actual math.** The browser's auto-clamp
always sets `scrollTop` to *exactly* `scrollHeight - newClientHeight` — i.e.
exactly the new max — because that's what "clamp" means. So immediately after
a grow-triggered clamp, `gap = maxScroll - scrollTop = 0` unconditionally,
regardless of which of RO #1 or the clamp's own `scroll` event is processed
first. `isNearBottom()` reads `true` at gap 0 no matter the threshold, so the
disengage branch is never reached from a pure grow. Confirmed empirically in
§3.5 with both orderings.

### H2 — Shrink case never gets a `scroll` event to correct a bad read from — **narrowed by §3.5**

When `clientHeight` **decreases**, there's no browser auto-clamp (the old
`scrollTop` is still a valid position — the gap to true bottom is simply now
bigger). The *only* thing that can notice and fix this is RO #1's
`scrollToTrueBottom()` call. §3.5's test proves that **if** RO #1 fires,
`scrollToTrueBottom()` unconditionally corrects the gap regardless of how large
the shrink was or how many shrink ticks landed in one frame — so this
hypothesis now reduces entirely to a single yes/no question: **does RO #1
actually fire for every live splitter-drag shrink tick?** That can't be
answered in jsdom (no ResizeObserver implementation to falsify against) — it
requires the live repro in §7 Q2.

### H3 — 100 Hz drag saturates rAF coalescing, final tick's `scrollToTrueBottom()` never lands — **disproven, see §3.5**

Theory: `onResizeMove` fires at up to 100 Hz; if several resize ticks (and
their RO #1 callbacks) land inside one animation frame, only the last one's
`scroll` event might get processed, silently dropping intermediate
corrections. §3.5's "rapid shrink ticks" test simulates exactly this — three
shrink ticks fired before a single rAF flush — and the final state is still
correct. `handleScrollNow` reads **live** `scrollRef` values on every call
(`AgentDocumentVirtualList.tsx:616`), not anything captured at event time, so
coalescing multiple `scroll` events into one `handleScrollNow` call is
harmless: whichever geometry is current when the (single, coalesced) call
runs is correct by construction.

### H4 — Visual-only: overlay/padding lag, not an actual scrollTop bug

`AgentWorkingRow`'s own `ResizeObserver` (`agent-view.tsx:1049-1052`, feeds
`--agent-working-row-height` → `.agent-document`'s `padding-bottom`) is a
**separate** RO from RO #1/RO #2. If it resolves in a later frame than RO #1's
correction, the working-row overlay could reposition after the scroll pin
lands, visually covering the tail message even though `scrollTop` is
technically correct. This would look identical to "lost the bottom pin" to a
user but requires a different fix (CSS/overlay sequencing, not scroll logic).
Only relevant if a mid-turn "Working…" row is visible during the resize —
worth ruling in/out early since it changes which file the fix belongs in.

### H5 — Zoom-factor interaction

`.agent-view` carries a live CSS `zoom` style (`agent-view.tsx:1469-1470`). RO
#1's callback reads `scrollRef.clientHeight` directly (not the RO entry's
`contentRect`), which the code's own comments assert is already unzoomed and
CDP-confirmed — lowest-likelihood hypothesis, included only for completeness
since it's the one variable that hasn't been re-verified specifically under a
resize (only under static zoom).

## 3. Diagnostic step (do this before writing any fix)

The code already ships a `[wave-scroll]` console instrumentation channel
(`AgentDocumentVirtualList.tsx:666-676`) that logs every engage/disengage
decision with `scrollTop`/`scrollHeight`/`clientHeight`/gap. This is enough to
distinguish H1–H3 without adding new logging:

1. `task dev`, open an agent pane with an active or recent conversation long
   enough to scroll, confirm it's pinned to bottom.
2. Open DevTools console, filter on `[wave-scroll]`.
3. Drag the pane's splitter to **grow** it a small amount, release, check the
   console:
   - A `disengage` log firing during/immediately after the drag with
     `wasProgrammatic` effectively false (i.e., not suppressed) confirms **H1**.
4. Reset (scroll back to bottom, confirm pinned), then drag the splitter to
   **shrink** the pane instead:
   - No `[wave-scroll]` log at all, and a visible gap at the bottom, confirms
     **H2** (RO #1 never fired, or fired with a stale `stickToBottom()` read).
   - Add a one-line temporary `console.info` inside RO #1's callback
     (`AgentDocumentVirtualList.tsx:477-493`) logging `h` and
     `props.viewState.stickToBottom()` on every call if the existing
     `[wave-scroll]` channel alone doesn't disambiguate.
5. Repeat both directions with a `AgentWorkingRow` actively visible (mid-turn)
   to rule H4 in or out independently of H1/H2.
6. Repeat once at non-1 zoom (`Ctrl` +/-) to rule out H5.

This takes under 15 minutes and turns 5 hypotheses into 1 confirmed root
cause before any code changes.

## 3.5. Empirical validation — real component, simulated resize (done)

Rather than only reasoning about ordering by hand, added
`frontend/app/view/agent/virtualization/AgentDocumentVirtualList.resize.test.tsx`.
It mounts the **actual, unmodified** `AgentDocumentVirtualList` component (not
a re-implementation of its logic) via `@solidjs/testing-library`, with small
deterministic fakes for the two things jsdom doesn't implement at all
(`ResizeObserver`, `requestAnimationFrame`) so the test can control exactly
when a resize is "observed" and when the resulting `scroll` event is
processed — including both possible orderings of the native scrollTop clamp
vs. the ResizeObserver notification, which is the crux of the (now-disproven)
H1 theory.

Five cases, run against current `main` with **zero changes to
`AgentDocumentVirtualList.tsx`**:

| Case | Result |
|---|---|
| Pinned, pane shrinks | ✅ stays pinned, `scrollTop` lands exactly at new true bottom |
| Pinned, pane grows, clamp-event processed before RO #1 | ✅ stays pinned |
| Pinned, pane grows, RO #1 processed before clamp-event | ✅ stays pinned |
| User scrolled away (not pinned), pane resizes | ✅ correctly does NOT force-scroll |
| Pinned, 3 shrink ticks land in one animation frame (simulated 100 Hz drag) | ✅ stays pinned, final size wins |

All 5 pass. `npx vitest run frontend/app/view/agent/virtualization/AgentDocumentVirtualList.resize.test.tsx` → 5/5.

**What this proves:** the scroll-pin *logic* in `AgentDocumentVirtualList.tsx`
is correct for every resize-event ordering this harness can construct — H1 is
disproven outright (§2), H3 is disproven (coalescing is provably harmless),
and H2 is narrowed from "is there a logic bug in the shrink path" down to "does
`ResizeObserver` actually fire during a live splitter drag" — a question this
test cannot answer, because the fake `ResizeObserver` only fires when the test
explicitly calls `triggerResize()`. It cannot tell you whether the *real*
browser/CEF ResizeObserver would have fired at that point; it only tells you
the code handles it correctly *if* it fires. That residual question is real
and needs a live repro (§7 Q2) — but per general web-platform behavior,
ResizeObserver notifications are guaranteed by spec to fire on any observed
element's box-size change regardless of cause, and this exact observer (RO #1
on `scrollRef`) is already relied on and confirmed working for two *other*
resize causes (hidden-tab reactivation, sibling-panel growth — both landed and
shipped). There's no mechanism by which the browser could distinguish "this
clientHeight change came from a splitter drag" from "this clientHeight change
came from a sibling panel growing" — both are just CSS reflows changing the
same element's box. This makes "RO #1 silently doesn't fire for splitter-drag
specifically" a low-probability explanation on its own merits, which is why
this plan does not recommend shipping a speculative code change against it
without the live confirmation in §7 Q2 first.

This test file is added as a permanent regression suite regardless of the
live-repro outcome — see §5.

## 4. Fix per hypothesis (historical — kept for the record; H1/H3 no longer apply)

- **H1 (grow-clamp race):** ~~moot — disproven by math and by §3.5's test.~~
- **H2 (shrink never corrected):** the only fix that makes sense *without*
  live confirmation that RO #1 fails to fire is unfounded speculation — don't
  make it. **If** §7 Q2's live repro shows RO #1 genuinely isn't firing for a
  splitter-drag shrink, the likely culprit is observer identity (a stale
  `scrollRef` closure from an unexpected remount) — audit whether
  `AgentDocumentVirtualList` or its parent ever remounts/re-keys when pane
  dimensions cross some threshold (no evidence found in static read of
  `agent-view.tsx`). If confirmed, the fix is either finding and removing the
  remount trigger, or adding a second, independent resize signal as a
  fallback (e.g. `window.addEventListener("resize", ...)` won't help — pane
  resize isn't a window resize — so the fallback would need to come from
  `layoutResize.ts`'s own `onResizeEnd` directly, similar to the old H3 idea
  below, scoped to "when RO #1 is confirmed unreliable," not preemptively).
- **H3 (rAF saturation on resize-end):** ~~moot — disproven by §3.5's rapid-tick test.~~
- **H4 (overlay lag):** merge the working-row height measurement into the same
  RO/timing path as RO #1, or make the overlay's positioning depend on the
  same `scrollToTrueBottom()` completion rather than its own independent RO,
  so the two can't visibly race. Still open — §3's manual repro (step 5) is
  the way to confirm or rule this out; nothing in §3.5 touches
  `AgentWorkingRow`.
- **H5 (zoom):** re-derive `clientHeight` from the RO entry with an explicit
  zoom-correction factor instead of trusting `scrollRef.clientHeight` mid-resize,
  if live testing shows a discrepancy. Still open, lowest priority.

With H1 and H3 disproven, the two remaining live candidates are H2 (RO #1 not
firing in practice — see §7 Q2) and H4 (a visual-only overlay-lag illusion,
independent of scrollTop). Do not implement a fix for either without the §3
manual repro confirming which one (if either) is real — there is currently no
evidence of a logic bug to fix.

## 5. Close the recurring-bug pattern, not just this instance

This is the **third** documented "stick-to-bottom silently breaks under a
trigger nobody wrote a test for" bug in two weeks (hidden-tab → sibling-panel
growth → content-reflow → now resize). Two structural gaps let this recur:

1. **Zero automated coverage** for `AgentDocumentVirtualList.tsx` — no
   `AgentDocumentVirtualList.test.tsx` exists anywhere in
   `frontend/app/view/agent/virtualization/`, unlike its sibling modules
   (`anchor.test.ts`, `state.test.ts`, `streaming-buffer.test.ts`, etc). The
   pure math (`isNearBottom`, `captureTopmostAnchor`) is tested; the DOM/RO
   wiring that actually drives the bug class is not.
2. **jsdom doesn't implement `ResizeObserver`** (`test/vitest-setup.ts` has no
   polyfill or mock for it) — so even a naive test attempt would silently no-op
   on the exact code path this bug lives in.

**Done:** `AgentDocumentVirtualList.resize.test.tsx` (§3.5) closes gap 2 with a
local `ResizeObserver`/`requestAnimationFrame` fake and covers pinned+grow,
pinned+shrink, not-pinned+resize, and rapid-multi-tick+resize. It does not yet
cover "pinned + resize while a `workingRowHeight` overlay is present" (H4) —
left for whoever picks up the H4/live-repro follow-up, since it requires
deciding H4's fix shape first. Gap 1 (zero coverage) is closed for this file;
gap 2 (no shared RO polyfill) is closed locally in this one test file — if a
second test in this directory needs `ResizeObserver`, extract the fake into
`test/vitest-setup.ts` or a shared test helper instead of duplicating it.

This is the same "whack-a-mole vs. systemic fix" tradeoff
`SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md` §3.3 already
flagged and deferred — three fixes later is the point where the deferred cost
has come due.

## 6. Suggested phases

| Phase | Scope | Status |
|---|---|---|
| 1 | Diagnostic pass (§3) — confirm root cause, no code changes | Superseded by §3.5 (jsdom-level) — H1/H3 resolved that way; §3's live steps still needed for H2/H4 |
| 2 | Add `ResizeObserver`/rAF test fakes + `AgentDocumentVirtualList.resize.test.tsx` regression suite (§5) | **Done** — 5/5 passing |
| 3 | Live manual repro (§3 steps 3-6, focused on H2 + H4 now that H1/H3 are closed) | **Not done — needs a human with the running app; no GUI automation tool available to this agent** |
| 4 | Targeted fix, only if step 3 finds a real live defect (§4) | Blocked on phase 3 |

## 7. Open questions for whoever picks this up

1. ~~Does `computeGroupResizeSizes`'s Shift+drag path (`layoutResize.ts:62-130`,
   landed 2026-08-03) resize the agent pane as a *passive* absorber
   differently enough from the *driven* two-node case to matter for RO
   firing?~~ **Resolved:** no. Read `layoutGeometry.ts:333-352`
   (`updateTreeHelper`'s per-child loop) — every child's `rect`/`transform` is
   unconditionally rewritten into `additionalPropsMap` on every `updateTree`
   pass, driven or passive, group-resize or plain two-node resize. There is no
   code path where a passive sibling's DOM box is left stale while its
   logical size changed. Whatever RO #1 does for the driven node, it does
   identically for every passive absorber.
2. **The one question this plan cannot close without a human:** does
   `ResizeObserver` on `.agent-document` actually fire on every real
   splitter-drag tick in the running CEF app? §3.5 shows the *logic* is
   correct if it fires; general web-platform semantics say it must fire (spec
   guarantee, no way for the browser to distinguish resize causes); and this
   exact observer already works for two shipped, confirmed-live sibling
   scenarios. All of that points toward "probably fires correctly" — but none
   of it is a substitute for actually watching it happen. **Ask:** open an
   agent pane with scrollback, confirm pinned to bottom, open DevTools
   console filtered on `[wave-scroll]`, drag the pane's splitter smaller by a
   noticeable amount, and report back either (a) the pane stayed pinned — bug
   is stale-build or environment-specific, close this out — or (b) a visible
   gap appeared at the bottom with no `[wave-scroll]` disengage log — confirms
   H2, come back and audit RO #1's observer identity per §4. Also worth a
   quick check with a `AgentWorkingRow` visible mid-turn, to separate H2 from
   H4 (§3 step 5).
