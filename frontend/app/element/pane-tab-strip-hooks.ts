// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What a tab strip does with its own size and scrolling: it animates its width across a tab-count change, and takes the wheel and touch as the person's scroll. */

import { createEffect, on, onCleanup, onMount } from "solid-js";
import { shouldReduceMotion } from "@/app/motion/motion";
import { DURATION_BASE_MS, EASE_OUT } from "@/app/motion/tokens";
import { stripWheelDelta } from "./strip-wheel";
import type { createStripReveal } from "./strip-reveal-controller";

// Matches the other reveal-gate/cross-fade durations added alongside this
// one in SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md §2.4.
const WIDTH_TRANSITION_MS = DURATION_BASE_MS;

type StripReveal = ReturnType<typeof createStripReveal>;

/** Opt-in (`animateWidth`) width animation of the strip across a tab-count change. */
export function useStripWidthAnimation(opts: {
    stripEl: () => HTMLDivElement | undefined;
    tabCount: () => number;
    animateWidth: () => boolean | undefined;
    reveal: StripReveal;
}): void {
    const { reveal } = opts;
    let lastMeasuredWidth: number | undefined;
    let widthResetTimeout: ReturnType<typeof setTimeout> | undefined;
    onCleanup(() => clearTimeout(widthResetTimeout));
    // FLIP-style width transition across a tab-count change (SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md
    // §2.4). Effects run after the DOM patch, so the box already has its NEW width; the previous run's
    // measurement is the "before". Hold the old width, force a reflow, then transition to the new one.
    // Not deferred: the first run records the initial width, or the 0-to-1-tab change would not animate.
    // A change that arrives mid-transition clears the pinned width first and holds from the box's
    // current visual width, or it would measure the interpolated value instead of the natural one.
    createEffect(
        on(
            opts.tabCount,
            () => {
                if (!opts.animateWidth()) return;
                const el = opts.stripEl();
                if (!el) return;
                const wasAnimating = widthResetTimeout !== undefined;
                const holdWidth = wasAnimating ? el.getBoundingClientRect().width : lastMeasuredWidth;
                clearTimeout(widthResetTimeout);
                reveal.setWidthAnimating(false);
                el.style.transition = "none";
                el.style.width = "";
                // Forces the reflow that reveals the TRUE natural width for
                // the current tab count — only trustworthy once the line
                // above has cleared any lingering override.
                const newWidth = el.getBoundingClientRect().width;
                if (holdWidth !== undefined && holdWidth !== newWidth && !shouldReduceMotion("transition")) {
                    el.style.width = `${holdWidth}px`;
                    el.getBoundingClientRect(); // force reflow before the transition kicks in
                    el.style.transition = `width ${WIDTH_TRANSITION_MS}ms ${EASE_OUT}`;
                    el.style.width = `${newWidth}px`;
                    reveal.setWidthAnimating(true);
                    widthResetTimeout = setTimeout(() => {
                        el.style.width = "";
                        el.style.transition = "";
                        reveal.setWidthAnimating(false);
                    }, WIDTH_TRANSITION_MS + 20);
                }
                lastMeasuredWidth = newWidth;
            }
        )
    );
}

/** The wheel, touch and pointer input on the strip, told to the reveal as the person's own scroll. */
export function useStripScrollInput(stripEl: () => HTMLDivElement | undefined, reveal: StripReveal): void {
    // A plain vertical wheel over the horizontally scrolling strip scrolls it sideways, as browsers' tab
    // bars do. Only when it really overflows and the gesture is vertical; a horizontal swipe and a strip
    // that fits are left alone. A real `addEventListener` with `{ passive: false }`, not the JSX
    // `onWheel`: Solid's delegated wheel listeners are passive, which makes `preventDefault()` a no-op.
    onMount(() => {
        const el = stripEl();
        if (!el) return;
        const handleWheel = (e: WheelEvent) => {
            if (el.scrollWidth <= el.clientWidth) return; // nothing to scroll: not the user scrolling it
            // The user's scroll: it stands until the next reveal, and stops one in flight.
            reveal.noteUserScroll();
            if (Math.abs(e.deltaX) >= Math.abs(e.deltaY)) return;
            e.preventDefault();
            el.scrollLeft += stripWheelDelta(e.deltaY, e.deltaMode);
        };
        const handleTouch = () => {
            if (el.scrollWidth > el.clientWidth) reveal.noteUserScroll();
        };
        const handlePointerDown = () => reveal.notePointerDown();
        el.addEventListener("wheel", handleWheel, { passive: false });
        el.addEventListener("touchmove", handleTouch, { passive: true });
        el.addEventListener("pointerdown", handlePointerDown, { passive: true });
        onCleanup(() => {
            el.removeEventListener("wheel", handleWheel);
            el.removeEventListener("touchmove", handleTouch);
            el.removeEventListener("pointerdown", handlePointerDown);
        });
    });
}
