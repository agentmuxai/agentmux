# SPEC: Anchor the startup-injection summary at the cursor during hover-expand

**Date:** 2026-05-24
**Author:** AgentA
**Status:** Draft — needs decision on §4.2 (Option A vs B) before implementation.
**Supersedes for §3 (hover-expand mechanics) parts of:** `SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24.md` §C.2.

---

## TL;DR

PR #1020 ships the startup-injection collapse-on-hover. In practice, when the user hovers the `⓵ Session context` summary, the row expands downward and pushes nearby content. The user perceives an unexpected layout shift, then their mouse — naturally pursuing what they were aiming at — ends up off the summary, mouseleave fires, the row collapses, and the click-to-pin never lands. The expand → mouse follow → collapse → repeat loop is the bug.

The fix: **the summary stays under the cursor across the expand transition.** The body opens UP or DOWN from the summary depending on viewport-space + cursor position, and the summary's screen-Y stays exactly where the user was already pointing. No jitter, no animation, no scroll-anchor hacks.

---

## 1. What's actually happening today

### 1.1 The current expand mechanics (PR #1020)

The collapsed startup row is a 32px-tall `<button>` (the summary). When `hovering` flips to `true` after the 150ms enter-delay, SolidJS swaps the `<Show>` branches: summary out, body in. The body is rendered inside `.agent-user-message-content` as a normal `<pre>` block, which means it sits in document flow **below** the row's top edge. The row grows from 32px to whatever the body is tall (often 200-400px for the session-context Markdown).

Because the body sits in normal flow, three things shift:

1. **The row itself.** Its measured height jumps from 32px to 200+px.
2. **Rows below it.** Pushed down by the row's new height.
3. **The virtualizer.** When the row's measured height changes, TanStack Virtual recomputes the offset map and adjusts scroll-anchor — `estimateUserMessage` is now newline-based (PR #1020 round 5), so the estimate vs measured delta is much smaller, but it isn't zero.

### 1.2 Why the user sees a collapse

The summary's screen-Y position **doesn't change** during the expand — its top edge is still anchored to the row's top edge in normal flow, and the row's top didn't move. So in theory the cursor remains over the summary's hit area.

In practice, two failure modes show up:

