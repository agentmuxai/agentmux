# Report: why the message-list scrollbar still visibly drifts up before snapping to bottom, and how to make it pin unconditionally

**Date:** 2026-07-30
**Author:** Agent3
**Status:** Audit + research only — no code changed this pass. Recommends a follow-up implementation spec.
**Scope reviewed:** `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx`, `frontend/app/view/agent/virtualization/state.ts`, `frontend/app/view/agent/virtualization/anchor.ts`, `frontend/app/view/agent/components/MarkdownBlock.tsx`, `frontend/app/view/agent/useAgentStream.ts`, `frontend/app/view/agent/stream-flush-queue.ts`, and the three prior scroll-follow passes: `docs/specs/SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md`, `docs/specs/SPEC_WORKING_STATE_AND_SCROLL_FOLLOW_HARDENING_2026_07_27.md`, and commit `30af99e8` (#2349, 2026-07-28).

---

## 1. Symptom, precisely

While an agent is actively streaming and the user hasn't touched the scrollbar, the thumb visibly **rises up the track as content streams in**, then some time later **snaps back down to true bottom**. Stick-to-bottom does eventually "get there," but the correction is visibly late rather than invisible. This is a distinct bug from the three the last three passes fixed (silent *disengagement* of stick-to-bottom) — here, `stickToBottom` stays `true` the whole time; the *visual* pin is just lagging behind the *state*.

## 2. Current architecture (as of `ff9afe98`, pulled fresh this session)

`AgentDocumentVirtualList.tsx` keeps `stickToBottom` as a Solid signal (`state.ts`) that is the source of truth, and re-projects it into `scrollRef.scrollTop` via **one `createEffect`** (lines 431–450):

```ts
createEffect(() => {
    const _len = props.viewState.nodes().length;
    const _totalSize = props.layoutView?.()?.totalSize;
    const _workingRowHeight = props.workingRowHeight?.();
    if (props.viewState.stickToBottom() && scrollRef) {
        queueMicrotask(() => { ... scrollToTrueBottom(); ... });
    }
});
```

This re-pins **only when one of three specific, hand-picked dependencies changes**: node count, the virtualized region's prefix-summed total height, and the floating working-row's measured height. A second, more general `ResizeObserver` (added 2026-07-27, §3 of the hardening spec) re-pins on *any* change to `scrollRef.clientHeight` — but `clientHeight` is the **viewport** size, driven by sibling panels (retry bar, decision panel, etc.) resizing the flex column above/below the scroll region. Neither mechanism observes the **content's own box size** growing, which is what actually happens on every streamed token.

This is the third documented pass at this bug class, and each pass added one more named dependency or observer rather than a structural fix:

| Date | Fix | Dependency added |
|---|---|---|
| 2026-07-24 | `SPEC_..._SCROLL_FOLLOW_AND_STATUS_OVERLAY` | `nodes().length`, `layoutView().totalSize` |
| 2026-07-27 | `SPEC_..._SCROLL_FOLLOW_HARDENING` §3 | `workingRowHeight`; generalized `clientHeight` RO |
| 2026-07-28 | #2349 | `pendingProgrammaticScroll` race flag (fixes false *disengage*, not lag) |

The pattern itself is the root cause: **the pin is derived from an itemized whitelist of "things that might grow the content," not from an observation of the content actually growing.** Every source of height change not on that list reproduces exactly the reported symptom — content grows, `stickToBottom` stays true, but nothing re-fires the effect, so the scrollbar visibly sits above true bottom until some *unrelated* dependency happens to change and the effect re-runs.

### 2.1 A concrete, currently-untracked growth source: `MarkdownBlock`'s trailing highlight render

`frontend/app/view/agent/components/MarkdownBlock.tsx:54-72` throttles markdown re-parsing during streaming (`STREAM_RENDER_MS = 90`, a deliberate perf rate-limit — re-highlighting on every token is O(n²)):

```ts
trailing = setTimeout(() => {
    streaming = false;
    lastCommitAt = performance.now();
    setView({ text: props.node.content, highlight: true });
}, STREAM_RENDER_MS);
```

The leading-edge render during active streaming skips syntax highlighting (`highlight: !streaming`); ~90ms after the *last* token in a burst, a **trailing-edge `setTimeout` fires and re-renders with `highlight: true`**. Syntax-highlighted code blocks (`<pre><code>` with per-token spans) routinely reflow to a different height than the plain-text intermediate — different line-wrapping, added block chrome, etc.

This `setTimeout` write is completely outside the Solid signal graph `AgentDocumentVirtualList`'s pin effect depends on: it doesn't touch `props.viewState.nodes()` (the node object reference in the streaming buffer was already updated for content; this is a *local* re-render of the same node), doesn't touch `layoutView()` (streaming-buffer rows aren't virtualized), and doesn't touch `workingRowHeight`. So every time a response contains a code block and the model pauses between tokens for ≥90ms (which is common — it happens between essentially every sentence/paragraph, not just at the end of a turn), the row's height jumps with **no corresponding re-pin**, and the scrollbar sits wrong until the *next* real node-count change fires the effect. This reproduces the reported "moves up, then later snaps back" behavior directly and repeatedly during normal streaming, not just at turn boundaries.

