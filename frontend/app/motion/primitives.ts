// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Motion primitives for motion that code measures or starts
// (docs/specs/SPEC_MOTION_PRIMITIVES_2026_10_29.md). `play()` is the only code outside
// this folder that calls `Element.animate` (check-motion-gates.mjs keeps it so); the
// rest are a few lines over it. State transitions (hover, focus) stay CSS.

import { shouldReduceMotion, type MotionCategory } from "./motion";
import {
    DURATION_BASE_MS,
    DURATION_FAST_MS,
    DURATION_INSTANT_MS,
    DURATION_MODERATE_MS,
    DURATION_QUICK_MS,
    DURATION_SLOW_MS,
    EASE_BOUNCE,
    EASE_DECELERATE,
    EASE_EMPHASIZED,
    EASE_IN_OUT,
    EASE_MATERIAL,
    EASE_OUT,
    EASE_STANDARD,
} from "./tokens";

export const DURATION = {
    instant: DURATION_INSTANT_MS,
    fast: DURATION_FAST_MS,
    quick: DURATION_QUICK_MS,
    base: DURATION_BASE_MS,
    moderate: DURATION_MODERATE_MS,
    slow: DURATION_SLOW_MS,
} as const;
export type DurationName = keyof typeof DURATION;

export const EASE = {
    standard: EASE_STANDARD,
    out: EASE_OUT,
    inOut: EASE_IN_OUT,
    emphasized: EASE_EMPHASIZED,
    material: EASE_MATERIAL,
    decelerate: EASE_DECELERATE,
    bounce: EASE_BOUNCE,
} as const;
export type EaseName = keyof typeof EASE;

type Feel = { category: MotionCategory; duration: DurationName; easing: EaseName };

/** What each primitive feels like, in one place: its policy bucket, duration and easing, by token name. */
export const MOTION = {
    move: { category: "transition", duration: "base", easing: "out" },
    travel: { category: "transition", duration: "moderate", easing: "emphasized" },
    enter: { category: "transition", duration: "fast", easing: "out" },
    exit: { category: "transition", duration: "fast", easing: "out" },
    pulseBounce: { category: "feedback", duration: "slow", easing: "bounce" },
    pulseFlash: { category: "feedback", duration: "moderate", easing: "out" },
    pulseGlow: { category: "feedback", duration: "moderate", easing: "out" },
    glide: { category: "transition", duration: "moderate", easing: "decelerate" },
} as const satisfies Record<string, Feel>;

/** A running (or finished) motion. `finished` resolves when it ends by finishing or by `cancel()`, and never rejects. */
export interface MotionHandle {
    cancel(): void;
    readonly finished: Promise<void>;
    /** True once `cancel()` ended it before it finished; a motion that was skipped counts as finished. */
    readonly cancelled: boolean;
}

export interface PlayOptions {
    /** Required, so no animation forgets which policy bucket it is in. */
    category: MotionCategory;
    duration: DurationName;
    easing: EaseName;
    composite?: CompositeOperation;
    pseudoElement?: string;
    fill?: FillMode;
    /** ms to wait before it starts. */
    delay?: number;
    /** One running motion per (element, key): a new `play` with the same key cancels the old one. */
    key?: string;
    /** Runs exactly once, when the motion finishes, is cancelled, or is skipped. */
    cleanup?: () => void;
}

const running = new WeakMap<Element, Map<string, MotionHandle>>();

/** Cancel the motion running on `el` under `key`, if any. */
function cancelRunning(el: Element, key: string): void {
    running.get(el)?.get(key)?.cancel();
}

function warn(what: string, err: unknown): void {
    console.warn(`[motion] ${what}`, err);
}

/** Whether a motion in `category` would run here: the policy allows it, and the page is one that paints. */
export function motionAllowed(category: MotionCategory): boolean {
    if (shouldReduceMotion(category)) return false;
    // A hidden page throttles timers and would play the motion late.
    return typeof document === "undefined" || document.visibilityState !== "hidden";
}

/** A handle for a motion that did not run (or that ended at once): finished, not cancelled. */
export function finishedHandle(): MotionHandle {
    return { cancel() {}, finished: Promise.resolve(), cancelled: false };
}

/** One handle for several: `cancel()` cancels each, `finished` resolves when every one has ended. */
export function groupHandle(handles: readonly MotionHandle[]): MotionHandle {
    return {
        cancel: () => handles.forEach((h) => h.cancel()),
        finished: Promise.all(handles.map((h) => h.finished)).then(() => {}),
        get cancelled() {
            return handles.some((h) => h.cancelled);
        },
    };
}

