# Agent pane: one-way flow while following

**Status:** proposed — Phase 0 (recorder + soak) built on `agento/one-way-flow-impl`; §3–§5 revised after an
adversarial review of the first draft (§9). Builds on, and does not replace, the follow state machine
(`SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24.md`; its Phase 0/0b shipped in #3652/#3658, Phases 1–5 open).
**Date:** 2026-10-07.
**Requested by:** repo owner (asafebgi): "what i continue to see in the agent pane is the overshoot followed by
correction. That model for new conversation content may be no good, it may need a rethink. the UX we want is a simple
1-way flow of text, every message goes in with no scroll back. We still get scrollback at different times, like when
tool call previews are processing … can we design a one and for all robust solution for this?"
**Author:** AgentO.
**Supersedes, once implemented:** the glide (`glideContent`, `takeGlideOffset`), the shrink hold (`holdBufferHeight`,
`releaseBufferHold`, `easeOutBufferHold`, `HOLD_MS`, `HOLD_RELEASE_MS`) and the row-enter rise in
`frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx` and `styles/_document.scss`, all from
#4178/#4217/#4322, and the shrink half of `resize-contract.ts`'s height animation while following.
**Related:** `docs/reports/REPORT_AGENT_PANE_SCROLL_LAYOUT_ARCHITECTURE_2026_10_04.md` (mechanism A/B, the tremor RCA),
`SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md`, `PLAN_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE_2026_06_16.md`,
`docs/reports/REPORT_AGENT_PANE_ROW_ENTER_MOTION_2026_10_01.md`, `SPEC_TOOL_PREVIEW_HEIGHT_THIRD_AND_FOLLOW_LATEST_2026_09_25.md`.

"VL" is `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx`. Line numbers are against `main` at
`22857409f`.

---

## 1. The goal, stated as something a test can check

While the pane is **FOLLOWING** and the user is not touching it:

- **V1 — one direction.** No row that is visible in two consecutive painted frames is lower on screen in the second
  frame than in the first (tolerance 0.5 CSS px). Text only ever moves up.
- **V2 — no overshoot.** The viewport never shows blank space below the last row that is later taken back. Every
  scroll position the follower writes is a real resting position; nothing is ever "corrected".

Everything below exists to make V1 and V2 true **by construction**, and §6 checks them on every frame in a soak run.

User-initiated changes are exempt: the user's own scroll, wheel, scrollbar drag or text-selection drag; a pane resize or
zoom change; the composer growing or shrinking as they type; sending a message; collapsing, expanding or pinning a tool;
answering a question or decision; `/clear`; switching to the pane's tab. Exempt means V1/V2 are not checked for those
frames, not that the code stops caring: it still never moves content down on its own.

## 2. Why the current model can't meet the goal

The two symptoms come from mechanisms that were added on purpose. Tuning them will not help; they are the wrong model.

### 2.1 Overshoot then correction: we draw content where it isn't

The glide (#4178) moves already-pinned content down with a `translateY(+Δ)` and eases it back to 0. New rows also
enter at `translateY(8px)` (`_document.scss:281-293`). A transform extends the scroller's scrollable overflow, so any
pin that reads `scrollHeight` while one is live lands **past** the real bottom, and the browser pulls it back as the
transform decays. That is the "prediction that corrects itself".

VL's content ResizeObserver knows this and strips the running glide before it pins (VL:1076-1088). The other pin
paths don't:

- the viewport ResizeObserver (VL:1013-1041), which fires whenever the working row, ActivityDock, a panel or the
  composer changes height, which is constantly during tool runs;
- `jumpToBottom` (VL:1105-1109), while typing (`agent-view.tsx:1467-1469`, at most once per frame via
  `AgentFooter.tsx:765-808`), on send and on a queued turn;
- the first-overflow forced pin (VL:1255-1288).

Even the content RO can't strip the row-enter rise, which is CSS, so every new agent row overshoots by about 6 px
(8 px rise, 2 px bottom padding) and settles over 180 ms.

Two smaller sources of the same look:

- a glide fires on growth that was **already** compensated (above the fold, or a mid-view preview growing), so the
  content below drops by Δ and rises again;
- a head-row remeasure can reach the pin one frame late (inferred, not measured). The measure RO is deeper in the
  tree than the content RO, so the browser's depth rule pushes the pin to the next frame. Native scroll anchoring
  covers it when its anchor is a buffer row, not when it is a head row, whose `translateY` (VL:1641) changes.

### 2.2 Scroll-back during tool previews: we let visible rows shrink, then hide it on a timer

A tool preview collapsing to its result is 81% of recorded shrinks (VL:414-415). The browser clamps `scrollTop` in the
same layout pass, so a shrink can't be eased (contract §2). The shrink hold answers with `min-height` for 350 ms, then
eases the content **down** over 160 ms (VL:413-478). The next row arrives a median 2–3.7 s after a tool result and
within 350 ms in at most 1% of cases (`REPORT_AGENT_PANE_ROW_ENTER_MOTION`), so the hold almost always turns a jump into
"gap, then slide back". Turns of more than 40 nodes also snap the hold off on almost every new node (VL:896-902,
`streaming-buffer.ts:115`).

Two more scroll-backs have nothing to do with rows:

- **The viewport grows mid-flow.** At turn end the working row leaves, and an ActivityDock row or a question panel
  closes. The scroller gets taller, the browser clamps, and the whole conversation slides down. Nothing eases this.
- **The preview box animates its own height down** (`resize-contract.ts:153-181`, `ToolOverlayLog.tsx:399-427`), then
  snaps again when an async renderer finishes about 150 ms later.

### 2.3 The common root

Both symptoms come from one choice: **let the layout change, then repair what the reader sees with motion.** The glide
repairs a jump by drawing content where it isn't. The hold repairs a shrink by delaying it. Each repair has edge cases
that leak, and every pass since April has added another repair. The fix is the reverse: **only allow layout changes
that are already one-way while following, and get motion only from the scroll position, which the browser can never
let us overshoot.**

## 3. The model

Four rules. The first draft of this section (per-row height floors, an eased follower that only ratcheted
`scrollTop` forward, overlay chrome, several ResizeObservers) did not survive review; §9 says why. What replaced it is
smaller and states V1 directly instead of approximating it.

### R1. No motion that isn't layout

While FOLLOWING, nothing in the transcript scroller may run a transform/translate **animation or transition**, or a
height animation that **shrinks**. Static transforms (the head rows' `translateY` positions, VL:1641) and growing
height animations are fine: they are layout the rest of the model sees. The glide and the shrink hold are not used
(the hold's ease-down is a transform animation; its `min-height` is replaced by R3). New rows fade in with opacity
only; the 8 px rise is dropped, because a rise extends the scroll range and makes every pin land past the bottom.

### R2. Visible rows never move down: compensate, don't predict

In one ResizeObserver callback (R4), with layout clean, the binding knows where every row that was visible at the last
observation is on screen now. Take the **largest downward move** of any of them and scroll forward by exactly that,
immediately, before paint. Every row then sits at or above where it was: V1 holds whatever the cause was:

| Cause | What the rows did | Compensation |
|---|---|---|
| A visible row or one above the viewport grew (preview opens, image, highlight swap, a head row remeasured) | Rows below the growth moved down | Scroll forward by the growth |
| A visible row shrank while at the bottom | The browser clamped `scrollTop`; rows above the shrink moved down | Scroll forward by the clamp; R3 makes room for it |
| The viewport grew (working row left, a panel closed) | The browser clamped; everything moved down | The same |
| A row was removed, replaced under a new id, or migrated into the head with a different height | Whatever moved, moved down by some amount | The same |
| Something shrank wholly above the viewport | Nothing visible moved down | Nothing to do (the clamp is invisible) |
| New content appended below the last visible row | Nothing visible moved | Nothing; the follower (§4) eases toward the new bottom |

This is what native scroll anchoring does for one anchor, applied to every visible row and to shrinks. So while
FOLLOWING the scroller gets `overflow-anchor: none`, and the browser and the app never both correct. DETACHED keeps
`auto`, so a reader's place is kept by the browser as today.

Between observations the binding updates its record of row positions by whatever it scrolled, so its own writes are
never mistaken for movement. A scroll the user made re-baselines the record instead.

### R3. One bottom spacer makes room, and new content fills it

A compensating scroll needs room below it. When the content got shorter or the viewport taller, the position R2
wants is past the end. A spacer element after the streaming buffer provides exactly that room:

```
spacer' = max(0, spacer + ceil(wanted + clientHeight − scrollHeight))
```

- It is the only thing that grows when content shrinks. Rows below a shrink move **up** into the space (allowed). The
  blank appears at the bottom of the pane, where the next content lands.
- New content fills it. Each observation recomputes it from the same formula, so growth reduces it first and nothing
  moves until it is used up.
- It shrinks only when it is below the viewport. As the reader scrolls up, the formula with the reader's `scrollTop`
  needs less room, and the spare part is off-screen when it goes. `/clear`, a width or zoom change, and a pane reset
  set it to 0.
- It is not a row. It is not measured by the head's measure RO or the tail's height cache, so it never leaks into the
  layout slice. It is not observed, so writing its height does not re-trigger the observer.
- It covers what the first draft needed three mechanisms for: per-row floors, the overlay chrome for the working row
  and `AgentBottomPanels`, and floors for removed or replaced nodes.

### R4. One observer, one writer, user input suspends it

- **One ResizeObserver** for the one-way path. It observes the scroller (viewport), the head container, the streaming
  buffer and every mounted row (head and buffer). It is created after the head's measure RO and the tail RO, so in a
  round where rows resized it runs after their callbacks have updated the slice, and reads geometry that includes
  them. In its callback, in order: re-read geometry, compensate (R2), resize the spacer (R3), let the follower (§4)
  continue.
- **One writer.** While FOLLOWING, only this callback and the follower's frames write `scrollTop`. The content RO,
  viewport RO, first-overflow pin, glide and hold do nothing on the one-way path. `jumpToBottom` engages FOLLOWING and
  asks the follower to snap to the bottom.
- **The user always wins.** While the user-input window is open (wheel anywhere in the pane, including the preview
  wheel relay `scroll-handoff.ts:187`; a scrollbar drag; a pointer held on the content, which covers selection drags;
  scroll keys) the binding neither compensates nor follows; it only re-baselines. A user scroll that moves the
  transcript up while FOLLOWING detaches at once, whatever the distance, so a small wheel-up is not pulled back.
- The browser's "ResizeObserver loop completed with undelivered notifications" can still fire when a row
  measurement grows the head container in the same round. That path exists today (measure RO → head height → content
  RO). The one-way observer does not add a new one: the spacer is not observed.

## 4. The follower

The follower only ever handles the part R2 doesn't: content that arrived **below** everything visible. It moves
`scrollTop` toward the bottom over a few frames, forward only:

```
next = gap ≤ 1 px            → at rest
       reduced motion or gap > 0.75 × clientHeight → bottom, at once
       otherwise              → pos + max(1, gap × (1 − e^(−dt / 60 ms))), never past the bottom
```

- **No overshoot.** The target is the bottom as last measured with layout clean, which can only be at or above the
  real bottom, and the browser clamps a write to the real bottom anyway. V2 holds for every write.
- **No layout reads in frames.** The follower's frame step only writes; the target and position come from the
  observer callback. Each write is marked as our own scroll with its geometry (`pinnedGeometry`), so `handleScrollNow`
  reads no layout for it and logs nothing.
- **At rest** is `gap < 1 px` or a write that did not move `scrollTop` (fractional zoom: integer `scrollHeight`,
  fractional `scrollTop`). It stops its frame loop there and when the pane is hidden or detached.
- **Why not snap everything?** Snapping (scroll to the bottom in the observer) also satisfies V1/V2 and is simpler.
  Easing only the appended part keeps the smooth arrival the owner asked for on 10-01 without the glide's transform.
  `REPORT_AGENT_PANE_ROW_ENTER_MOTION_2026_10_01.md` flagged an animated scroll as risky because of the scroll events
  it fires; marking each write as our own with its geometry is the answer. Reduced motion snaps.

## 5. Rollout

Everything new sits behind **`agent:onewayflow`** (default on; `false` restores the old path, the kill switch), the
same pattern as `agent:turnscopedtail`. Read once at pane mount. Nothing merges before the owner has smoke-tested a
build of it.

| Phase | What | Exit criterion |
|---|---|---|
| **0. Measure** | The recorder (`frontend/app/view/agent/scroll/one-way-recorder.ts`, opt-in in every build: `__agentmuxOneWay.enable()`) and the soak (`scripts/ui-screenshots/one-way-soak.mjs`). Baseline current main. | A baseline count of V1/V2 violations per minute of tool-heavy streaming, with causes. |
| **1. The one-way path** | R1–R4 and the follower, behind the flag: one observer, compensation, the spacer, the follower, `overflow-anchor: none` while following, no rise, no glide/hold/old pins on this path. | The soak reports zero violations not explained by an exempt user action. |
| **2. Sources** | Fewer things to compensate: a Write/Bash preview completes inside a box that doesn't shrink while visible; `ToolOverlayLog` and `SystemToolInstallInline` share the user-intent window and log format (09-24 Phase 3). | Spacer time and size per tool call drop in the soak. |
| **3. Default on** | A 30-minute, 3-pane soak with zero violations; owner sign-off on a live run; flip the default; delete the old path, the glide/hold constants and the 200 px threshold. Update this spec and 09-24 to `implemented`. | Owner sign-off. |

## 6. Verification

- **Unit (pure).** The follower step (monotonic, never past the target, snap above the threshold, reduced motion,
  at-rest); the compensation (largest downward move over the rows visible in both, ignoring new and removed rows); the
  spacer formula (grows on shrink and viewport growth, filled by growth, never negative, shrinks only below the
  viewport). The recorder's frame check (built).
- **jsdom binding.** With fake geometry and ResizeObserver callbacks: one writer of `scrollTop` while FOLLOWING; a
  shrink at the bottom grows the spacer and keeps `scrollTop`; growth above the viewport scrolls forward by exactly the
  growth; user input suspends both; a wheel-up while following detaches.
- **Live soak (acceptance).** `node scripts/ui-screenshots/one-way-soak.mjs --strict` against a dev build: scripted
  tool-heavy turns (Bash, Write, parallel Reads, Edit, code blocks, turn end) through the real stream pipeline, with
  the recorder checking every painted frame. Run on the base build for the baseline and with the flag on for the
  change.

The recorder samples once per frame, right after the frame is painted (a high-priority task queued from
`requestAnimationFrame`). That is an approximation: a task that runs first could show a state that was never
painted, so a violation is a lead to confirm, and a clean run is the claim.

## 7. What this trades away (decisions for the owner)

1. **Blank space at the bottom until new content fills it.** A tool whose result is shorter than its preview, or a
   panel closing below the transcript, leaves its height as blank space at the bottom of the pane. The next content
   fills it with nothing moving; scrolling up releases it. This replaces today's "gap, then slide back". Phase 2
   reduces how often it happens. **Recommended:** accept; it is the only way a visible shrink can avoid moving content
   down.
2. **Motion style.** Appended content arrives by the follower's short scroll (§4) instead of the glide; rows fade in
   without the rise. The alternative is an instant snap. **Recommended:** the follower.

## 8. What this doesn't cover

- Mechanism B of the 10-04 report (head estimates corrected while the user scrolls history) is a DETACHED problem.
  It is tracked there; R2's compensation is only active while FOLLOWING.
- The ANCHORED (ChatGPT-style turn anchoring) state and the "↓ New activity" pill stay where 09-24 put them.

## 9. Review of the first draft

An adversarial review (2026-10-07) of the first draft found:

- **The ratchet could not keep V1.** A follower that only moves `scrollTop` forward does nothing for growth above the
  last visible row (rows below it move down) and, as written, eased back from a clamped position instead of restoring
  it. R2 (compensate the measured move of every visible row) replaces it.
- **Several observers could not be ordered.** Floors in one observer and the pin in another raced, and writes to an
  observed element at a shallower depth re-triggered the loop error. R4 uses one observer, and the spacer is not
  observed.
- **Per-row floors leaked into measurement** (the tail height cache and the head's measure RO read the floored height)
  and missed removed or replaced rows and forced migrations. R3's spacer is not a row.
- **The overlay chrome reversed `SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01.md`** and missed most of
  `AgentBottomPanels`. The spacer handles a taller viewport the same way as a shrink.
- **"At rest" was unreachable at fractional zoom**, other writers (the preview wheel relay, scrollbar and selection
  drags) would have been fought, a small wheel-up would have been pulled back, and the exemption list was incomplete.
  §1, §4 and R4 now cover these.
- **R1 contradicted the code** (head rows are placed with a static transform). R1 now forbids animated transforms and
  shrinking height animations only.
