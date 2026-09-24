// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Activity flash — the visual twin of a tool-call tone. Every tone that
 * passes its policy gates also "clicks" the source's own pill in its pane
 * header (a brightened, more saturated version of the pane's color) and,
 * more subtly, its window tab (a slight lift of the tab's own color). Both
 * appear instantly and fade out fast.
 *
 * Spec: docs/specs/SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md.
 *
 * Deliberately store-free: consumers (tab.tsx, PaneTabStrip.tsx) decide
 * whether an event is theirs and which color to click with; this module is
 * a subscriber set plus one animation helper.
 */

/** One tool-call tone from `blockId`. */
export type FlashTarget = { blockId: string };

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
// A click: instant attack to full strength, a hold just long enough to
// register the color, then a fast ease-out. Each tone is its own click; a
// tone that lands mid-decay restarts from peak.
//
// The keyframes run the overlay from 1 to 0. HOW strong "full strength" is
// lives in each stylesheet, as the alpha of its overlay fill: the pane pill
// is a strong click in the pane's color, the window tab a faint tint.
//
// Photosensitivity (WCAG 2.3.1): the general flash threshold only applies
// once the flashing area reaches roughly a 341×256 px block (25% of a 10°
// field at typical viewing distance). A window tab (~200×33 px) or a pane
// pill (~120×20 px) is a small fraction of that, so repeated clicks on one
// target stay inside the small-safe-area exemption. The throttle below still
// caps a single target at 10 restarts/s.
//
// Not gated on prefers-reduced-motion: this only animates opacity in
// place — nothing moves, scales or slides, which is what that preference
// guards against — and a fade is the substitution reduced-motion guidance
// itself recommends.

export const FLASH_PEAK_OPACITY = 1;
export const FLASH_HOLD_MS = 40;
export const FLASH_DURATION_MS = 300;
/** At most one restart per element per this window; extras are dropped. */
export const FLASH_THROTTLE_MS = 100;
/** Inline custom property the stylesheets brighten into the flash color. */
export const FLASH_BASE_COLOR_VAR = "--activity-flash-base";

const FLASH_KEYFRAMES: Keyframe[] = [
    { opacity: FLASH_PEAK_OPACITY, offset: 0 },
    { opacity: FLASH_PEAK_OPACITY, offset: FLASH_HOLD_MS / FLASH_DURATION_MS, easing: "ease-out" },
    { opacity: 0, offset: 1 },
];

interface RunningFlash {
    startedAt: number;
    anim: Animation | null;
}
const running = new WeakMap<Element, RunningFlash>();

function now(): number {
    return typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();
}

/**
 * Click the `::before` overlay of `el`. The element's stylesheet owns the
 * overlay (position, `opacity: 0` at rest, `pointer-events: none`, and the
 * brightening of `--activity-flash-base` into its fill); this drives its
 * opacity and, when `baseColor` is given, which color it brightens.
 *
 * Web Animations API rather than a CSS class toggle: re-triggering a CSS
 * keyframe animation needs a forced reflow between remove and re-add, and
 * a forced layout on every tool call is exactly the cost the agent-pane
 * typing work (PR #3599) removed.
 */
export function flashElement(el: HTMLElement, baseColor?: string | null): void {
    const t = now();
    const prev = running.get(el);
    if (prev && t - prev.startedAt < FLASH_THROTTLE_MS) return;
    prev?.anim?.cancel();

    if (baseColor) el.style.setProperty(FLASH_BASE_COLOR_VAR, baseColor);
    else el.style.removeProperty(FLASH_BASE_COLOR_VAR);

    if (typeof el.animate !== "function") return;
    const anim = el.animate(FLASH_KEYFRAMES, {
        duration: FLASH_DURATION_MS,
        pseudoElement: "::before",
    });
    running.set(el, { startedAt: t, anim });
}

// ── Test helpers (NEVER call from production) ────────────────────────────

export function __resetActivityFlash(): void {
    listeners.clear();
}
