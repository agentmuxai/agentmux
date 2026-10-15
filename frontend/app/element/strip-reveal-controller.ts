// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// When a strip (pane tabs or document tabs) scrolls its selected tab into view
// (docs/specs/SPEC_PANE_TAB_STRIP_REVEAL_SELECTED_2026_10_15.md §3-§5).
//
// A reveal is requested, never continuous: it runs once, a frame after the
// request, and the user's own wheel or drag on the strip is never fought, and
// cancels a smooth reveal in flight. Requests read no layout; a strip with no
// overflow does nothing at all for an interaction or a focus.

import { createEffect, on, onCleanup, onMount, type Accessor } from "solid-js";
import { shouldReduceMotion } from "@/app/motion/motion";
import { registerPaneStrip, type StripRevealReason } from "./pane-strip-registry";
import { revealInStrip, STRIP_EDGE_FADE_PX } from "./strip-reveal";

/** A landing or drag animation on a tab (PaneTabStrip.tsx's LANDING_BOUNCE_MS) moves its rect: wait it out. */
const ANIMATION_SETTLE_MS = 420;
/** Without `scrollend`, a smooth reveal is assumed over after this long. */
const SMOOTH_FALLBACK_MS = 800;

const SMOOTH_REASONS: readonly StripRevealReason[] = ["selected", "focus", "interaction"];

export interface StripRevealInput {
    strip: Accessor<HTMLElement | undefined>;
    /** The selected tab, and the tabs in order, to find the first and last. */
    activeId: Accessor<string | null>;
    tabIds: Accessor<string[]>;
    /** False while the strip's pane is hidden: no geometry is read or written then. */
    shown: Accessor<boolean>;
    /** `createScrollEdges`' signals: whether the strip overflows at all. */
    overflows: Accessor<boolean>;
}

export interface StripReveal {
    request(reason: StripRevealReason): void;
    /** The user scrolled the strip, or touched it mid-reveal. */
    noteUserScroll(): void;
    /** A pointer went down on the strip: a reveal in flight stops, nothing else changes. */
    notePointerDown(): void;
    /** The strip's width transition (`animateWidth`) started or ended. */
    setWidthAnimating(animating: boolean): void;
}

function findTab(strip: HTMLElement, id: string): HTMLElement | null {
    for (const el of strip.querySelectorAll<HTMLElement>("[data-tab-id]")) {
        if (el.dataset.tabId === id) return el;
    }
    return null;
}