/**
 * The one place that calls `animate`. Skips the motion (already finished, `cleanup` run) when the
 * policy asks for none in `category`, the page is hidden, or the engine cannot animate; ends it
 * with `cleanup` exactly once whether it finishes or is cancelled; and never throws into the
 * caller, since a failed animation must not break the update that triggered it.
 */
export function play(el: Element, keyframes: Keyframe[], opts: PlayOptions): MotionHandle {
    const { key } = opts;
    if (key) cancelRunning(el, key);

    let ended = false;
    let cancelled = false;
    let animation: Animation | undefined;
    let settle!: () => void;
    const finished = new Promise<void>((resolve) => (settle = resolve));

    const handle: MotionHandle = {
        cancel() {
            if (ended) return;
            cancelled = true;
            try {
                animation?.cancel();
            } catch (err) {
                warn("cancel failed", err);
            }
            end();
        },
        finished,
        get cancelled() {
            return cancelled;
        },
    };

    function end(): void {
        if (ended) return;
        ended = true;
        if (key) {
            const keyed = running.get(el);
            if (keyed?.get(key) === handle) keyed.delete(key);
        }
        try {
            opts.cleanup?.();
        } catch (err) {
            warn("cleanup failed", err);
        }
        settle();
    }

    if (!motionAllowed(opts.category) || typeof el.animate !== "function") {
        end();
        return handle;
    }
    try {
        animation = el.animate(keyframes, {
            duration: DURATION[opts.duration],
            easing: EASE[opts.easing],
            composite: opts.composite,
            pseudoElement: opts.pseudoElement,
            fill: opts.fill,
            delay: opts.delay,
        });
        animation.onfinish = end;
        animation.oncancel = end;
    } catch (err) {
        warn("animate failed", err);
        end();
        return handle;
    }
    if (key) {
        let keyed = running.get(el);
        if (!keyed) running.set(el, (keyed = new Map()));
        keyed.set(key, handle);
    }
    return handle;
}

// ── move (FLIP) ─────────────────────────────────────────────────────────────

export type Snapshot = ReadonlyMap<string, { left: number; top: number }>;

/** Where each element is now, by key, to hand to `move` after the DOM changes. Elements without a key are left out. */
export function capture(els: Iterable<Element>, keyOf: (el: Element) => string | null | undefined): Snapshot {
    const out = new Map<string, { left: number; top: number }>();
    for (const el of els) {
        const key = keyOf(el);
        if (!key) continue;
        const { left, top } = el.getBoundingClientRect();
        out.set(key, { left, top });
    }
    return out;
}

export type EnterKind = "fade" | "fadeScale" | "slideY" | "slideX";

export interface MoveOptions {
    axis?: "x" | "y" | "both";
    /** The zoom of the box the elements sit in: a transform is in that box's units. */
    zoom?: number;
    /** Offsets under this many px are not animated. */
    threshold?: number;
    category?: MotionCategory;
    /** What to play for an element that has no entry in `before` (it is new). Default: nothing. */
    onNew?: EnterKind;
}

/**
 * Slide every element that is now somewhere else from where `before` had it. An element whose
 * key is missing from `before` is new (`onNew` says what to do with it); a key that vanished
 * is skipped. Nothing moves, and nothing is measured, when the policy asks for no motion.
 *
 * All the writes that could change layout come first (cancelling a slide still running, whose
 * transform would be in the measurement), then every measurement, then the animations: reading
 * a rectangle after an `animate()` forces a layout per element, which on a long list is a long task.
 */
export function move(
    before: Snapshot,
    els: Iterable<Element>,
    keyOf: (el: Element) => string | null | undefined,
    opts: MoveOptions = {}
): MotionHandle {
    const { axis = "both", threshold = 1, zoom = 1, onNew } = opts;
    const category = opts.category ?? MOTION.move.category;
    if (!motionAllowed(category)) return finishedHandle();
    const moving: Array<{ el: Element; was: { left: number; top: number } }> = [];
    const fresh: Element[] = [];
    for (const el of els) {
        const key = keyOf(el);
        if (!key) continue;
        const was = before.get(key);
        if (was) moving.push({ el, was });
        else fresh.push(el);
    }
    for (const { el } of moving) cancelRunning(el, "move");
    const now = moving.map(({ el }) => el.getBoundingClientRect());
    const handles: MotionHandle[] = [];
    moving.forEach(({ el, was }, i) => {
        const dx = axis === "y" ? 0 : (was.left - now[i].left) / (zoom || 1);
        const dy = axis === "x" ? 0 : (was.top - now[i].top) / (zoom || 1);
        if (Math.abs(dx) < threshold && Math.abs(dy) < threshold) return;
        const from =
            axis === "x" ? `translateX(${dx}px)` : axis === "y" ? `translateY(${dy}px)` : `translate(${dx}px, ${dy}px)`;
        handles.push(play(el, [{ transform: from }, { transform: "none" }], { ...MOTION.move, category, key: "move" }));
    });
    if (onNew) for (const el of fresh) handles.push(enter(el, onNew));
    return groupHandle(handles);
}

