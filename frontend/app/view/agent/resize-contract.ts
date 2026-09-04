// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A single content-resize contract for the agent pane —
 * docs/specs/SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md.
 *
 * Landed with no call sites (step 2, zero runtime effect); step 3 migrated
 * `ToolOverlayLog.tsx`'s `flipHeight()` to it, the first real consumer.
 *
 * ## What this replaces
 *
 * Today, "a subtree is about to change height, ease it instead of jump-
 * cutting" is reimplemented per call site — `ToolOverlayLog.tsx`'s
 * `flipHeight()` tracks a `lastMeasuredHeight`/`lastBranch` pair across
 * effect runs by hand, with its own reduced-motion check, its own
 * content-visibility guard (`heightStale`), and its own `prevNodeId` reset
 * for the streaming-buffer slot-reuse hazard. `ToolBlock.tsx` needed the
 * identical `prevNodeId` guard for an unrelated reason (#1317). This module
 * owns that machinery once.
 *
 * ## The physical constraint this respects
 *
 * `scrollTop` cannot be eased after a shrink — the browser clamps it to the
 * new `scrollHeight - clientHeight` synchronously, in the same layout pass
 * that produces the shrink, before any JS runs. See
 * SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md §2 for the full argument. The
 * only place easing can happen is the content layer, BEFORE that layout
 * pass — freeze the shrinking element at its old height, then ease it down,
 * so the browser's clamp lands in many imperceptible per-frame steps
 * instead of one visible jump. That is what `flip()` below does; it is not
 * a novel technique, just `ToolOverlayLog.tsx`'s existing `flipHeight()`
 * moved here and given a shared policy layer.
 *
 * ## Two entry points, two call shapes
 *
 * `withHeightContinuity(el, mutate)` — the mutation is something YOU
 * trigger (e.g. a signal write whose effect runs synchronously). Measure
 * before, run it, measure after.
 *
 * `beginHeightContinuity(el)` — the mutation lands asynchronously (a
 * throttled re-render's trailing commit, or a reactive effect reacting to a
 * prop that changed from OUTSIDE, where there is no single call you make
 * that "is" the mutation). Call this right before the mutation is set in
 * motion; it captures the current height and returns a commit function —
 * call that once the mutation has landed in the DOM.
 *
 * Both close over their OWN `fromPx` in their own call's closure — neither
 * reads or writes any state shared across calls, which shrinks (but does
 * NOT eliminate) the node-identity-reset class of bug: the ORIGINAL version
 * of this comment claimed callers would never need a `prevNodeId`-style
 * guard because of this. That was wrong, corrected once `ToolOverlayLog.tsx`
 * actually migrated (step 3): a streaming-buffer cap-advance can still swap
 * a different node into the same DOM slot between calling
 * `beginHeightContinuity` and calling the commit function it returned — the
 * CALLER still holds that returned closure across the swap in its own
 * variable, and this module has no way to know the node identity changed
 * underneath it. `ToolOverlayLog.tsx` keeps its own `lastNodeId` guard for
 * exactly this — simplified from the old code (no `lastMeasuredHeight`/
 * `heightStale` to carry, just discard the pending commit on a swap), but
 * still necessary. What this module's closure-per-call design DOES remove
 * is the OTHER half of the old bug: a stale `lastMeasuredHeight` baseline
 * leaking from the outgoing node into whatever the incoming node's `commit`
 * eventually reads as `toPx` — that's now impossible by construction, since
 * `fromPx` is captured fresh in each `beginHeightContinuity` call.
 */

import { atoms } from "@/app/store/global";

const HEIGHT_FLIP_MS = 150;
const EASE = "cubic-bezier(0.4, 0, 0.2, 1)";

/**
 * Above this magnitude, skip the ease and let the mutation land instantly.
 * The 08-22 findings doc's whole-pane 0px collapses were 21,000-44,000px —
 * animating that at normal speed reads as absurd, and slowing the
 * per-pixel rate down to compensate would make an ordinary small shrink
 * feel sluggish. A shrink this large is a different phenomenon (§3 of that
 * doc calls it "categorically different... not a third data point for the
 * same mechanism") that a content-layer ease was never meant to smooth
 * over — this cap keeps this module from trying anyway.
 */
const MAX_ANIMATED_DELTA_PX = 2000;

/** One in-flight cancel fn per element, so re-entry (a second mutation
 *  landing before the first ease finishes) cancels the previous transition
 *  instead of stacking two `height` transitions on the same element. */
const inFlight = new WeakMap<HTMLElement, () => void>();

/**
 * Can `el`'s height be usefully measured and FLIPped right now? Only one
 * case says no: `content-visibility: hidden` on `el` OR ANY ANCESTOR —
 * reading `offsetHeight` there would force a synchronous layout of a
 * subtree the browser has deliberately skipped laying out, which both
 * emits a console warning AND produces a measurement that doesn't reflect
 * what will actually render once visible again.
 *
 * Walks the ancestor chain rather than checking only `el` itself because
 * `content-visibility` is a NON-INHERITED property (codex P1, PR #2954):
 * for the intended `ToolOverlayLog` migration, `el` is
 * `.agent-tool-overlay-log`, but `.agent-tool-panel--hidden` (the ancestor
 * that actually sets `content-visibility: hidden`, `_document-nodes.scss`)
 * is a DIFFERENT element — confirmed directly, `ToolOverlayLog.tsx`'s own
 * current code walks `scrollRef.closest(".agent-tool-panel")` to reach it,
 * proving it's an ancestor, not the element itself. `getComputedStyle(el)`
 * on the descendant reports the descendant's OWN value (the initial
 * `visible`, absent an explicit override) regardless of an ancestor
 * skipping its layout — checking only `el` would never detect the one case
 * this function exists to catch.
 *
 * A zero-size element (`display: none`, or genuinely 0px tall) does NOT
 * need a separate branch here — `shouldAnimate`'s own `fromPx <= 0` guard
 * already refuses to FLIP from a zero "from" height, and a zero "to"
 * height is just an ordinary small-delta shrink. An earlier version of
 * this function had a redundant `offsetHeight > 0 || offsetWidth > 0`
 * branch; removing it changed no observable behavior (confirmed by
 * deleting it and running the suite — nothing failed), which is exactly
 * why it was worth deleting rather than keeping "for safety."
 *
 * Deliberately reads the COMPUTED `content-visibility` at each ancestor,
 * not a class name or a `MutationObserver`-driven signal tracking one —
 * this is the fix for the bug SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md
 * §3a traces in `ToolOverlayLog.tsx`'s current `panelHidden`: a
 * `MutationObserver` on a `--hidden` class flips instantly, but the CSS
 * collapse it triggers uses `content-visibility ... allow-discrete`, which
 * doesn't actually take effect until the END of that transition — so for
 * the whole transition window, class-based detection says "hidden" while
 * the computed style (and the rendered pixels) still say otherwise.
 * Reading the computed style directly has no such lag.
 */
function isMeasurable(el: HTMLElement): boolean {
    for (let node: Element | null = el; node; node = node.parentElement) {
        if (getComputedStyle(node).contentVisibility === "hidden") return false;
    }
    return true;
}

function shouldAnimate(fromPx: number, toPx: number): boolean {
    if (atoms.prefersReducedMotionAtom()) return false;
    if (fromPx <= 0) return false; // nothing to freeze FROM — first paint, not a resize
    const delta = Math.abs(toPx - fromPx);
    return delta > 1 && delta <= MAX_ANIMATED_DELTA_PX;
}

/**
 * Classic FLIP height transition: freeze `el` at `fromPx` (forcing a reflow
 * so the browser commits it before animating), then ease to `toPx`,
 * clearing the inline style once the transition ends so the box goes back
 * to tracking its content naturally. `overflow-y` is forced to `hidden` for
 * the transition's duration only, so an internal scrollbar doesn't flash
 * on/off as the height passes through intermediate values.
 */
function flip(el: HTMLElement, fromPx: number, toPx: number): void {
    // No cancel-in-flight check here — by the time this runs, every call
    // site has already cancelled any prior transition on `el` BEFORE
    // measuring (see `withHeightContinuity`/`beginHeightContinuity`'s own
    // comments for why measuring first is itself the bug). Cancelling here
    // too would be redundant and one JS-visible tick too late to fix a
    // measurement that already happened.
    el.style.transition = "none";
    el.style.height = `${fromPx}px`;
    el.style.overflowY = "hidden";
    void el.offsetHeight; // force reflow so the "from" height commits before animating
    el.style.transition = `height ${HEIGHT_FLIP_MS}ms ${EASE}`;
    const raf = requestAnimationFrame(() => {
        el.style.height = `${toPx}px`;
    });
    const onEnd = (e: TransitionEvent): void => {
        if (e.target === el && e.propertyName === "height") cleanup();
    };
    const cleanup = (): void => {
        cancelAnimationFrame(raf);
        el.style.transition = "";
        el.style.height = "";
        el.style.overflowY = "";
        el.removeEventListener("transitionend", onEnd);
        inFlight.delete(el);
    };
    el.addEventListener("transitionend", onEnd);
    inFlight.set(el, cleanup);
}

/**
 * Cancel any transition already in flight on `el`, BEFORE measuring it.
 * Required, not optional (codex P1, PR #2954): while a prior FLIP is still
 * running, `el` has an explicit inline `height` pinning its rendered size —
 * an explicit height always overrides content-based sizing, so
 * `el.offsetHeight` reports the PINNED value regardless of what the
 * content actually is, for BOTH the "from" read (would report a stale
 * pinned value instead of the true current height) and the "to" read after
 * `mutate()` runs (a content change doesn't move a height-pinned box at
 * all, so it would report the SAME pinned value again, making `shouldAnimate`
 * see a zero delta and skip animating a real change — leaving `el` wrongly
 * pinned at the stale height until the ORIGINAL transition's own
 * `transitionend` eventually fires and clears it, at which point the box
 * snaps instantly to the true height — an uncontrolled jump, exactly what
 * this module exists to prevent). Cancelling first clears the pin so every
 * subsequent read in this call reflects reality.
 */
function cancelInFlight(el: HTMLElement): void {
    inFlight.get(el)?.();
}

/** Default height read — `offsetHeight`, the rendered box. Correct for the
 *  common case (a row that simply grows/shrinks with its own content, e.g.
 *  `AgentDocumentVirtualList`'s row-height sampling), wrong for an element
 *  that scrolls its OWN overflow inside an ancestor's `max-height` — for
 *  that shape, `offsetHeight` is clamped to whatever's left of the
 *  ancestor's budget and stops changing once content exceeds it, while
 *  `scrollHeight` keeps reflecting the true content height. That's exactly
 *  `ToolOverlayLog.tsx`'s `.agent-tool-overlay-log` (bounded by
 *  `.agent-tool-panel`'s `max-height: 50vh`, `overflow-y: auto` on itself)
 *  — the element this module's first real migration target needs to FLIP,
 *  and precisely the large-shrink case (a long raw chunk log collapsing to
 *  a short compact result) this whole effort exists to fix. Callers with
 *  that shape must pass `measure: (el) => el.scrollHeight` explicitly; the
 *  default stays `offsetHeight` rather than switching everyone to
 *  `scrollHeight`, since for a non-scrolling element the two are normally
 *  identical and `offsetHeight` is the cheaper, more universally correct
 *  read (a `position: absolute`/zero-overflow row has no scrollHeight
 *  distinct from its rendered size to begin with). */
const DEFAULT_MEASURE = (el: HTMLElement): number => el.offsetHeight;

/**
 * Run `mutate` (assumed to synchronously produce `el`'s new rendered
 * height — true for a plain Solid signal write, per the same synchronous-
 * effect guarantee `ToolOverlayLog.tsx`'s own comments already rely on),
 * easing `el`'s height across the change if it's animatable. Falls straight
 * through to `mutate()` with no measurement at all when `el` isn't
 * currently measurable — there is nothing to FLIP FROM.
 *
 * `measureForGating` (codex P2, PR #2962): the delta `MAX_ANIMATED_DELTA_PX`
 * bounds must be the VISUALLY RENDERED one, not necessarily the same number
 * `measure` produces. For `ToolOverlayLog.tsx`'s `.agent-tool-overlay-log`,
 * `measure` is `scrollHeight` (see `DEFAULT_MEASURE`'s doc comment for why) —
 * but a long raw chunk log easily exceeds several thousand px of
 * `scrollHeight` (the 1,000-line output cap alone gets there), while the
 * box's actual RENDERED shrink is bounded by the ancestor panel's
 * `max-height: 50vh` to at most a few hundred px. Gating the magnitude cap
 * on the unclamped `scrollHeight` delta would skip animating exactly the
 * long-output transitions this module exists to smooth, for a reason
 * (`MAX_ANIMATED_DELTA_PX`) that was never about THIS element's visible
 * shrink at all — it exists for the OTHER, unrelated whole-pane-collapse
 * case (see that constant's own doc comment). Defaults to `measure` — most
 * callers have no scrolling/clamping divergence, so the two numbers are the
 * same and this parameter is a no-op for them.
 */
export function withHeightContinuity(
    el: HTMLElement,
    mutate: () => void,
    measure: (el: HTMLElement) => number = DEFAULT_MEASURE,
    measureForGating: (el: HTMLElement) => number = measure,
): void {
    cancelInFlight(el);
    if (!isMeasurable(el)) {
        mutate();
        return;
    }
    const fromPx = measure(el);
    const fromGatePx = measureForGating(el);
    mutate();
    const toPx = measure(el);
    const toGatePx = measureForGating(el);
    if (shouldAnimate(fromGatePx, toGatePx)) flip(el, fromPx, toPx);
}

/**
 * Two-phase variant for a mutation you can't wrap in a single synchronous
 * call — capture `el`'s current height now, and return a function to call
 * once the mutation has actually landed in the DOM. Calling the returned
 * function is what performs the measure-after + FLIP; if it's never called,
 * nothing happens (no timer, no listener left running).
 *
 * Cancels an in-flight transition at BOTH capture and commit time, not just
 * once — a different mutation could start a fresh flip on the same `el`
 * during the (potentially long) gap between calling this and calling the
 * function it returns, and the "to" read needs the same unpinning the
 * "from" read does, for the identical reason (see `cancelInFlight`).
 *
 * `measureForGating` — see `withHeightContinuity`'s doc comment; same
 * reasoning, applied at both capture and commit time.
 */
export function beginHeightContinuity(
    el: HTMLElement,
    measure: (el: HTMLElement) => number = DEFAULT_MEASURE,
    measureForGating: (el: HTMLElement) => number = measure,
): (this: void) => void {
    cancelInFlight(el);
    if (!isMeasurable(el)) {
        return () => {};
    }
    const fromPx = measure(el);
    const fromGatePx = measureForGating(el);
    return () => {
        cancelInFlight(el);
        if (!isMeasurable(el)) return;
        const toPx = measure(el);
        const toGatePx = measureForGating(el);
        if (shouldAnimate(fromGatePx, toGatePx)) flip(el, fromPx, toPx);
    };
}
