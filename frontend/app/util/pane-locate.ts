// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The locate flash: when a block is revealed (a Swarm row, Tower, the token
 * popover, the agent picker, or another window asking), its pane flashes once
 * in its own colour so the user can see which pane was just selected.
 *
 * `revealBlockLocally` asks (`emitPaneLocate`). The pane that owns the block
 * waits until it is actually on screen (`whenPaneReady`), then shows the
 * overlay (`showLocateFlash`) and tells its tab pills to light up
 * (`emitPaneLocatePlay`). A reveal can switch the window tab or raise another
 * window first, so the flash starts when the pane can be seen, not when the
 * reveal was requested.
 *
 * Store-free, like activity-flash.ts: a subscriber set plus the animation.
 *
 * One flash, never a repeat: the overlay covers a whole pane, which is larger
 * than the small-area exemption of WCAG 2.3.1, so it is a single slow fade and
 * starts at most three times a second (`locateSlotWaitMs`). The animation moves
 * nothing (opacity only), so it has no reduced-motion variant beyond dropping
 * the ring when the motion policy asks for it.
 */

import { EASE_OUT } from "@/app/motion/tokens";
import { userInThisWindow } from "@/app/store/input-owner";
import type { FlashPattern } from "@/app/notification/activity-flash";

/** The flash rises over this long... */
export const LOCATE_RISE_MS = 60;
/** ...holds at full strength this long... */
export const LOCATE_HOLD_MS = 80;
/** ...and has faded out completely by this time after it started. */
export const LOCATE_TOTAL_MS = 900;
/** Flashes start at least this far apart: three a second at most. */
export const LOCATE_MIN_GAP_MS = 340;
/** How long a pane gets to become visible before the flash is dropped. */
export const LOCATE_READY_TIMEOUT_MS = 1500;
/** The same, when the window is hidden (another window is still being raised). */
export const LOCATE_READY_TIMEOUT_HIDDEN_MS = 3000;
/** Frames the pane's box must be unchanged before it counts as settled. */
export const LOCATE_STABLE_FRAMES = 2;
/** Overlay fill, as a share of the pane colour. */
export const LOCATE_WASH_DARK = "16%";
export const LOCATE_WASH_LIGHT = "12%";

/** The pane tab pill: one strike with a tail as long as the pane's fade. */
export const LOCATE_PILL_FLASH: { pattern: FlashPattern; delayMs: number } = {
    pattern: { strikes: [{ atMs: 0, intensity: 0.85 }], tailMs: LOCATE_TOTAL_MS - 20 },
    delayMs: 0,
};

// ── Bus ──────────────────────────────────────────────────────────────────

type BlockListener = (blockId: string) => void;
/** Told which block, and which pane's flash it is: a block can have pills in more than one pane. */
type PlayListener = (blockId: string, paneKey: string) => void;
const requests = new Set<BlockListener>();
const plays = new Set<PlayListener>();

function notify<A extends unknown[]>(listeners: Set<(...args: A) => void>, ...args: A): void {
    for (const l of listeners) {
        try {
            l(...args);
        } catch (e) {
            console.warn("[pane-locate] listener threw", e);
        }
    }
}

/** `blockId` was just revealed: its pane should flash once it is on screen. */
export function emitPaneLocate(blockId: string): void {
    notify(requests, blockId);
}

/** Every pane hears every request: the pane that owns the block starts, the others stop. */
export function onPaneLocate(listener: BlockListener): () => void {
    requests.add(listener);
    return () => requests.delete(listener);
}

/** The pane `paneKey` is flashing for `blockId` now: that pane's pills light up too. */
export function emitPaneLocatePlay(blockId: string, paneKey: string): void {
    notify(plays, blockId, paneKey);
}

export function onPaneLocatePlay(listener: PlayListener): () => void {
    plays.add(listener);
    return () => plays.delete(listener);
}

// ── Rate ─────────────────────────────────────────────────────────────────

let lastStartAt = -Infinity;

/** How long until a flash may start: 0 when it may start at `nowMs`. */
export function locateSlotWaitMs(nowMs: number): number {
    return Math.max(0, lastStartAt + LOCATE_MIN_GAP_MS - nowMs);
}

/** A flash started at `nowMs`. */
export function recordLocateStart(nowMs: number): void {
    lastStartAt = nowMs;
}

// ── Readiness ────────────────────────────────────────────────────────────

/** What one frame saw of a pane. */
export interface PaneSample {
    /** Everything but size holds: shown, focused, and showing the block. */
    ok: boolean;
    /** The pane's box, or "" when it has none. Equal strings mean it has not moved. */
    box: string;
}

