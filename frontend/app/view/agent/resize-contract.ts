// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A single content-resize contract for the agent pane — step 2 of
 * docs/specs/SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md.
 *
 * Landed with NO call sites yet (spec §5 step 2) — zero runtime effect. The
 * migration (step 3: `ToolOverlayLog.tsx`'s `flipHeight()`) is a separate,
 * reviewable-in-isolation change.
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
 * reads or writes any state shared across calls. This is what eliminates
 * the node-identity-reset class of bug by construction rather than by a
 * bespoke guard: `ToolOverlayLog.tsx`'s current `flipHeight()` needs a
 * `prevNodeId` check specifically because its baseline
 * (`lastMeasuredHeight`) is a variable shared across every effect
 * invocation, so a streaming-buffer cap-advance swapping a different node
 * into the same DOM slot without unmounting can make an unrelated old
 * height leak into a new node's first real transition. Neither entry point
 * here has an equivalent shared field for that to leak through.
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
 * case says no: `content-visibility: hidden` — reading `offsetHeight` there
 * would force a synchronous layout of a subtree the browser has
 * deliberately skipped laying out, which both emits a console warning AND
 * produces a measurement that doesn't reflect what will actually render
 * once visible again.
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
 * Deliberately reads the COMPUTED `content-visibility`, not a class name or
 * a `MutationObserver`-driven signal tracking one — this is the fix for the
 * bug SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md §3a traces in
 * `ToolOverlayLog.tsx`'s current `panelHidden`: a `MutationObserver` on a
 * `--hidden` class flips instantly, but the CSS collapse it triggers uses
 * `content-visibility ... allow-discrete`, which doesn't actually take
 * effect until the END of that transition — so for the whole transition
 * window, class-based detection says "hidden" while the computed style
 * (and the rendered pixels) still say otherwise. Reading the computed style
 * directly has no such lag.
 */
function isMeasurable(el: HTMLElement): boolean {
    return getComputedStyle(el).contentVisibility !== "hidden";
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
    inFlight.get(el)?.();
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
 * Run `mutate` (assumed to synchronously produce `el`'s new rendered
 * height — true for a plain Solid signal write, per the same synchronous-
 * effect guarantee `ToolOverlayLog.tsx`'s own comments already rely on),
 * easing `el`'s height across the change if it's animatable. Falls straight
 * through to `mutate()` with no measurement at all when `el` isn't
 * currently measurable — there is nothing to FLIP FROM.
 */
export function withHeightContinuity(el: HTMLElement, mutate: () => void): void {
    if (!isMeasurable(el)) {
        mutate();
        return;
    }
    const fromPx = el.offsetHeight;
    mutate();
    const toPx = el.offsetHeight;
    if (shouldAnimate(fromPx, toPx)) flip(el, fromPx, toPx);
}

/**
 * Two-phase variant for a mutation you can't wrap in a single synchronous
 * call — capture `el`'s current height now, and return a function to call
 * once the mutation has actually landed in the DOM. Calling the returned
 * function is what performs the measure-after + FLIP; if it's never called,
 * nothing happens (no timer, no listener left running).
 */
export function beginHeightContinuity(el: HTMLElement): (this: void) => void {
    if (!isMeasurable(el)) {
        return () => {};
    }
    const fromPx = el.offsetHeight;
    return () => {
        if (!isMeasurable(el)) return;
        const toPx = el.offsetHeight;
        if (shouldAnimate(fromPx, toPx)) flip(el, fromPx, toPx);
    };
}
