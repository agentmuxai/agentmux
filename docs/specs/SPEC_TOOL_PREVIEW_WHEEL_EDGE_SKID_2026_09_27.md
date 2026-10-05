# SPEC: Wheel edge skid — a nested preview absorbs one wheel notch before the pane scrolls

**Status:** implemented — #4335 (2026-10-04), in `components/scroll-handoff.ts`
**Date:** 2026-09-27, revised 2026-10-04 against main before implementing (see §0)
**Author:** Camper (agent), at operator request
**Related:** `SPEC_TOOL_PREVIEW_SCROLL_CHAINING_2026_07_03.md` (the hand-off this spec
amends), `SPEC_AGENT_PANE_PREVIEW_CLEANUPS_2026_09_26.md` §2 (the shared hand-off it builds
on), `SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24.md` (pane follow / pin, must
not be disturbed), `frontend/app/view/agent/components/scroll-handoff.ts`.

---

## 0. Revision, 2026-10-04

The first draft was written against a checkout that predated #3933 (2026-09-26), which had
already replaced the tool preview's own wheel listener with one shared hand-off,
`attachScrollHandoff` in `components/scroll-handoff.ts`, used by every capped box in the
conversation. So the draft's plan for a new controller (`wheelHandoff.ts`) and re-plumbing
four boxes was out of date. §3–§7 are rewritten against main: the skid goes inside
`attachScrollHandoff`, and no call site changes. §1 and §2 stand. The operator's answers
to the open questions are in §8.

## 1. The problem