export interface ReadyOptions {
    timeoutMs: number;
    requestFrame?: (cb: () => void) => number;
    cancelFrame?: (handle: number) => void;
    setTimer?: (cb: () => void, ms: number) => number;
    clearTimer?: (handle: number) => void;
}

export interface ReadyGate {
    /** True once the pane has been on screen and still for `LOCATE_STABLE_FRAMES`; false on timeout or cancel. */
    ready: Promise<boolean>;
    cancel: () => void;
}

/**
 * Watch a pane frame by frame until it can be seen and has stopped moving.
 * A hidden window produces no frames, so the loop simply resumes when the
 * window is shown; the timer is what ends the wait.
 */
export function whenPaneReady(sample: () => PaneSample, opts: ReadyOptions): ReadyGate {
    const requestFrame = opts.requestFrame ?? ((cb) => requestAnimationFrame(cb));
    const cancelFrame = opts.cancelFrame ?? ((h) => cancelAnimationFrame(h));
    const setTimer = opts.setTimer ?? ((cb, ms) => window.setTimeout(cb, ms));
    const clearTimer = opts.clearTimer ?? ((h) => window.clearTimeout(h));

    let settle: (ok: boolean) => void = () => {};
    const ready = new Promise<boolean>((resolve) => {
        settle = resolve;
    });
    let frame = 0;
    let done = false;
    const finish = (ok: boolean) => {
        if (done) return;
        done = true;
        cancelFrame(frame);
        clearTimer(timer);
        settle(ok);
    };
    const timer = setTimer(() => finish(false), opts.timeoutMs);

    let lastBox = "";
    let stable = 0;
    const step = () => {
        if (done) return;
        const s = sample();
        if (s.ok && s.box !== "") {
            stable = s.box === lastBox ? stable + 1 : 0;
            lastBox = s.box;
            if (stable >= LOCATE_STABLE_FRAMES) return finish(true);
        } else {
            stable = 0;
            lastBox = "";
        }
        frame = requestFrame(step);
    };
    frame = requestFrame(step);
    return { ready, cancel: () => finish(false) };
}

/**
 * What a frame sees of the pane at `el`: its box, and whether it is painted
 * (not behind a hidden window tab, not `visibility: hidden` while a tab
 * settles, not in a hidden window) in the window the user is in. A window can
 * be visible yet still behind another while it is being raised.
 */
export function samplePane(el: HTMLElement | undefined, holds: boolean): PaneSample {
    if (!el || !holds || document.visibilityState !== "visible" || !userInThisWindow()) return { ok: false, box: "" };
    if (typeof el.checkVisibility === "function" && !el.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true })) {
        return { ok: false, box: "" };
    }
    const r = el.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) return { ok: false, box: "" };
    return { ok: true, box: `${Math.round(r.left)},${Math.round(r.top)},${Math.round(r.width)},${Math.round(r.height)}` };
}

// ── Overlay ──────────────────────────────────────────────────────────────

export interface LocateFlashOptions {
    /** Light themes get a lighter wash. */
    light: boolean;
    /** The motion policy asks for less: wash only, no ring. */
    reduced: boolean;
}

/** The opacity envelope: rise, hold, then an ease-out fade. */
export function locateKeyframes(): Keyframe[] {
    const riseEnd = LOCATE_RISE_MS / LOCATE_TOTAL_MS;
    const holdEnd = (LOCATE_RISE_MS + LOCATE_HOLD_MS) / LOCATE_TOTAL_MS;
    return [
        { offset: 0, opacity: 0, easing: "linear" },
        { offset: riseEnd, opacity: 1 },
        { offset: holdEnd, opacity: 1, easing: EASE_OUT },
        { offset: 1, opacity: 0 },
    ];
}

/**
 * Add the flash overlay to `host` (the pane's root) and play it once. The
 * overlay is removed when it finishes. Returns a function that removes it
 * at once; calling it again, or after the end, does nothing.
 */
export function showLocateFlash(host: HTMLElement, opts: LocateFlashOptions): () => void {
    const el = host.ownerDocument.createElement("div");
    el.className = opts.reduced ? "pane-locate-flash pane-locate-flash--reduced" : "pane-locate-flash";
    el.style.setProperty("--pane-locate-wash", opts.light ? LOCATE_WASH_LIGHT : LOCATE_WASH_DARK);
    el.setAttribute("aria-hidden", "true");
    host.appendChild(el);
    let removed = false;
    const remove = () => {
        if (removed) return;
        removed = true;
        el.remove();
    };
    if (typeof el.animate !== "function") {
        // No animation support (tests): leave nothing behind.
        remove();
        return remove;
    }
    const anim = el.animate(locateKeyframes(), { duration: LOCATE_TOTAL_MS });
    anim.onfinish = remove;
    anim.oncancel = remove;
    return () => {
        if (removed) return;
        anim.cancel();
        remove();
    };
}