export function createStripReveal(input: StripRevealInput): StripReveal {
    let frame: number | undefined;
    let settleTimer: ReturnType<typeof setTimeout> | undefined;
    let smoothTimer: ReturnType<typeof setTimeout> | undefined;
    let wantSmooth = false;
    let userScrolled = false;
    let pointerDown = false;
    let smoothing = false;
    let widthAnimating = false;
    let recheckAfterWidth = false;
    // A resize or rename arrived while a smooth reveal travelled: look again when it ends.
    let layoutDeferred = false;
    let waited = false;
    let observedTabs = new Set<Element>();
    let tabListKey: string | undefined;

    const stopSmooth = () => {
        smoothing = false;
        clearTimeout(smoothTimer);
        if (layoutDeferred) {
            layoutDeferred = false;
            // Not under a held pointer (a scrollbar drag) or after the user's own scroll.
            if (!userScrolled && !pointerDown && input.shown()) request("layout");
        }
    };
    // Any scroll write cancels a smooth scroll in flight; this one stays where it is.
    const cancelInFlight = () => {
        if (frame != null) cancelAnimationFrame(frame);
        frame = undefined;
        clearTimeout(settleTimer);
        waited = false;
        const strip = input.strip();
        if (smoothing && strip) strip.scrollLeft = strip.scrollLeft;
        stopSmooth();
    };

    const run = () => {
        frame = undefined;
        const strip = input.strip();
        const id = input.activeId();
        const smooth = wantSmooth && !shouldReduceMotion("transition");
        wantSmooth = false;
        if (!strip || id == null || !input.shown()) return;
        // A just-landed or dragged tab is mid-animation: measure after it.
        if (!waited && strip.querySelector(".pane-tab--landing, .pane-tab--dragging")) {
            waited = true;
            wantSmooth = smooth;
            settleTimer = setTimeout(() => schedule(), ANIMATION_SETTLE_MS);
            return;
        }
        waited = false;
        const item = findTab(strip, id);
        if (!item) return;
        const ids = input.tabIds();
        const scrolled = revealInStrip(strip, item, ids[ids.length - 1] === id, {
            isFirst: ids[0] === id,
            edgeClearance: STRIP_EDGE_FADE_PX,
            behavior: smooth ? "smooth" : "instant",
        });
        // Only a scroll that started is in flight: a tab already in view holds nothing off.
        if (smooth && scrolled) {
            smoothing = true;
            clearTimeout(smoothTimer);
            smoothTimer = setTimeout(stopSmooth, SMOOTH_FALLBACK_MS);
        }
        if (widthAnimating) recheckAfterWidth = true;
    };
    const schedule = () => {
        if (frame == null) frame = requestAnimationFrame(run);
    };

    const request = (reason: StripRevealReason) => {
        if (!input.shown()) return; // a hidden strip reveals once, when it is shown
        if (reason === "layout") {
            // Never over the user's scroll or a smooth reveal; during a width
            // transition tab widths still change, so measure once when it ends.
            if (widthAnimating) recheckAfterWidth = true;
            if (smoothing && !userScrolled) layoutDeferred = true;
            if (userScrolled || widthAnimating || smoothing) return;
        } else {
            userScrolled = false;
            if ((reason === "interaction" || reason === "focus") && !input.overflows()) return;
        }
        if (SMOOTH_REASONS.includes(reason)) wantSmooth = true;
        schedule();
    };

    // Mount, and a hidden pane becoming shown: an instant reveal, once.
    createEffect(
        on(input.shown, (shown) => {
            if (shown) request("visible");
        })
    );
    createEffect(on(input.activeId, () => request("selected"), { defer: true }));

    // The strip or its selected tab resized (window resize, a sibling closed, a rename).
    onMount(() => {
        const strip = input.strip();
        if (!strip) return;
        const onScrollEnd = () => stopSmooth();
        const onScroll = () => {
            if (pointerDown) userScrolled = true;
        };
        const onPointerUp = () => {
            pointerDown = false;
        };
        strip.addEventListener("scrollend", onScrollEnd);
        strip.addEventListener("scroll", onScroll, { passive: true });
        window.addEventListener("pointerup", onPointerUp, true);
        window.addEventListener("pointercancel", onPointerUp, true);
        onCleanup(() => {
            strip.removeEventListener("scrollend", onScrollEnd);
            strip.removeEventListener("scroll", onScroll);
            window.removeEventListener("pointerup", onPointerUp, true);
            window.removeEventListener("pointercancel", onPointerUp, true);
        });
        const ro = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(() => request("layout"));
        ro?.observe(strip);
        // Every tab, not only the selected one: a tab before it that closes, is
        // reordered, or is renamed to another width moves the selected tab without
        // resizing it or the strip. A tab list that changed is a layout change too.
        createEffect(() => {
            const key = input.tabIds().join("\u0000");
            for (const el of observedTabs) ro?.unobserve(el);
            observedTabs = new Set(strip.querySelectorAll("[data-tab-id]"));
            for (const el of observedTabs) ro?.observe(el);
            if (tabListKey !== undefined && tabListKey !== key) request("layout");
            tabListKey = key;
        });
        onCleanup(() => ro?.disconnect());
    });

    onCleanup(() => {
        if (frame != null) cancelAnimationFrame(frame);
        clearTimeout(settleTimer);
        clearTimeout(smoothTimer);
    });

    return {
        request,
        noteUserScroll() {
            userScrolled = true;
            cancelInFlight();
        },
        notePointerDown() {
            pointerDown = true;
            cancelInFlight();
        },
        setWidthAnimating(animating) {
            widthAnimating = animating;
            if (!animating && recheckAfterWidth) {
                recheckAfterWidth = false;
                if (userScrolled || !input.shown()) return;
                // A smooth reveal still travelling aims at the settled widths too.
                wantSmooth = smoothing;
                schedule();
            }
        },
    };
}

/** Registers a strip's reveal with its pane, once mounted (pane-strip-registry.ts). */
export function registerStripWithPane(strip: Accessor<HTMLElement | undefined>, reveal: StripReveal): void {
    onMount(() => {
        const el = strip();
        if (!el) return;
        const off = registerPaneStrip(el, {
            revealSelected: reveal.request,
            contains: (node) => !!node && el.contains(node),
        });
        onCleanup(off);
    });
}