There's also a second, structural culprit: `.agent-document`'s `padding-bottom` is driven by `--agent-working-row-height` (`frontend/app/view/agent/styles/_document.scss:43`, fed from `workingRowHeight()` in `agent-view.tsx:1122/1624`). This one *is* tracked as an effect dependency today, but it illustrates the same fragility from the other direction: it needed its own hand-added dependency in the 2026-07-27 pass specifically because nothing structural was watching the true-bottom target.

## 3. What the wider ecosystem does (external research)

Searched for how production chat/streaming UIs solve exactly this ("stick to bottom" flicker during variable-height streaming content):

- **[stackblitz-labs/use-stick-to-bottom](https://github.com/stackblitz-labs/use-stick-to-bottom)** ([README](https://github.com/stackblitz-labs/use-stick-to-bottom/blob/main/README.md), [npm](https://www.npmjs.com/package/use-stick-to-bottom)) — the most widely cited reference implementation for exactly this problem (AI chat streaming). Core mechanism: a `ResizeObserver` on the **content wrapper** (not the scroll viewport) drives the re-pin; a separate boolean tracks whether the user is "stuck," toggled only by real scroll events, with explicit logic to distinguish the library's own programmatic scroll-driven `scroll` events from genuine user ones (the same class of problem `pendingProgrammaticScroll` in this codebase already solves).
- **[GetStream/stream-chat-react PR #1608](https://github.com/GetStream/stream-chat-react/pull/1608)** — a production chat SDK's own postmortem of this exact bug. Their prior implementation used a **200ms `setTimeout` to scroll to bottom after the last invocation** — structurally identical to the itemized/timer-driven approach this codebase currently uses — and it broke under slow image loads because the timeout fired before the image finished resizing the container. Their fix: **switch to `ResizeObserver` on the message container**, keeping it scrolled to bottom on every resize until the user's own scroll cancels it (detected by comparing the count of programmatic `scrollToBottom()` calls against observed `scroll` events — again, the same disambiguation problem already half-solved here).
- **[WICG/resize-observer chat example](https://github.com/WICG/resize-observer/blob/master/examples/chat.html)** — the spec authors' own canonical demo for this exact use case: observe the chat-text element, and on every resize entry, set `chat.scrollTop = chat.scrollHeight - chat.clientHeight`.
- **[web.dev: ResizeObserver](https://web.dev/articles/resize-observer)** — explains *why* this timing matters: the spec places ResizeObserver's notification step **after layout, before paint** in the rendering pipeline, specifically so a callback can make further layout-affecting changes (like adjusting `scrollTop`) without an intermediate frame ever being painted. This is the structural reason a `queueMicrotask`-after-a-Solid-effect approach can visibly flicker where a `ResizeObserver` callback cannot: a microtask queued from inside a signal-write callback is *timing-adjacent* to pre-paint but is not *tied* to an actual box-size change — it fires (or doesn't) based on which signals happened to be written, not based on whether the box actually grew.

### Convergent best practice across all sources

1. **Persist "am I stuck to bottom" as a plain boolean, set only by real user-scroll evidence** — this codebase already does this correctly (`AgentViewState.stickToBottom`, `isNearBottom` heuristic in `anchor.ts`, and the `pendingProgrammaticScroll` disambiguation flag from #2349). No change needed here.
2. **Drive the actual re-pin from a `ResizeObserver` on the content box that can grow, not from an itemized list of application-state dependencies.** This is the one piece this codebase does *not* do — it re-pins from Solid signal changes (`nodes().length`, `layoutView().totalSize`, `workingRowHeight`, and separately `clientHeight` on the viewport), which is exactly the pattern Stream Chat's team identified as unreliable and replaced.
3. **Do the scrollTop write synchronously inside the observer callback**, not deferred via `setTimeout`/`queueMicrotask` — the pre-paint timing guarantee is what eliminates the flicker; deferring the write (even by one microtask) reopens the window the RO callback is specifically positioned to close relative to unrelated paints.
4. **Use instant (`behavior: "auto"`) scroll during streaming, reserve smooth/eased scrolling for explicit user-triggered jumps.** Already correct here (`scrollToTrueBottom` uses `"auto"`; only `jumpToBottom`/`scrollToNode` use `"smooth"`).

## 4. Recommended fix

Replace the itemized-dependency `createEffect` (lines 431–450) with a `ResizeObserver` observing the two elements whose box size actually represents "how tall is the content right now":

- `virtualContainerRef` (the virtualized region — currently sized via `style.height` tied to `layoutView().totalSize`, so this RO firing here is redundant with the existing tracked dependency, but unifies the mechanism)
- the `.agent-document-streaming-buffer` element (currently **not observed by anything** — this is the actual gap; it grows from every markdown token append, every `MarkdownBlock` trailing-highlight re-render, every tool-log chunk, and any image/font load inside a rendered node)

```ts
const contentRO = new ResizeObserver(() => {
    if (!props.viewState.stickToBottom() || !scrollRef) return;
    pendingProgrammaticScroll = true; // existing race-guard from #2349, unchanged
    scrollRef.scrollTop = scrollRef.scrollHeight - scrollRef.clientHeight;
});
// observe() calls added alongside virtualContainerRef / streaming-buffer mount
```

Notes on integrating this without regressing the three already-fixed bugs:

- **Keep `pendingProgrammaticScroll` and the `handleScrollNow` disengage-suppression logic exactly as-is** (#2349) — setting `scrollTop` still synthesizes a native `scroll` event, so the existing programmatic-vs-user disambiguation is still required and already correct.
- **`workingRowHeight` needs to stay in the picture.** It changes `.agent-document`'s own `padding-bottom`, not a child element's box, so a RO on the two content children won't observe it. Cheapest option: keep reading `props.workingRowHeight?.()` as a plain (untracked) value inside the RO callback's target computation — it doesn't need to be a *trigger* anymore (the RO fires on every real content change, and the working row's own resize is already covered by the *existing* `clientHeight`-triggered `ResizeObserver` on `scrollRef`, since the working row is a sibling overlay, not part of `scrollRef`'s content — confirm this during implementation rather than assuming). Alternatively, migrate the working-row spacer from `padding-bottom` to a real last-child spacer `<div>` inside the observed content region, which would let the *same* RO cover it structurally and remove the special case entirely.
- **The itemized `createEffect` doesn't need to be deleted outright.** It's cheap, harmless, and covers the *new-message-just-appended* case redundantly with the RO. Simplest low-risk migration: keep it as a secondary trigger, but change its body to call the *same* synchronous `scrollRef.scrollTop = ...` write instead of `queueMicrotask(() => scrollToTrueBottom())` — removing the microtask indirection is itself worth doing even before the RO lands, since it's one less deferred hop between "content grew" and "scrollbar corrected."
- **Verify RO doesn't fight the existing virtualized-height `style.height` write.** Since `virtualContainerRef`'s height is already driven by `layoutView().totalSize`, observing it too should be a no-op most frames (RO only fires on actual size deltas) — but confirm there's no feedback loop (RO fires → scrollTop write → does *not* itself resize either observed element, so this should be a non-issue, but WICG's own examples note some browsers historically had double-fire quirks worth a quick manual check).

## 5. Suggested next step

This report is audit + research only. Recommend a short follow-up implementation spec (`SPEC_AGENT_PANE_SCROLL_PIN_CONTENT_RESIZE_OBSERVER_<date>.md`) covering:

1. The `ResizeObserver` swap described in §4, scoped to `AgentDocumentVirtualList.tsx`.
2. A manual repro check specifically exercising the `MarkdownBlock` trailing-highlight gap identified in §2.1 (stream a response containing a large fenced code block, confirm no visible thumb-rise-then-snap ~90ms after the block finishes streaming).
3. `npx vitest run app/view/agent` regression pass — the existing scroll-follow test coverage (`state.test.ts` and whatever covers `AgentDocumentVirtualList`) should be extended with a case asserting the pin fires from a content-only resize with no `nodes()`/`layoutView()` change, since that's precisely the gap being closed.

## Sources

- [stackblitz-labs/use-stick-to-bottom](https://github.com/stackblitz-labs/use-stick-to-bottom)
- [use-stick-to-bottom README](https://github.com/stackblitz-labs/use-stick-to-bottom/blob/main/README.md)
- [use-stick-to-bottom on npm](https://www.npmjs.com/package/use-stick-to-bottom)
- [GetStream/stream-chat-react PR #1608 — "use ResizeObserver to keep Channel scrolled to bottom"](https://github.com/GetStream/stream-chat-react/pull/1608)
- [WICG/resize-observer — chat.html example](https://github.com/WICG/resize-observer/blob/master/examples/chat.html)
- [web.dev — ResizeObserver: it's like document.onresize for elements](https://web.dev/articles/resize-observer)