// ── travel and mergeInto ────────────────────────────────────────────────────

export interface TravelOptions {
    category?: MotionCategory;
    /** ms before the copy starts to move. */
    delay?: number;
    /** Adjust the copy before it is shown, e.g. drop a class that only makes sense on the real element. */
    onCopy?: (copy: HTMLElement) => void;
    /** When the element is shown again at its place, also when motion is off; not when the travel is cancelled. */
    onArrive?: () => void;
}

const flying = new WeakMap<Element, MotionHandle>();

/**
 * `el` is already at its new place; a copy of it flies there from `from` (a rectangle read before
 * the change), in a fixed layer on <body> so no clipping ancestor hides it. The real element is
 * hidden until the copy lands, and shown again, with the copy removed, however the motion ends.
 */
export function travel(el: HTMLElement, from: DOMRect, opts: TravelOptions = {}): MotionHandle {
    const category = opts.category ?? MOTION.travel.category;
    if (!motionAllowed(category) || typeof document === "undefined") {
        opts.onArrive?.();
        return finishedHandle();
    }
    // One flight per element: a second would clone the hidden element and restore it hidden.
    flying.get(el)?.cancel();
    const to = el.getBoundingClientRect();
    const copy = el.cloneNode(true) as HTMLElement;
    opts.onCopy?.(copy);
    Object.assign(copy.style, {
        position: "fixed",
        left: `${from.left}px`,
        top: `${from.top}px`,
        width: `${to.width}px`,
        height: `${to.height}px`,
        margin: "0",
        pointerEvents: "none",
        zIndex: "10000",
    });
    const hiddenBefore = el.style.visibility;
    document.body.append(copy);
    el.style.visibility = "hidden";
    let handle: MotionHandle | undefined;
    handle = play(
        copy,
        [
            { transform: "translate(0, 0)" },
            { transform: `translate(${to.left - from.left}px, ${to.top - from.top}px)` },
        ],
        {
            ...MOTION.travel,
            category,
            delay: opts.delay,
            cleanup: () => {
                copy.remove();
                el.style.visibility = hiddenBefore;
                if (!handle?.cancelled) opts.onArrive?.();
            },
        }
    );
    flying.set(el, handle);
    void handle.finished.then(() => {
        if (flying.get(el) === handle) flying.delete(el);
    });
    return handle;
}

export interface MergeOptions {
    category?: MotionCategory;
    /** ms between pieces; 0 flies them together. */
    stagger?: number;
    /** Per piece, when it arrives (also when motion is off, so the end state is the same): the caller marks it landed. */
    land?: (el: Element, index: number) => void;
    /** A pulse of each arrived piece. Default: false when `land` is given, else "bounce". */
    pulse?: "bounce" | "flash" | false;
    /** Adjust each flying copy before it is shown (see `travel`). */
    onCopy?: (copy: HTMLElement) => void;
}

/**
 * Several pieces fly into a place that does not exist yet: `sources` are where each was, read
 * before the change; `dest` is where each is now, paired by index (a function is called once the
 * layout has rendered it, one animation frame later). A destination missing at an index is
 * skipped. `cancel()` removes every copy and shows every real element.
 */
export function mergeInto(
    sources: ReadonlyArray<DOMRect>,
    dest: ReadonlyArray<Element | undefined> | (() => Array<Element | undefined>),
    opts: MergeOptions = {}
): MotionHandle {
    const category = opts.category ?? MOTION.travel.category;
    const stagger = opts.stagger ?? 0;
    const pulseKind = opts.pulse ?? (opts.land ? false : "bounce");
    const flights: MotionHandle[] = [];
    const pulses: MotionHandle[] = [];
    let cancelled = false;
    let frame: number | undefined;
    let settle!: () => void;
    const finished = new Promise<void>((resolve) => (settle = resolve));

    const arrive = (el: Element, i: number) => {
        try {
            opts.land?.(el, i);
        } catch (err) {
            warn("land failed", err);
        }
        if (pulseKind) pulses.push(pulse(el, pulseKind));
    };

    const run = () => {
        frame = undefined;
        const els = typeof dest === "function" ? dest() : dest;
        sources.forEach((from, i) => {
            const el = els[i];
            if (!el) return;
            const onArrive = () => arrive(el, i);
            if (el instanceof HTMLElement)
                flights.push(travel(el, from, { category, delay: i * stagger, onArrive, onCopy: opts.onCopy }));
            else onArrive();
        });
        // Done when every piece has landed and its pulse has ended.
        void Promise.all(flights.map((f) => f.finished))
            .then(() => Promise.all(pulses.map((p) => p.finished)))
            .then(settle);
    };

    if (typeof dest === "function") {
        frame =
            typeof requestAnimationFrame === "function"
                ? requestAnimationFrame(run)
                : (setTimeout(run, 0) as unknown as number);
    } else {
        run();
    }

    return {
        cancel() {
            if (cancelled) return;
            cancelled = true;
            if (frame !== undefined) {
                if (typeof cancelAnimationFrame === "function") cancelAnimationFrame(frame);
                else clearTimeout(frame);
                frame = undefined;
            }
            flights.forEach((f) => f.cancel());
            pulses.forEach((p) => p.cancel());
            settle();
        },
        finished,
        get cancelled() {
            return cancelled;
        },
    };
}

