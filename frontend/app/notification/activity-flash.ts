// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Activity flash — the visual twin of a tool-call tone. Every tone that
 * passes its policy gates also pulses the one on-screen element that
 * identifies where it came from, so a tone from a pane you can't see can be
 * traced to its window tab or pane-header pill.
 *
 * Spec: docs/specs/SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md.
 *
 * Deliberately store-free: the sound service decides the route (it already
 * owns the focus + window-focus reads the gates use) and consumers
 * (tab.tsx, PaneTabStrip.tsx) decide whether a target is theirs. That keeps
 * this module a pure routing rule + a subscriber set + one animation helper,
 * all unit-testable without a layout or a MOS cache.
 */

/**
 * `tab` — the source is in a background window tab: pulse that tab.
 * `pane-tab` — the source is in the active window tab: pulse its own pill
 * in its pane's header (flashing the active window tab would not say which
 * pane it was).
 */
export type FlashTarget = { kind: "tab" | "pane-tab"; blockId: string };

export interface FlashRouteInput {
    /** The source block is a member of the currently active window tab. */
    sourceInActiveTab: boolean;
    /** The source block is the focused block. */
    sourceFocused: boolean;
    /** The OS window has focus. */
    windowFocused: boolean;
}

/** The routing rule, spec §2.1. `null` = nothing to flash. */
export function flashTargetFor(blockId: string, input: FlashRouteInput): FlashTarget | null {
    // Focused pane in a focused window: its own transcript is already in
    // front of the user — a flash would add noise, not information.
    if (input.sourceFocused && input.windowFocused) return null;
    return { kind: input.sourceInActiveTab ? "pane-tab" : "tab", blockId };
}

// ── Bus ──────────────────────────────────────────────────────────────────
//
// A plain subscriber set, NOT a Solid signal: a signal would re-run a
// reactive computation in every tab and pill on every tool call. Each
// subscriber does one cheap equality/membership check and returns.

type FlashListener = (target: FlashTarget) => void;
const listeners = new Set<FlashListener>();

export function emitActivityFlash(target: FlashTarget): void {
    for (const l of listeners) {
        try {
            l(target);
        } catch (e) {
            console.warn("[activity-flash] listener threw", e);
        }
    }
}

export function onActivityFlash(listener: FlashListener): () => void {
    listeners.add(listener);
    return () => {
        listeners.delete(listener);
    };
}

// ── Animation ────────────────────────────────────────────────────────────
//
// Envelope: instant attack to a low peak, short hold, ease-out decay.
// Instant attack is what reads as "that one, just now"; the long tail keeps
// it soft.
//
// Why this cannot strobe (WCAG 2.3.1, three flashes per second): a
// re-trigger inside the decay restarts from PEAK while the overlay is still
// bright, a small luminance step, so continuous activity reads as a steady
// glow. A full dark→peak transition needs the previous flash to have
// decayed, which takes FLASH_DURATION_MS — at most ~1.4 full flashes/s —
// and a 0.22-opacity accent overlay is far below the luminance change that
// criterion counts in the first place.

export const FLASH_PEAK_OPACITY = 0.22;
export const FLASH_HOLD_MS = 80;
export const FLASH_DURATION_MS = 700;
/** At most one restart per element per this window; extras are dropped. */
export const FLASH_THROTTLE_MS = 150;
/** Class the reduced-motion path toggles; the stylesheets hold the overlay at peak while it is set. */
export const FLASH_STATIC_CLASS = "activity-flash-static";

const FLASH_KEYFRAMES: Keyframe[] = [
    { opacity: FLASH_PEAK_OPACITY, offset: 0 },
    { opacity: FLASH_PEAK_OPACITY, offset: FLASH_HOLD_MS / FLASH_DURATION_MS, easing: "ease-out" },
    { opacity: 0, offset: 1 },
];

interface RunningFlash {
    startedAt: number;
    anim: Animation | null;
    timer: ReturnType<typeof setTimeout> | null;
}
const running = new WeakMap<Element, RunningFlash>();

function prefersReducedMotion(): boolean {
    // matchMedia directly, not `atoms.prefersReducedMotionAtom` — that atom
    // is a hard-coded `false` stub today (store/global.ts), and this path is
    // driven from JS, so the app's CSS-only reduced-motion rules never see it.
    return typeof window !== "undefined" && window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true;
}

function now(): number {
    return typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();
}

/**
 * Pulse the `::before` overlay of `el`. The element's stylesheet owns the
 * overlay (position, color, `opacity: 0` at rest, `pointer-events: none`);
 * this only drives its opacity.
 *
 * Web Animations API rather than a CSS class toggle: re-triggering a CSS
 * keyframe animation needs a forced reflow between remove and re-add, and
 * a forced layout on every tool call is exactly the cost the agent-pane
 * typing work (PR #3599) removed.
 */
export function flashElement(el: HTMLElement): void {
    const t = now();
    const prev = running.get(el);
    if (prev && t - prev.startedAt < FLASH_THROTTLE_MS) return;
    prev?.anim?.cancel();
    if (prev?.timer != null) clearTimeout(prev.timer);

    if (prefersReducedMotion()) {
        // Same signal (which tab), no motion: show at peak, then remove.
        el.classList.add(FLASH_STATIC_CLASS);
        const timer = setTimeout(() => {
            el.classList.remove(FLASH_STATIC_CLASS);
            running.delete(el);
        }, FLASH_DURATION_MS);
        running.set(el, { startedAt: t, anim: null, timer });
        return;
    }

    el.classList.remove(FLASH_STATIC_CLASS);
    if (typeof el.animate !== "function") return;
    const anim = el.animate(FLASH_KEYFRAMES, {
        duration: FLASH_DURATION_MS,
        pseudoElement: "::before",
    });
    running.set(el, { startedAt: t, anim, timer: null });
}

// ── Test helpers (NEVER call from production) ────────────────────────────

export function __resetActivityFlash(): void {
    listeners.clear();
}
