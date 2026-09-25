// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Activity flash — the visual twin of a tool-call tone. Every tone that
 * passes its policy gates also "clicks" the source's own pill in its pane
 * header (a brightened, more saturated version of the pane's color) and,
 * more subtly, its window tab (a slight lift of the tab's own color).
 *
 * The flash follows its sound: one pulse per audible strike, at the
 * strike's own time and relative loudness (a two-note syllable pulses
 * twice, Edit's three-note syllable three times).
 *
 * Specs: docs/specs/SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md (targets,
 * colors, overlay) and SPEC_AGENT_ACTIVITY_FLASH_SOUND_SYNC_2026_09_24.md
 * (patterns, envelope, timing).
 *
 * Deliberately store-free and sound-free: consumers (tab.tsx,
 * PaneTabStrip.tsx) decide whether an event is theirs; the sound side
 * (sound/flash-patterns.ts) turns a sound into a pattern. This module is a
 * subscriber set plus the animation.
 */

/** One strike of a sound: when it lands, and how hard (0–1). */
export interface FlashStrike {
    atMs: number;
    intensity: number;
}

/** The strikes of one sound, sorted by `atMs`, the first at 0. */
export interface FlashPattern {
    strikes: FlashStrike[];
}

/**
 * One sound from `blockId`. `delayMs` is how long to wait before the
 * pattern's first strike so it lands with the audible sound, not ahead of it.
 * Negative when the sound is already that far in: the pattern resumes
 * mid-way instead of restarting behind it.
 */
export type FlashTarget = { blockId: string; pattern: FlashPattern; delayMs: number };

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

// ── Envelope ─────────────────────────────────────────────────────────────
//
// Each strike jumps to its intensity (the audio attack is 6–10 ms, under one
// frame), holds for one guaranteed 60 Hz frame, then eases out. Between
// strikes the decay bottoms out at a fraction of the strike's intensity
// exactly when the next strike lands; that dip is what makes two or three
// fast strikes read as separate pulses. After the last strike it fades to
// zero, keeping the 300 ms click length the owner approved.
//
// The value is the overlay's opacity. HOW strong opacity 1 is lives in each
// stylesheet, as the alpha of its overlay fill: the pane pill is a strong
// click in the pane's color, the window tab a faint tint.
//
// Photosensitivity (WCAG 2.3.1): a three-strike syllable is three flashes
// inside 124 ms, so a single target can exceed "three flashes in any one
// second". That is allowed by the small-safe-area exemption, which this
// design relies on entirely: the general flash threshold only applies once
// the combined flashing area reaches roughly 341×256 px (≈87,000 px², 25% of
// a 10° field at typical viewing distance). One agent flashes a pill
// (~120×20 px) plus a window tab (~200×33 px), ≈9,000 px², so about nine
// agents would have to strike in the same instant to reach it. The window
// tab's 0.12-alpha tint is likely below the 10% relative-luminance change
// that counts as a flash at all. The red-flash rule (saturated red pane
// colors) has the same area exemption.
//
// Not gated on prefers-reduced-motion: this only animates opacity in
// place — nothing moves, scales or slides, which is what that preference
// guards against — and a fade is the substitution reduced-motion guidance
// itself recommends.

/** Each strike stays at its intensity this long: one 60 Hz frame, whatever the vsync phase. */
export const FLASH_STRIKE_HOLD_MS = 20;
/** Between strikes, the decay reaches this fraction of the strike's intensity at the next onset. */
export const FLASH_INTER_STRIKE_FLOOR = 0.15;
/** After the last strike's hold, the fade to zero. */
export const FLASH_TAIL_MS = 280;
/** Keyframe sampling step: ≥120 Hz, finer than any display this runs on. */
export const FLASH_SAMPLE_STEP_MS = 8;
/**
 * The display pipeline's own delay (compositor plus scan-out, about a frame)
 * between an animation value changing and it being on screen. The flash
 * starts this much before the audible onset. Calibrate against an A/V
 * capture (sync spec §6).
 */
export const FLASH_VISUAL_LEAD_MS = 16;
/** Inline custom property the stylesheets brighten into the flash color. */
export const FLASH_BASE_COLOR_VAR = "--activity-flash-base";

/** CSS `ease-out`, i.e. cubic-bezier(0, 0, 0.58, 1), for x in [0, 1]. */
export function easeOut(x: number): number {
    if (x <= 0) return 0;
    if (x >= 1) return 1;
    // x(s) = 1.74·s²(1−s) + s³ is monotonic on [0, 1]; bisect for s.
    let lo = 0;
    let hi = 1;
    for (let i = 0; i < 24; i++) {
        const s = (lo + hi) / 2;
        const xs = 1.74 * s * s * (1 - s) + s * s * s;
        if (xs < x) lo = s;
        else hi = s;
    }
    const s = (lo + hi) / 2;
    return 3 * s * s - 2 * s * s * s;
}

/** How long a pattern stays visible, from its first strike. */
export function patternDurationMs(pattern: FlashPattern): number {
    const last = pattern.strikes.at(-1);
    return last ? last.atMs + FLASH_STRIKE_HOLD_MS + FLASH_TAIL_MS : 0;
}

/** Overlay opacity `t` ms after the pattern starts. Zero before and after it. */
export function patternEnvelopeAt(pattern: FlashPattern, t: number): number {
    const strikes = pattern.strikes;
    let k = -1;
    for (let i = 0; i < strikes.length && strikes[i].atMs <= t; i++) k = i;
    if (k < 0) return 0;
    const { atMs, intensity } = strikes[k];
    const holdEnd = atMs + FLASH_STRIKE_HOLD_MS;
    const next = strikes[k + 1];
    if (next) {
        if (t < holdEnd || next.atMs <= holdEnd) return intensity;
        const floor = intensity * FLASH_INTER_STRIKE_FLOOR;
        return intensity - (intensity - floor) * easeOut((t - holdEnd) / (next.atMs - holdEnd));
    }
    if (t < holdEnd) return intensity;
    return intensity * (1 - easeOut((t - holdEnd) / FLASH_TAIL_MS));
}

/** A pattern placed on the page clock: its first strike lands at `startAt`. */
export interface ScheduledFlash {
    startAt: number;
    pattern: FlashPattern;
}

/**
 * Overlay opacity at page time `t` for overlapping sounds: the brightest of
 * them, the way the loudest of several overlapping sounds dominates.
 */
export function envelopeAt(scheduled: readonly ScheduledFlash[], t: number): number {
    let v = 0;
    for (const s of scheduled) v = Math.max(v, patternEnvelopeAt(s.pattern, t - s.startAt));
    return v;
}

/**
 * Keyframes for page times [from, to]: a sample every FLASH_SAMPLE_STEP_MS,
 * plus exact points at each hold's end, and at each strike a pair of
 * keyframes with the same offset (the value just before, then the peak) for
 * the instant jump. The Web Animations API allows equal offsets.
 */
export function buildFlashKeyframes(scheduled: readonly ScheduledFlash[], from: number, to: number): Keyframe[] {
    const span = to - from;
    if (span <= 0) return [];
    const times: { t: number; onset: boolean }[] = [];
    for (let t = 0; t < span; t += FLASH_SAMPLE_STEP_MS) times.push({ t, onset: false });
    times.push({ t: span, onset: false });
    for (const s of scheduled) {
        for (const strike of s.pattern.strikes) {
            const onset = s.startAt + strike.atMs - from;
            if (onset >= 0 && onset <= span) times.push({ t: onset, onset: true });
            const holdEnd = onset + FLASH_STRIKE_HOLD_MS;
            if (holdEnd >= 0 && holdEnd <= span) times.push({ t: holdEnd, onset: false });
        }
    }
    // Onsets first at equal times, so a coinciding sample point is dropped.
    times.sort((a, b) => a.t - b.t || Number(b.onset) - Number(a.onset));

    const keyframes: Keyframe[] = [];
    let prevT = -Infinity;
    for (const { t, onset } of times) {
        const same = Math.abs(t - prevT) < 1e-6;
        if (same && !onset) continue;
        const offset = t / span;
        if (onset && !same) {
            keyframes.push({ offset, opacity: envelopeAt(scheduled, from + t - 1e-3) });
        }
        if (!same) keyframes.push({ offset, opacity: envelopeAt(scheduled, from + t) });
        prevT = t;
    }
    // Pin the ends exactly; float division can leave 0.9999999.
    keyframes[0].offset = 0;
    keyframes[keyframes.length - 1].offset = 1;
    return keyframes;
}

interface RunningFlash {
    scheduled: ScheduledFlash[];
    anim: Animation | null;
}
const running = new WeakMap<Element, RunningFlash>();

function now(): number {
    return typeof performance !== "undefined" && performance.now ? performance.now() : Date.now();
}

/**
 * Play `flash` on the `::before` overlay of `el`. The element's stylesheet
 * owns the overlay (position, `opacity: 0` at rest, `pointer-events: none`,
 * and the brightening of `--activity-flash-base` into its fill); this drives
 * its opacity and, when `baseColor` is given, which color it brightens.
 *
 * A sound that arrives while an earlier one is still showing is merged, not
 * restarted: both play in full in the audio, so the overlay shows the
 * brighter of the two at every instant.
 *
 * Web Animations API rather than a CSS class toggle: re-triggering a CSS
 * keyframe animation needs a forced reflow between remove and re-add, and
 * a forced layout on every tool call is exactly the cost the agent-pane
 * typing work (PR #3599) removed. The keyframes animate only `opacity`, so
 * the whole flash runs on the compositor.
 */
export function flashElement(
    el: HTMLElement,
    flash: { pattern: FlashPattern; delayMs: number },
    baseColor?: string | null
): void {
    if (flash.pattern.strikes.length === 0) return;
    const t = now();
    // A negative delay means the sound is already that far in: place the
    // pattern in the past so it resumes where the sound is. One that has
    // entirely finished has nothing left to show, and must not interrupt
    // whatever this element is still flashing.
    const incoming: ScheduledFlash = { startAt: t + flash.delayMs, pattern: flash.pattern };
    if (incoming.startAt + patternDurationMs(incoming.pattern) <= t) return;
    const prev = running.get(el);
    prev?.anim?.cancel();
    const scheduled = (prev?.scheduled ?? []).filter((s) => s.startAt + patternDurationMs(s.pattern) > t);
    scheduled.push(incoming);

    if (baseColor) el.style.setProperty(FLASH_BASE_COLOR_VAR, baseColor);
    else el.style.removeProperty(FLASH_BASE_COLOR_VAR);

    if (typeof el.animate !== "function") {
        running.set(el, { scheduled, anim: null });
        return;
    }
    const end = Math.max(...scheduled.map((s) => s.startAt + patternDurationMs(s.pattern)));
    const anim = el.animate(buildFlashKeyframes(scheduled, t, end), {
        duration: end - t,
        pseudoElement: "::before",
    });
    // Pin the animation's zero to `t`. Left alone it would start on the next
    // frame, up to ~17 ms late, and every strike would land that much after
    // its sound. The document timeline and performance.now() share a clock.
    try {
        anim.startTime = t;
    } catch {
        // No timeline (tests): the animation just starts when it starts.
    }
    running.set(el, { scheduled, anim });
}

// ── Test helpers (NEVER call from production) ────────────────────────────

export function __resetActivityFlash(): void {
    listeners.clear();
}
