# Agent pane: one-way flow while following

**Status:** proposed — nothing built. Builds on, and does not replace, the follow state machine
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

User-initiated changes are exempt: the user's own scroll, a pane resize, a zoom change, the composer growing or
shrinking as they type, and sending a message.

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
- `jumpToBottom` (VL:1105-1109), on **every keystroke** (`agent-view.tsx:1468-1470`), on send and on a queued turn;
- the first-overflow forced pin (VL:1255-1288).

Even the content RO can't strip the row-enter rise, which is CSS, so every new agent row overshoots by about 6 px
(8 px rise, 2 px bottom padding) and settles over 180 ms.

Two smaller sources of the same look:

- a glide fires on growth that was **already** compensated (above the fold, or a mid-view preview growing), so the
  content below drops by Δ and rises again;
- a head-row remeasure reaches the pin one frame late. The measure RO is deeper in the tree than the content RO, so the
  browser's depth rule pushes the pin to the next frame, and one frame paints the content pushed down.

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

Four rules. Each one closes a class of bug outright, without trying to patch individual paths.

### R1. Motion comes only from `scrollTop`, never from transforms

While FOLLOWING, nothing in the transcript scroller may carry a transform, a `translate`, or a height animation that
changes its scrollable overflow. Smooth arrival is produced by the **follower** (§4) moving `scrollTop` toward the
bottom.

- **Overshoot becomes impossible.** The browser clamps `scrollTop` to `scrollHeight − clientHeight`, so the follower
  cannot write a position past the real bottom. This guarantees V2 for every pin path, with no per-path compensation
  to forget.
- The glide and the hold-release ease are deleted, not fixed.
- **Row entry becomes opacity only** (180 ms fade, no rise). The upward motion a reader sees comes from the follower,
  the same as the rest of the conversation. If the rise is wanted back, it must be clipped so it can't add overflow:
  `overflow: clip` on the row wrapper, so the row's own box clips it.

### R2. Visible rows are grow-only while following

A row that intersects the viewport, or is below it, may grow but may not shrink. A shrink is turned into a **floor**:
the row wrapper keeps `min-height` at its high-water mark, and the gap shows **inside that row**, under the tool card,
not as a hole at the bottom of the pane. A floor is released **only where the release is invisible**:

| Situation | Release |
|---|---|
| The row's bottom is above the viewport top, and the follower is at rest at the true bottom | Release now. Content above the viewport shrinks, the browser clamps `scrollTop` by the same amount, and nothing on screen moves. This is the 06-16 "collapse once scrolled off the top" rule, applied to every row instead of only tools. |
| DETACHED (the user is reading) | Release when the row is fully off-screen. If it is above the viewport, correct `scrollTop` by the delta in the same RO pass (anchor-row correction, the 10-04 report §6.3). |
| Pane width or zoom changed (user-initiated) | Drop every floor and re-measure. Exempt by §1. |

There are no timers. A floor never "times out" onto a reader.

**Where the floor is enforced.** In one ResizeObserver over the live-region row wrappers. When an entry's height
dropped below its recorded max, write `min-height = floor(max)` (always the **floor** of the exact zoom-corrected
height, never `offsetHeight`, which rounds up; that rounding was the 367 ms tremor in the 10-04 RCA). Then call the
follower's `settle()` in the same callback. RO runs after layout and before paint, so the shrink is never painted, and
the `scrollTop` the browser clamped during that layout is restored before paint as well.

This is generic. It covers tool results, panel collapses, markdown re-highlighting, async renderers and anything added
later, without each component having to opt in. Components can still do better at the source, for example by
rendering a preview's result inside the same box at the same height, so the floor has nothing to hold. Those are polish
items (§5 Phase 4), not correctness.

### R3. The viewport doesn't grow mid-flow

The scroller's `clientHeight` may shrink while following (the follower pins, and content moves up), but it may not
grow except for the user-initiated changes in §1. Transient chrome below the transcript (the working row,
ActivityDock rows, the question panel, notices above the composer) is laid **over** the bottom of the scroller instead
of beside it. The transcript content gets a matching **bottom inset**, managed by the same floor rule as R2:

- the inset grows immediately (content moves up);
- when the chrome shrinks or leaves, the inset stays as blank space where the chrome was;
- the inset is released only at the next **user commit point**: a send, the user's own scroll, or a pane resize.

At turn end, the working row disappears and leaves its space empty. Nothing slides down.

### R4. One writer, one place it runs

Exactly one function moves the transcript's `scrollTop` while FOLLOWING: the follower's `settle()`. It is called at
the end of **every** ResizeObserver callback that can change a height: the content RO, the viewport RO, the floor RO,
and the row-measure RO, which also fixes the one-frame-late head pin. `jumpToBottom` dispatches `JumpToBottom` to the
state machine and calls `settle()`. It no longer writes `scrollTop` itself.

This is invariant I3 of the 09-24 spec, extended with the motion. It also gives the 09-24 Phase 1/2 reducer and
binding (`frontend/app/view/agent/scroll/`) their first real job, so the two specs are built as one module.

## 4. The follower

```
settle(now):                         // runs inside RO callbacks and from rAF while moving
  if state != FOLLOWING: return
  max = scrollHeight - clientHeight  // layout is clean here (RO) or we're in rAF after a write
  gap = max - scrollTop
  if gap <= 0.5:          at rest; apply pending invisible floor releases (R2 row 1); stop rAF
  elif reducedMotion or gap > SNAP_PX:   scrollTop = max         // one step
  else:                   scrollTop += max(1, gap * (1 - exp(-dt / TAU)))   // never past max
                          schedule rAF(settle)
```