// ── enter, exit, pulse, glide ───────────────────────────────────────────────

const SLIDE_PX = 8;

const ENTER_FROM: Record<EnterKind, Keyframe> = {
    fade: { opacity: 0 },
    fadeScale: { opacity: 0, transform: "scale(0.96)" },
    slideY: { opacity: 0, transform: `translateY(${SLIDE_PX}px)` },
    slideX: { opacity: 0, transform: `translateX(${SLIDE_PX}px)` },
};

const ENTER_TO: Keyframe = { opacity: 1, transform: "none" };

/** Show an element the code just mounted. */
export function enter(el: Element, kind: EnterKind = "fade", opts: { category?: MotionCategory } = {}): MotionHandle {
    return play(el, [ENTER_FROM[kind], ENTER_TO], {
        ...MOTION.enter,
        ...opts,
        category: opts.category ?? MOTION.enter.category,
    });
}

/**
 * Take an element out. It stays at the end of the motion until the caller removes it when
 * `finished` resolves; with motion off that is at once, so the node goes at once.
 */
export function exit(el: Element, kind: EnterKind = "fade", opts: { category?: MotionCategory } = {}): MotionHandle {
    return play(el, [ENTER_TO, ENTER_FROM[kind]], {
        ...MOTION.exit,
        category: opts.category ?? MOTION.exit.category,
        fill: "forwards",
    });
}

export type PulseKind = "bounce" | "flash" | "glow";

const PULSE: Record<PulseKind, { keyframes: Keyframe[]; feel: Feel }> = {
    // The landing bounce of a tab, a row or a pill: a spring that overshoots and settles.
    bounce: {
        keyframes: [
            { transform: "scaleX(0.88)", transformOrigin: "center bottom", offset: 0 },
            { transform: "scaleX(1.04)", transformOrigin: "center bottom", offset: 0.55 },
            { transform: "scaleX(0.98)", transformOrigin: "center bottom", offset: 0.8 },
            { transform: "scaleX(1)", transformOrigin: "center bottom", offset: 1 },
        ],
        feel: MOTION.pulseBounce,
    },
    // The accent colour fades out to the element's own (am-accent-flash).
    flash: { keyframes: [{ color: "var(--accent-color)" }, {}], feel: MOTION.pulseFlash },
    // An outline glow that swells and goes (ui-attention-glow).
    glow: {
        keyframes: [
            { boxShadow: "0 0 0 transparent", offset: 0 },
            { boxShadow: "0 0 18px color-mix(in srgb, var(--accent-color) 75%, transparent)", offset: 0.5 },
            { boxShadow: "0 0 0 transparent", offset: 1 },
        ],
        feel: MOTION.pulseGlow,
    },
};

/** One attention beat on an element. */
export function pulse(el: Element, kind: PulseKind = "bounce", opts: { category?: MotionCategory } = {}): MotionHandle {
    const { keyframes, feel } = PULSE[kind];
    return play(el, keyframes, { ...feel, category: opts.category ?? feel.category, key: "pulse" });
}

export interface GlideOptions {
    category?: MotionCategory;
    /** The token to run for; the preset's `moderate` when absent. */
    duration?: DurationName;
    axis?: "x" | "y";
    cleanup?: () => void;
}

/**
 * An offset (px) that decays to nothing, added to whatever else is animating on the element
 * (`composite: "add"`), so overlapping glides sum; the caller keeps the handles (`groupHandle`) to cancel them together.
 */
export function glide(el: Element, offset: number, opts: GlideOptions = {}): MotionHandle {
    const fn = opts.axis === "x" ? "translateX" : "translateY";
    return play(el, [{ transform: `${fn}(${offset}px)` }, { transform: `${fn}(0)` }], {
        ...MOTION.glide,
        category: opts.category ?? MOTION.glide.category,
        duration: opts.duration ?? MOTION.glide.duration,
        composite: "add",
        cleanup: opts.cleanup,
    });
}
