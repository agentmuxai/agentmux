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
import { scaledMs, shouldReduceMotion } from "@/app/motion/motion";
import { DURATION_BASE_MS } from "@/app/motion/tokens";
import { registerPaneStrip, type StripRevealReason } from "./pane-strip-registry";
import { cancelStripScroll, revealInStrip, STRIP_EDGE_FADE_PX } from "./strip-reveal";

/** A landing or drag animation on a tab (PaneTabStrip.tsx's LANDING_BOUNCE_MS) moves its rect: wait it out. */
const ANIMATION_SETTLE_MS = 420;

/** A reveal that comes from the person (a selection, a focus, a click or key in the pane) travels; a mount and a resize do not. */
const SMOOTH_REASONS: readonly StripRevealReason[] = ["selected", "focus", "interaction"];

export interface StripRevealInput {
    /** The box that scrolls: the strip, or a box around it that scrolls it with other content (strip-scroller.ts). */
    strip: Accessor<HTMLElement | undefined>;
    /** The selected tab, and the tabs in order, to find the first and last. */
    activeId: Accessor<string | null>;
    tabIds: Accessor<string[]>;
    /** False while the strip's pane is hidden: no geometry is read or written then. */
    shown: Accessor<boolean>;
    /** `createScrollEdges`' signals: whether the strip overflows at all. */
    overflows: Accessor<boolean>;
    /** Width of the fade over a scrollable edge, which a revealed tab stays clear of. Default: the strip's own. */
    edgeClearance?: Accessor<number>;
    /** False when the box holds more after the tabs (the header's range), so the last tab is not the box's end. Default true. */
    lastTabEndsStrip?: Accessor<boolean>;
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

/** What a strip's reveal keeps between requests: the frame and timer it waits on, and what it has been told. */
interface RevealCore {
    input: StripRevealInput;
    frame: number | undefined;
    settleTimer: ReturnType<typeof setTimeout> | undefined;
    wantSmooth: boolean;
    userScrolled: boolean;
    pointerDown: boolean;
    smoothing: boolean;
    widthAnimating: boolean;
    recheckAfterWidth: boolean;
    // A resize or rename arrived while a smooth reveal travelled: look again when it ends.
    layoutDeferred: boolean;
    waited: boolean;
    // A selection, a mount or a layout change is waiting: the reveal runs wherever the keyboard focus is.
    mustReveal: boolean;
    observedTabs: Set<Element>;
    tabListKey: string | undefined;
}

function createRevealCore(input: StripRevealInput): RevealCore {
    return {
        input,
        frame: undefined,
        settleTimer: undefined,
        wantSmooth: false,
        userScrolled: false,
        pointerDown: false,
        smoothing: false,
        widthAnimating: false,
        recheckAfterWidth: false,
        layoutDeferred: false,
        waited: false,
        mustReveal: false,
        observedTabs: new Set<Element>(),
        tabListKey: undefined,
    };
}

function stopSmooth(c: RevealCore): void {
    c.smoothing = false;
    if (c.layoutDeferred) {
        c.layoutDeferred = false;
        // Not under a held pointer (a scrollbar drag) or after the user's own scroll.
        if (!c.userScrolled && !c.pointerDown && c.input.shown()) request(c, "layout");
    }
}

// Any scroll write cancels a smooth scroll in flight; this one stays where it is.
function cancelInFlight(c: RevealCore): void {
    if (c.frame != null) cancelAnimationFrame(c.frame);
    c.frame = undefined;
    clearTimeout(c.settleTimer);
    c.waited = false;
    const strip = c.input.strip();
    if (strip) cancelStripScroll(strip);
    stopSmooth(c);
}

function run(c: RevealCore): void {
    c.frame = undefined;
    const strip = c.input.strip();
    const id = c.input.activeId();
    const durationMs = scaledMs(DURATION_BASE_MS);
    const smooth = c.wantSmooth && !shouldReduceMotion("transition") && durationMs > 0;
    c.wantSmooth = false;
    const must = c.mustReveal;
    c.mustReveal = false;
    if (!strip || id == null || !c.input.shown()) return;
    // A focus or an interaction alone never moves the box from under the keyboard focus: the person is
    // working in it (a control they tabbed to, which the browser scrolled into view).
    if (!must && strip.contains(document.activeElement)) return;
    // A just-landed or dragged tab is mid-animation: measure after it.
    if (!c.waited && strip.querySelector(".pane-tab--landing, .pane-tab--dragging")) {
        c.waited = true;
        c.wantSmooth = smooth;
        c.mustReveal = must;
        c.settleTimer = setTimeout(() => schedule(c), scaledMs(ANIMATION_SETTLE_MS));
        return;
    }
    c.waited = false;
    const item = findTab(strip, id);
    if (!item) return;
    const ids = c.input.tabIds();
    const isLast = (c.input.lastTabEndsStrip?.() ?? true) && ids[ids.length - 1] === id;
    const scrolled = revealInStrip(strip, item, isLast, {
        isFirst: ids[0] === id,
        edgeClearance: c.input.edgeClearance?.() ?? STRIP_EDGE_FADE_PX,
        behavior: smooth ? "smooth" : "instant",
        durationMs,
        onSettled: () => stopSmooth(c),
    });
    // Only a tween that started is in flight (a tab in view, or an instant write, ended any before it; a resize that waited for it is covered by this look).
    if (!(c.smoothing = smooth && scrolled)) c.layoutDeferred = false;
    if (c.widthAnimating) c.recheckAfterWidth = true;
}

function schedule(c: RevealCore): void {
    if (c.frame == null) c.frame = requestAnimationFrame(() => run(c));
}

function request(c: RevealCore, reason: StripRevealReason): void {
    if (!c.input.shown()) return; // a hidden strip reveals once, when it is shown
    if (reason === "layout") {
        // Never over the user's scroll or a smooth reveal; during a width
        // transition tab widths still change, so measure once when it ends.
        if (c.widthAnimating) c.recheckAfterWidth = true;
        if (c.smoothing && !c.userScrolled) c.layoutDeferred = true;
        if (c.userScrolled || c.widthAnimating || c.smoothing) return;
    } else {
        c.userScrolled = false;
        if ((reason === "interaction" || reason === "focus") && !c.input.overflows()) return;
    }
    if (SMOOTH_REASONS.includes(reason)) c.wantSmooth = true;
    if (reason !== "interaction" && reason !== "focus") c.mustReveal = true;
    schedule(c);
}

function setWidthAnimating(c: RevealCore, animating: boolean): void {
    c.widthAnimating = animating;
    if (!animating && c.recheckAfterWidth) {
        c.recheckAfterWidth = false;
        if (c.userScrolled || !c.input.shown()) return;
        // A smooth reveal still travelling aims at the settled widths too.
        c.wantSmooth = c.smoothing;
        c.mustReveal = true;
        schedule(c);
    }
}

// The strip or its selected tab resized (window resize, a sibling closed, a rename).
function watchStrip(c: RevealCore): void {
    const strip = c.input.strip();
    if (!strip) return;
    const onScroll = () => {
        if (c.pointerDown) c.userScrolled = true;
    };
    const onPointerUp = () => {
        c.pointerDown = false;
    };
    strip.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("pointerup", onPointerUp, true);
    window.addEventListener("pointercancel", onPointerUp, true);
    onCleanup(() => {
        strip.removeEventListener("scroll", onScroll);
        window.removeEventListener("pointerup", onPointerUp, true);
        window.removeEventListener("pointercancel", onPointerUp, true);
    });
    const ro = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(() => request(c, "layout"));
    ro?.observe(strip);
    // Every tab, not only the selected one: a tab before it that closes, is
    // reordered, or is renamed to another width moves the selected tab without
    // resizing it or the strip. A tab list that changed is a layout change too.
    createEffect(() => {
        const key = c.input.tabIds().join("\u0000");
        for (const el of c.observedTabs) ro?.unobserve(el);
        c.observedTabs = new Set(strip.querySelectorAll("[data-tab-id]"));
        for (const el of c.observedTabs) ro?.observe(el);
        if (c.tabListKey !== undefined && c.tabListKey !== key) request(c, "layout");
        c.tabListKey = key;
    });
    onCleanup(() => ro?.disconnect());
}

function teardown(c: RevealCore): void {
    if (c.frame != null) cancelAnimationFrame(c.frame);
    clearTimeout(c.settleTimer);
    const strip = c.input.strip();
    if (strip) cancelStripScroll(strip);
}

export function createStripReveal(input: StripRevealInput): StripReveal {
    const c = createRevealCore(input);

    // Mount, and a hidden pane becoming shown: an instant reveal, once.
    createEffect(
        on(input.shown, (shown) => {
            if (shown) request(c, "visible");
        })
    );
    createEffect(on(input.activeId, () => request(c, "selected"), { defer: true }));

    onMount(() => watchStrip(c));
    onCleanup(() => teardown(c));

    return {
        request: (reason) => request(c, reason),
        noteUserScroll() {
            c.userScrolled = true;
            cancelInFlight(c);
        },
        notePointerDown() {
            c.pointerDown = true;
            cancelInFlight(c);
        },
        setWidthAnimating: (animating) => setWidthAnimating(c, animating),
    };
}

/** Registers a strip's reveal with its pane, once mounted (pane-strip-registry.ts). `strip` is the box that scrolls. */
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