- **`TAU` = 60 ms** (about 140 ms to land 90% of a step). Steady streaming reads as one continuous scroll. A new
  target arriving mid-move just raises `max`; there is no carried offset to get wrong.
- **`SNAP_PX` = 0.75 × clientHeight**, the threshold #4178 already chose. A paste-sized arrival is shown at once
  instead of scrolled through slowly.
- **Monotonic by construction.** While FOLLOWING the follower only ever increases `scrollTop`, and the browser caps it
  at `max`. The only way `scrollTop` decreases is the clamp from a release above the viewport, which R2 restricts to
  "at rest", where it moves nothing on screen.
- **Lag is visible and harmless.** While moving, the newest few lines sit just below the viewport edge for about 100 ms.
  That is the same as the glide today, without drawing anything in the wrong place.
- **User input wins at once.** A wheel-up or a scroll key cancels the rAF synchronously in the input handler (the
  09-24 §5.4 gesture table) before the next `settle` can run. The follower's own scroll events carry no user-intent
  window, so the state machine ignores them (I1).
- **The inner preview boxes use the same follower** (`ToolOverlayLog`'s internal follow, `SystemToolInstallInline`).
  Their duplicated follow rules go away, as 09-24 Phase 3 planned.

## 5. Rollout

Each phase is one PR. Everything new sits behind **`agent:onewayflow`** (default on in dev builds, off in release),
with the old path kept for one release as the kill switch, the same pattern as `agent:turnscopedtail`.

| Phase | What | Exit criterion |
|---|---|---|
| **0. Measure** | Frame recorder (dev, Ctrl+Shift+D HUD): per rAF, `scrollTop`, `scrollHeight`, `clientHeight`, and the screen `top` of each visible row by node id. It checks V1 and V2 live and logs `[one-way] violation kind=… row=… dy=… cause=…`, with cause attributed from the last RO and timer that ran. A production counter for violations, rate-limited. **Baseline it on current main** with the soak script (§6) before any fix. | A baseline number for V1/V2 violations per minute of tool-heavy streaming, with causes. This settles §2's inferences. |
| **1. Follower + R1** | `scroll/follow-controller.ts` (the 09-24 reducer) plus `follower.ts` (§4). Route every pin path through `settle()` (R4). Delete the glide and the row-enter rise behind the flag. | V2 violations = 0 in the soak. |
| **2. Floors (R2)** | The floor RO, invisible release, and DETACHED anchor-row correction. Delete the shrink hold and the hold ease. | V1 violations from row shrinks = 0. |
| **3. Overlay chrome (R3)** | Working row, ActivityDock, question panel and notices overlay the scroller with a managed bottom inset. | V1 violations from viewport growth = 0. |
| **4. Sources and DRY** | Preview boxes on the follower; the preview's result renders in the same box at its running height, so there is no floor gap; `resize-contract.ts` keeps grow animations only. | Floor-gap time per tool call is reported; no new violations. |
| **5. Default on** | 30-minute, 3-pane soak with zero violations; flip the default; delete the old path, `anchor.ts`'s 200 px threshold, the dead flags, and the hold/glide constants. Update this spec and 09-24 to `implemented`. | Owner sign-off on a live run. |

## 6. Verification

- **Unit (pure).** Follower table tests: monotonic `scrollTop` across random growth sequences; never above `max`;
  snap above `SNAP_PX`; cancel on `UserEscape`; reduced motion snaps. Floor policy tests: a shrink while visible gives a
  floor; release only when off-top and at rest, or off-screen when DETACHED; width or zoom change drops floors.
- **jsdom binding.** Extend `AgentDocumentVirtualList.pin.test.tsx` / `.resize.test.tsx`: assert a single writer of
  `scrollTop` while FOLLOWING, and that no element in the scroller has a transform or height animation while FOLLOWING.
- **Live soak (the acceptance test).** `scripts/ui-screenshots/full-conversation-bench.mjs --one-way-soak <min>` over
  CDP against a dev build. It streams a scripted tool-heavy turn through the real pipeline: Bash previews that
  stream then complete shorter, Read/Edit previews, parallel tools, panel expand/collapse, subagent dock rows, a
  question panel, turn end, zoom 0.89. It reads the Phase 0 recorder and fails on any V1/V2 violation outside a
  synthetic user gesture. It also runs the 09-24 follow-soak assertions (FOLLOWING stays at the bottom within
  `SNAP_PX` and settles within 300 ms).

## 7. What this trades away (decisions for the owner)

1. **Blank space inside a shrunk row until it scrolls off.** A finished tool whose result is shorter than its preview
   keeps the preview's height until new content pushes it above the viewport. That replaces today's "gap, then slide
   back". Phase 4 removes most of it at the source. **Recommended:** accept; it is the only way a visible shrink can
   avoid moving the content.
2. **Blank space where the working row was, until the next send.** The same trade for chrome (R3). **Recommended:**
   accept.
3. **Motion style.** The follower scroll (§4) replaces the glide and the row rise; rows still fade in. The alternative
   is an instant snap with fade only, which is simplest, with no rAF at all. **Recommended:** the follower, keeping
   the smoothness the owner asked for on 10-01.

## 8. What this doesn't cover

- Mechanism B of the 10-04 report (head estimates corrected while the user scrolls history) is a DETACHED problem. It
  needs anchor-row correction and a width-aware estimator, and is tracked there. R2's DETACHED correction shares the
  same helper.
- The ANCHORED (ChatGPT-style turn anchoring) state and the "↓ New activity" pill stay where 09-24 put them.