- **Mouse drift during the 150ms enter-delay.** The user hovers, sees nothing for 150ms, naturally drifts the cursor a few pixels. When expansion finally fires, the cursor may already be 2-3px below the summary (still inside the row's collapsed 32px box, but on its bottom edge). The expansion replaces the summary's bottom 16px with `<pre>` content (because `<button>` had padding that the `<pre>` doesn't reproduce exactly), the cursor lands on `<pre>` instead of `<button>`, the body has no `onMouseLeave/Enter` reciprocal binding, and mouseleave on the outer block fires the moment the user drifts another pixel.
- **Layout-shift surprise.** Even when the cursor stays on the summary's hit area, the row's height suddenly tripling pushes the rows below it down by ~200-400px. The user's eye tracks "the thing I was about to click" and instinctively moves the cursor toward where they expect the summary to be — except the summary is exactly where it always was, and they over-correct off it.

Both cancel the hover; mouseleave fires; the row collapses. Click-to-pin requires re-entering the now-collapsed summary, which restarts the 150ms timer.

### 1.3 What `feedback_no_timers_or_delays` rules out

We cannot fix this with a grace window, a debounce on mouseleave, a transition animation, or a setTimeout-based "give the user a moment before collapsing." Per the user's memory, those are exactly the workarounds that get rejected as jitter. The fix has to be **deterministic and geometric**: arrange the DOM so the summary is structurally under the cursor after the expand, full stop.

---

## 2. What the user wants

Quoted verbatim:

> wherever the mouse cursor is the expansion happens (either up or down, depending on if the hover is near the bottom). We need to ensure the cursor always ends up at the line (header during expansion) after expansion.

Two parts:

- **Direction is chosen at expand-time.** Some hovers should expand the body **upward** (body rendered above the summary), some **downward** (body rendered below the summary). The signal is "is there room below, or is this row near the bottom edge of the viewport."
- **The summary is always under the cursor after expand.** No matter which direction the body grows, the summary's screen-Y matches the cursor's screen-Y immediately after the expand fires.

This is a known pattern — context-menu placement uses the same rule: open in the direction with more space, and anchor the menu trigger at the cursor.

---

## 3. The geometric invariant we need

Define:

- `summary.y` = the summary's screen-Y when mouseEnter fires (a constant for the duration of this hover).
- `cursor.y` = the cursor's screen-Y at the moment the 150ms timer fires.

The invariant: **after the expand, `summary.y` is unchanged**.

If the body grows DOWN (body rendered below summary in document flow), `summary.y` doesn't change — that's the current behavior, and it's fine. The bug is only the user's mental model surprise.

If the body grows UP (body rendered above summary), `summary.y` *would* change in normal flow — the row's top edge moves up by `body.height`, summary shifts down by `body.height`. To prevent this, the body must NOT participate in normal document flow.

So the implementation has two equivalent shapes:

- **Always downward in flow** (today's behavior), and accept the user's complaint about the surprise.
- **Body is positioned absolutely** so it can grow in either direction without moving the summary in flow.

The user's request is incompatible with the first shape. So: absolute positioning.

---

## 4. Proposed design

### 4.1 The DOM shape

```html
<div class="agent-user-message agent-user-message--startup"
     style="position: relative">

  <!-- ALWAYS in normal flow. Screen-Y never changes during hover.
       Bounding box stays 32px tall regardless of expand state. -->
  <button class="agent-user-message-summary">
    ⓵ Session context  (hover to peek · click to pin)
  </button>

  <!-- ONLY when hovering or pinned. Absolute-positioned inside
       .agent-user-message; placement = below or above summary,
       chosen at expand-time. Carries its own onMouseEnter /
       onMouseLeave to keep `hovering` true while the cursor is
       over the body. -->
  <div class="agent-user-message-content
              agent-user-message-content--below"
       style="position: absolute; top: 100%; left: 0; right: 0;
              z-index: 10;">
    <pre>{message}</pre>
    <button class="agent-user-message-pin">📌</button>
  </div>
</div>
```

When the body is `--above` instead of `--below`:

```html
       style="position: absolute; bottom: 100%; left: 0; right: 0;
              z-index: 10;"
```

The summary stays in normal flow at 32px. The body floats above neighboring rows (z-index handles the visual stacking). The row's height in document flow is **always 32px** while the body is shown via hover — neighbors don't reflow, the virtualizer doesn't remeasure.

### 4.2 OPEN QUESTION: pinned state — overlay or in-flow?

This is the one decision that needs your input before I start coding.

**Option A: Pinned = stays absolute (overlay forever).**

- Pinned body floats over the rows below it.
- Document flow unchanged; the row is still 32px in the document.
- Virtualizer never sees a height change.
- Visually: the user has a floating panel anchored to one row.

**Option B: Pinned = drops back into normal flow.**

- When the user clicks 📌, the body switches from `position: absolute` to in-flow rendering below the summary.
- Row's height in document flow grows to 32 + body.height.
- Virtualizer remeasures; rows below shift down.
- Visually: the pinned context "anchors" itself into the document like a real expanded block.

I lean Option B — pinning is a persistence statement, and persistence wants document flow so the user can read past it without floating chrome covering things. But there's a coherent case for A (consistent overlay UX, no virtualizer churn on pin). Specify your preference and the implementation follows.

**Recommendation:** Option B. Pinned = full in-flow render; hover-expanded = absolute overlay.

### 4.3 Direction selection

When the 150ms enter timer fires:

```ts
const rect = summaryEl.getBoundingClientRect();
const viewportH = window.innerHeight;
const bodyEstimate = estimateUnwrappedTextHeight(node.message);

// Space available in each direction.
const spaceBelow = viewportH - rect.bottom;
const spaceAbove = rect.top;

// Pick the direction with more room; fall back to below on a tie.
const direction =
    spaceBelow >= bodyEstimate || spaceBelow >= spaceAbove
        ? "below"
        : "above";

setExpandDirection(direction);
```

Captured **once per hover cycle**. The direction is locked for the duration of this hover — the body doesn't flip mid-hover. Mouseleave clears it; the next mouseenter re-evaluates.

This is the only place we read `getBoundingClientRect()`, and we do it once. No animation frames, no resize-observer, no scroll listener — the direction is right at expand-time and stays consistent.

### 4.4 Mouse-region continuity across summary ↔ body

The summary's bounding box is 32px. The body's absolute-positioned bounding box sits adjacent to it (above or below). The cursor moves from one to the other.

Today (PR #1020), the outer `.agent-user-message` div has the `onMouseEnter / onMouseLeave`. With the body in normal flow, the outer div's bounding box covers both. With the body absolutely-positioned, **the outer div's bounding box is only the summary** — moving the cursor into the body would fire `mouseleave` on the outer div, and the row would collapse.

The fix: each region (summary, body) owns its own enter/leave, and a derived signal combines them:

```ts
const [overSummary, setOverSummary] = createSignal(false);
const [overBody, setOverBody] = createSignal(false);
const hovering = () => overSummary() || overBody();
```

Sequence when the cursor crosses from summary to body, with body anchored at `top: 100%` (zero-pixel gap by construction):

1. `summary.mouseleave` fires → `setOverSummary(false)`.
2. `body.mouseenter` fires → `setOverBody(true)`.
3. SolidJS batches both into one re-render. `hovering()` = false || true = true. No flicker.

The CSS `top: 100% / bottom: 100%` placement is critical — it guarantees zero gap. With `top: calc(100% + 1px)` the user would catch a 1px gap that fires mouseleave AND no body mouseenter → collapse. The spec lives or dies on this contract; the L2 test pins it.

### 4.5 No jitter, no timers added

This whole design is geometry + already-deterministic SolidJS reactivity. No new `setTimeout`. The existing 150ms enter-delay on the summary stays as-is (it's the only timer in the file). Per `feedback_no_timers_or_delays`.

---

## 5. Edge cases

### 5.1 Body too tall for the viewport

`bodyEstimate > Math.max(spaceAbove, spaceBelow)`. Pick the larger side, cap the body's height at `Math.max(spaceAbove, spaceBelow) - <some margin>`, add `overflow-y: auto`. User scrolls inside the body.

The cap is applied in CSS via `max-height: calc(100vh - <reserved>px)`; no JS measurement needed at render time. Spec value: `max-height: calc(100vh - 80px)` (40px top margin from viewport edge + 40px breathing room).

### 5.2 Window resize mid-hover

When the viewport changes size, the direction we picked at expand-time may now be the wrong one. We do NOT add a resize listener — the body re-positions on next hover anyway. Resize while hovering is an acceptable rare case where the user gets a brief moment of awkward placement; mouseleave/mouseenter resolves it.

### 5.3 Row near the very top of the agent pane

`rect.top` is small (e.g. 20px). Body can't expand upward more than 20px worth. Direction selection falls through to "below" via the `spaceBelow >= spaceAbove` tie-breaker.

### 5.4 Row that's currently scrolled partly off-viewport

The summary's `rect.top` is negative (off-screen at top) or `rect.bottom` is past `viewportH` (off-screen at bottom). Mouseenter shouldn't fire — the cursor isn't physically over the row. The direction calculation has self-consistent fallback behavior (negative space ends up tiny; the other side wins). No special case needed.

### 5.5 Multiple startup rows in the same pane

`buildStartupPayload` is called once per fresh agent. Realistically there's one startup row per pane. But the spec doesn't break with N: each row's body is anchored to its own summary; z-index ordering means later-hovered overlays sit above earlier-pinned ones; no shared state to corrupt.

### 5.6 The body overlaps with another row that is also pinned/hovered

Rare but possible: two pinned rows whose absolute bodies overlap. Use `z-index: 10` consistently; the most-recently-hovered/clicked row wins by mount order. Acceptable.

### 5.7 Pinned body when scrolling

(If Option B chosen, this is moot.) Under Option A, the pinned body stays anchored to the summary via absolute positioning. Scrolling the agent pane scrolls the summary; the body moves with it (same `.agent-user-message` ancestor). No issue.

### 5.8 Pin-then-unpin while still over the body

User clicks 📌 → pinned=true. They keep the cursor on the body. They click ✕ → pinned=false. Body collapses; cursor is now in the empty space the body occupied. Next mousemove either lands somewhere new entirely (and stays collapsed) or moves into the summary (and starts a fresh hover cycle). Both are fine.

### 5.9 Keyboard expansion

The summary is a `<button>` already, so Tab focuses it. Today, pressing Space/Enter calls `onTogglePin` directly (no hover-expansion step). The body renders pinned, in whichever direction Option B says — for keyboard activation, pinned = always in-flow (Option B), so body renders below in document order. Keyboard users skip the hover-direction logic entirely; they get the predictable downward expansion.

### 5.10 Pre-existing tool blocks doing the same

ToolBlock has its own overlay portal (`ToolBlockOverlay`). It does NOT have the cursor-anchor problem because tool blocks already always render their overlay below the trigger via `top: 100%`, and ToolBlock's trigger doesn't change size from "1 line" to "many lines" the way the user message does today. The user message's case is unique — we don't need to also re-spec ToolBlock.

But: the design here generalizes. If a future tool block grows enough to want the up/down choice, the same `getBoundingClientRect()` heuristic drops in.

---

## 6. Tests

### 6.1 L1 — pure unit tests on the direction selector

Pull the direction-selection logic into a free function:

```ts
export function pickExpandDirection(
    summaryRect: { top: number; bottom: number },
    viewportHeight: number,
    bodyEstimate: number,
): "above" | "below" { … }
```

Tests:
- Plenty of space below → returns "below".
- Plenty of space above, none below → returns "above".
- Equal space → returns "below" (tie-breaker).
- Body larger than both → returns the side with more space.
- Negative-Y summary (off-screen at top) → returns "below".
- Past-viewport summary (off-screen at bottom) → returns "above".

### 6.2 L2 — component tests on UserMessageBlock

- Hover-expanded body has class `agent-user-message-content--below` when there's room below.
- Hover-expanded body has class `agent-user-message-content--above` when crowded at the bottom.
- Body has `onMouseEnter / onMouseLeave` handlers attached when collapsible.
- Cursor moves from summary to body keeps `hovering` true (simulate via fireEvent: mouseleave summary → mouseenter body in sequence within one task tick).
- Cursor leaving the body (without re-entering summary) collapses the row.
- Pinned state renders body in normal flow (Option B) regardless of direction selection.

### 6.3 L3 — manual

- Open a fresh agent at the top of the pane. Hover the summary. Body opens DOWN. Cursor stays on summary.
- Scroll the pane down so the summary is near the viewport bottom. Hover. Body opens UP. Cursor stays on summary.
- Hover for 150ms, then click — pin lands every time (no race with collapse).
- Move cursor smoothly from summary into body — no flicker, body stays open.
- Move cursor out of the body — body closes, summary returns to collapsed.
- Repeat 20×; should be 20/20 successful pin clicks.

---

## 7. Order of delivery

One PR, three commits on `agenta/hover-anchor-direction`:

1. **Extract + L1 test the direction selector.** Pure function, no JSX changes. Tests cover the 6 cases in §6.1.
2. **Add absolute positioning + body mouse-region binding to UserMessageBlock.** SCSS adds `--below / --above` placement classes. Component adds `overSummary / overBody` signals, combined `hovering()`. Body still in normal flow when pinned (Option B). L2 tests in §6.2.
3. **Wire the direction selector to the component.** Read `getBoundingClientRect()` inside the existing 150ms timer callback. Set the direction signal. L3 manual verification.

Each commit is independently revertible; commit 1 alone ships nothing user-visible, commits 2+3 together deliver the fix.

---

## 8. Out of scope

- **Animations / transitions on the expand.** Out — adds jitter risk; the geometric pin is the whole spec.
- **Keyboard-driven direction selection.** Out — keyboard goes straight to pinned/in-flow per §5.9.
- **Generalizing to other agent-pane block types.** Out — ToolBlock has its own overlay; agent-message / markdown don't collapse on hover.
- **Mobile / touch.** Out — agent pane is desktop-only today.
- **Theming the overlay shadow.** A subtle `box-shadow` on `.agent-user-message-content--below / --above` to visually distinguish the floating body from the row chrome — included in commit 2 but only as a default; theme palette overrides come in a follow-up.

---

## 9. Related

- `SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24.md` — the parent spec. §C.2's render-shape claim ("body renders inline beneath summary") is updated by this doc.
- `docs/specs/tool-collapse.md` — ToolBlock overlay precedent; same direction-of-overlay pattern but no anchor-at-cursor invariant.
- `feedback_no_timers_or_delays` — the constraint that rules out grace windows; this spec satisfies it.
- PR #1020 — ships the version with the bug this spec fixes.