The agent pane (`.agent-document`) scrolls as a whole. A tool preview inside it
(`.agent-tool-overlay-log`) scrolls on its own, capped in height. When the wheel is turned
over a preview, the preview scrolls until it reaches its top or bottom. From then on the
very next wheel notch (one detent of the wheel's rotation) scrolls the whole pane
(`scroll-handoff.ts`, the Phase 2 hand-off of the 07-03 spec, shared since #3933).

That hand-off has no margin. Two cases feel wrong:

- **Scrolling a preview to its edge.** The notch that brings the preview to its top is
  followed immediately by a notch that jerks the pane. There is no moment where "the
  preview is at its top" is felt. A user reading upward overshoots out of the preview.
- **A preview arriving under the cursor.** While the pane scrolls, the cursor stays still
  and previews slide under it. A preview that arrives already at the edge in the scroll
  direction passes the next notch straight through, so it feels like the preview isn't
  there. A preview that isn't at that edge takes the notch instead. From the user's side
  the same gesture gives different results depending on the preview's hidden scroll
  position.

**The ask (operator, 2026-09-27):** when the cursor is over a tool preview and the next
wheel notch would leave the preview and scroll the pane, the preview absorbs that one
notch first. Scrolling up or down through a conversation, the wheel "skids" one notch at
each preview edge, then carries on.

## 2. Research: is there a best practice?

Yes, and it points the same way. Browsers don't hand a wheel over to the parent scroller
mid-gesture. They *latch*.

| Engine | Rule | Source |
|---|---|---|
| Chromium (our CEF 148) | A wheel scroll sequence latches to the scroller where it started. When that scroller hits its edge, the rest of the sequence goes nowhere; the parent scrolls on the *next* sequence. The sequence ends after about 500 ms without wheel events. | [Intent to Ship: Wheel scroll latching and async wheel events](https://groups.google.com/a/chromium.org/g/blink-dev/c/5jrqZmUBV9c); Chrome behaviour recorded in [lightpanda-io/demo#253](https://github.com/lightpanda-io/demo/pull/253) (a 1000 px wheel over a box with 416 px of travel leaves the page unmoved, and the page moves on the next wheel) |
| Firefox | "Wheel transactions": wheel events stay with the scroller they started on until 1500 ms pass without a scroll (`mousewheel.transaction.timeout`), or the pointer moves out of it. A scroller at its edge holds the parent back for the timeout. | [Gecko: Mouse Wheel Scrolling](https://wiki.mozilla.org/Gecko:Mouse_Wheel_Scrolling) |
| WebKit | Same latching idea. It released the latch too late once and was tightened so the next gesture always picks a scroller fresh. | [WebKit changeset 271730](https://trac.webkit.org/changeset/271730/webkit) |
| Apps re-implementing it in JS | Latch on the first event of a run, and keep it until 500 ms of wheel silence (Chromium's value), the pointer leaving the latched element, or that element being unable to scroll on the axis. | [pingdotgg/t3code#13405](https://github.com/pingdotgg/t3code/pull/13405), [blockeditor-org/be3#109](https://github.com/blockeditor-org/be3/pull/109) |

The CSS side is settled too: `overscroll-behavior: contain` stops the chain at a nested
scroller ([MDN](https://developer.mozilla.org/en-US/docs/Web/CSS/overscroll-behavior),
[CSS Overscroll Behavior Level 1](https://www.w3.org/TR/css-overscroll-1)). It is
all-or-nothing, with no "one notch of resistance". A skid can only be done in JS.

**What this means for us:**

1. The best practice is *resistance at the boundary*: the parent doesn't scroll in the
   same gesture that exhausted the child. Our current hand-off is the opposite. It
   forwards on every event at the edge, so it chains mid-gesture, which is the "jerk" the
   07-03 spec set out to remove in the first place.
2. Browsers measure that resistance in **time** (a 500 ms or 1500 ms idle gap). The
   operator asked for it in **notches**, and that's the better fit for a notched mouse
   wheel. A notch is a deliberate unit the user feels. Time-based latching makes a fast
   spin stop dead at every preview until the user pauses, which over a long conversation
   full of previews would be worse. So: **one notch** for a notched wheel, and the
   browsers' gesture rule for a trackpad, which has no notches (§4.3).
3. Chromium's async wheel events make every event after the first in a sequence
   non-cancelable. The design must not depend on `preventDefault()`. It doesn't: the
   preview already carries `overscroll-behavior: contain`, so an event we choose not to
   forward scrolls nothing by itself.

**What a notch is in the DOM** (Chromium on Windows, default settings): `deltaY = ±100`,
`deltaMode = 0` (pixels), and the legacy `wheelDeltaY = ±120` (Windows `WHEEL_DELTA`),
independent of display scaling. With "N lines per notch" set in Windows, `deltaY` scales
(33.3 px per line) but `wheelDeltaY` stays 120 per notch. "One screen at a time" sends
`deltaMode = 2`. A fast spin can coalesce into one event carrying ±240, ±360 and so on.
High-resolution and free-spinning wheels and precision touchpads send fractions of 120.
Sources: [Microsoft WHEEL_DELTA](https://learn.microsoft.com/dotnet/api/system.windows.forms.mouseeventargs.delta),
[MDN deltaMode](https://developer.mozilla.org/en-US/docs/Web/API/WheelEvent/deltaMode),
[mokuro-reader#277](https://github.com/Gnathonic/mokuro-reader/pull/277) (counting
detents, not pixels). Pane zoom scales CSS pixels, which is one more reason to count
notches with `wheelDeltaY` rather than `deltaY`.

## 3. Where it applies

Every capped box inside the conversation already goes through `attachScrollHandoff`, with
`overscroll-behavior: contain` on the box:

| Box | Renderer | Style |
|---|---|---|
| Tool preview `.agent-tool-overlay-log` | `ToolOverlayLog.tsx` | `_tool-overlay-portal.scss` |
| Jekt body `pre.agent-jekt-body` | `JektBubble.tsx` | `_document-nodes.scss` |
| Jekt raw payload (`.agent-jekt-raw pre`) | `JektBubble.tsx` | `_document-nodes.scss` |
| Context-delivery body `.agent-context-delivery-body` | `ContextDeliveryCard.tsx` | `_document-nodes.scss` |

So the skid reaches all four by changing the one helper. The dispatch result
(`.agent-tool-agent-result`) that the first draft called a dead end is no longer rendered by
anything: it became `.agent-tool-agent-report`, which sits inside the tool preview. Its CSS
rule is orphaned, and this spec leaves it alone.

Out of scope, as before: overlays, modals, pickers, the activity dock and the peek overlay.
They aren't inside the conversation flow, so there is no pane to skid into.

## 4. Design

### 4.1 Inside the shared hand-off

`attachScrollHandoff(el)` keeps its signature and its per-box `wheel` listener
(non-passive, as before, because forwarding calls `preventDefault()`). It adds:

- **One skid state per pane** (there's one pointer), in a `WeakMap` keyed by the pane's
  `.agent-document`. The skid needs to know which box the *previous* event was over, which
  one box's listener can't see on its own.
- **One passive pane listener** (capture phase, installed once per pane by the first box
  attached in it). A wheel over the pane but outside every box resets the state, so coming
  back to the same box is an arrival again. It never cancels, so it costs the pane's
  scrolling nothing. Boxes are tracked in a `WeakSet`.
- **A pure reducer**, `nextSkid(state, input) → { state, action }`, where `action` is
  `native`, `absorb` or `forward(deltaY)`. The listener reads the DOM, calls it, and
  applies the action.

### 4.2 State

```ts
interface WheelSkid {
    box: object | null;   // the box the last wheel event was over, or null
    dir: -1 | 0 | 1;      // sign of that event's deltaY
    phase: "armed" | "skidding" | "spent";
    lastAt: number;       // timeStamp of the last wheel event over box
}
```

### 4.3 The rule, per wheel event over a box

Ignore, without touching state: `ctrlKey` (zoom) and `deltaY === 0` (a horizontal scroll).

1. **Arrival:** `box` or `dir` differs from the state's. The phase becomes `armed`.
2. **A box whose content fits** (`scrollHeight - clientHeight <= 1`) isn't a scroller:
   forward the event at once and set `spent`. It never skids (as before).
3. **Not at its edge in `dir`:** the box scrolls natively; set `armed`. Any movement off
   the edge re-arms the skid.
4. **At the edge:**
   - `spent`: forward `deltaY` to the pane (`pane.scrollTop += deltaY`, with
     `preventDefault()`), exactly as before.
   - **Notched input** (`wheelDeltaY` a non-zero multiple of 120, or `deltaMode` line or
     page): the skid is one notch. Absorb it; if the event coalesced *n* notches, forward
     `deltaY × (n−1)/n`. Then `spent`.
   - **Continuous input** (trackpad, high-resolution wheel, or no `wheelDeltaY`): absorb,
     and stay `skidding` until an event arrives `SKID_GESTURE_IDLE_MS` (250 ms) or more
     after the previous one; that one is forwarded, then `spent`.
5. Record `box`, `dir` and `lastAt`.

"Absorb" means don't forward and don't `preventDefault()`: with `contain` on the box,
nothing scrolls.

### 4.4 What the user feels

With a notched wheel and the pointer still: the pane scrolls, a preview slides under the
pointer, notches scroll the preview to its edge, the next notch is the skid, and the one
after scrolls the pane. The same happens downward. Reversing direction re-arms.

**The edge flash (decided: on).** While a box absorbs, the edge it's pinned against shows
a 2 px accent line (`box-shadow: inset`, so nothing moves), which fades out over 150 ms
(`SKID_FLASH_MS`) after the last absorbed event. The helper adds `scroll-handoff-box` to
each box (for the transition) and toggles `scroll-handoff-skid--top` /
`scroll-handoff-skid--bottom`. None of the four boxes sets its own `box-shadow` or
`transition`, so nothing is overridden. Reduced motion drops the fade, not the flash.

### 4.5 Unchanged

- Ctrl+wheel zoom, and horizontal scrolling inside a box.
- Pane follow and pin (`AgentDocumentVirtualList`). A skid scrolls nothing; a forwarded
  notch sets `scrollTop` exactly as before. The wheel event still bubbles to the pane, so
  the pane's user-scroll-intent window sees the gesture either way.
- The tool preview's own follow-the-output logic (`ToolOverlayLog.tsx`), which listens for
  the wheel on the box itself.
- Wheel over the pane outside every box: native.

## 5. Edits

1. `components/scroll-handoff.ts`: `nextSkid` (pure), `wheelNotches`, the per-pane state
   and passive pane listener, and the edge flash. `attachScrollHandoff` keeps its
   signature.
2. `styles/_document-nodes.scss`: `.scroll-handoff-box` (the transition) and the two
   `.scroll-handoff-skid--*` edge lines.
3. `components/scroll-handoff.test.ts`: the hand-off tests now expect the skid.
4. A pointer to this spec from `SPEC_TOOL_PREVIEW_SCROLL_CHAINING_2026_07_03.md` and
   `SPEC_AGENT_PANE_PREVIEW_CLEANUPS_2026_09_26.md` §2.

No renderer changes: `ToolOverlayLog.tsx`, `JektBubble.tsx` and `ContextDeliveryCard.tsx`
already call `attachScrollHandoff`.

## 6. Tests

**Unit, `nextSkid`:** arrival absorbs, then forwards; off the edge is native and re-arms;
a different box or direction is an arrival; a coalesced event forwards the notches beyond
the first; continuous input is absorbed under the idle gap and forwarded after it; a box
that doesn't overflow forwards at once.

**DOM, `attachScrollHandoff`** (stubbed geometry; `wheelDeltaY` and `timeStamp` set on the
event): skid then forward, at the bottom and at the top; a notch that brings the box to its
edge, then the skid; Ctrl+wheel and `deltaY 0` ignored; a box that fits forwards at once; a
three-notch event; reversal; leaving for the pane and returning; a second box; a trackpad
gesture; the edge flash and its fade; detach.

**Live** (isolated `task dev`): drive real wheel input over a box with CDP
`Input.dispatchMouseEvent` (`mouseWheel`), logging both `scrollTop` values and the event's
`wheelDeltaY`. CDP-injected wheels skip Chromium's own latching, which this design doesn't
rely on. A real mouse and a precision touchpad each still need one manual pass.

## 7. Risks

| Risk | Mitigation |
|---|---|
| A skid at every box edge slows a fast scroll through a long conversation | One notch per edge, by design. If it proves too sticky, skip the skid when the previous wheel event was under ~40 ms ago (a fast spin). Not in v1. |
| High-resolution wheels (a fraction of 120 per event) are treated as continuous | They skid until the gesture pauses, like a trackpad, which matches how they feel. Tests pin both classes. |
| A box that grows or shrinks under the pointer (streaming output) flips `atEdge` | Read per event; leaving the edge re-arms, so at worst one more skid. |
| The virtual list replaces a box node mid-gesture | A new node is a different box: an arrival, one skid. Harmless. |
| `wheelDeltaY` is non-standard | Present in Chromium, our only engine; absent means continuous. |

## 8. Decisions (operator, 2026-10-04)

The operator accepted the recommendations:

1. **The edge flash during a skid:** on (§4.4).
2. **Skid length:** a constant of one notch, not a setting, until someone asks for one.
3. **Scope:** every capped box in the conversation, which on main is the four boxes of §3,
   all through the one helper.
