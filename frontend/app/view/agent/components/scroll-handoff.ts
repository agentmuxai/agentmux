// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Scroll hand-off from a capped preview box to the agent pane
 * (SPEC_TOOL_PREVIEW_SCROLL_CHAINING_2026_07_03.md Phase 2; shared by every
 * capped transcript box per SPEC_AGENT_PANE_PREVIEW_CLEANUPS_2026_09_26.md §2),
 * with a short "skid" (`agent:previewskidnotches` notches, 2 by default) at the box's edge
 * (SPEC_TOOL_PREVIEW_WHEEL_EDGE_SKID_2026_09_27.md).
 *
 * Native scroll chaining latches a wheel gesture to the inner box, so reaching
 * its edge mid-gesture either dead-ends or jerks the pane (found live; Phase 1,
 * CSS only, was not enough). So the box sets `overscroll-behavior: contain` —
 * which blocks the browser's own relay unconditionally — and this listener
 * does the relay by hand. Bubbling doesn't help: `contain` isn't an
 * event-propagation setting.
 *
 * The skid: when the wheel arrives at a box's edge, the first few notches
 * (the setting) are absorbed (nothing moves, a line shows on that edge), and only
 * the next one scrolls the pane — so a preview that slides under the pointer
 * can't be scrolled past by notches meant for it. A trackpad or
 * high-resolution wheel is absorbed until its gesture pauses. This is the
 * browsers' own wheel-latching rule, done by hand because `contain` takes the
 * browser's out of the loop.
 */

import { getSettingsKeyAtom } from "@/app/store/global";
import { onCleanup } from "solid-js";
import { scrollTranscriptBy } from "../scroll/one-way-flow";

/** A continuous gesture (trackpad, high-res wheel) at the edge is absorbed
 *  until this long passes between two of its events: momentum arrives every
 *  frame, so a quarter second of silence means the gesture ended. */
export const SKID_GESTURE_IDLE_MS = 250;
/** The setting for how many notches of a notched wheel the edge absorbs before the pane moves. */
export const SKID_NOTCHES_SETTING = "agent:previewskidnotches";
/** The notches absorbed when the setting is unset or not a whole number from 0 up. */
export const SKID_NOTCHES_DEFAULT = 2;
/** The most the setting can ask for; more is this. */
export const SKID_NOTCHES_MAX = 10;

/** Setting -> notches absorbed: a whole number from 0 (no skid) up, held to the maximum; anything else is the default. */
export function resolveSkidNotches(setting: unknown): number {
    return typeof setting === "number" && Number.isInteger(setting) && setting >= 0
        ? Math.min(setting, SKID_NOTCHES_MAX)
        : SKID_NOTCHES_DEFAULT;
}
/** How long the edge line stays on after the last absorbed notch, so it reads
 *  as one indicator for the whole skid. */
export const SKID_FLASH_MS = 300;
/** Chromium's `wheelDeltaY` for one notch of a standard wheel. */
const WHEEL_NOTCH = 120;

export type SkidPhase = "armed" | "skidding" | "spent";

export interface WheelSkid {
    /** The box the last wheel event was over, or null. */
    box: object | null;
    dir: -1 | 0 | 1;
    phase: SkidPhase;
    /** Notches absorbed in this skid so far. */
    absorbed: number;
    /** `timeStamp` of the last wheel event over `box`. */
    lastAt: number;
}

export const SKID_IDLE: WheelSkid = { box: null, dir: 0, phase: "armed", absorbed: 0, lastAt: 0 };

export interface SkidInput {
    box: object;
    dir: -1 | 1;
    deltaY: number;
    /** The box takes part in the skid: it has content to scroll, or it opted in
     *  with `skidWhenFits`. Otherwise it hands every notch straight on. */
    canSkid: boolean;
    /** The box can't scroll further in `dir`. */
    atEdge: boolean;
    /** Notches of a notched wheel the edge absorbs; 0 hands every notch straight on. */
    limit: number;
    /** Whole notches this event carries, or null for continuous input. */
    notches: number | null;
    timeStamp: number;
}

/** `absorbed` on a forward: this event also used up part of the skid, so the
 *  edge still flashes. */
export type SkidAction = { kind: "native" } | { kind: "absorb" } | { kind: "forward"; deltaY: number; absorbed?: true };

/** One wheel event over a box: what happens to it, and the next state. Pure. */
export function nextSkid(prev: WheelSkid, input: SkidInput): { state: WheelSkid; action: SkidAction } {
    const arrival = prev.box !== input.box || prev.dir !== input.dir;
    const base = { box: input.box, dir: input.dir, lastAt: input.timeStamp };
    // Stay armed: a box that starts overflowing later (a preview whose output
    // grew) must still skid on the next notch in the same direction.
    if (!input.canSkid) return { state: { ...base, phase: "armed", absorbed: 0 }, action: { kind: "forward", deltaY: input.deltaY } };
    // A box with room left scrolls itself, whatever the limit: 0 only removes the absorbing at the edge.
    if (!input.atEdge) return { state: { ...base, phase: "armed", absorbed: 0 }, action: { kind: "native" } };
    if (input.limit <= 0) return { state: { ...base, phase: "armed", absorbed: 0 }, action: { kind: "forward", deltaY: input.deltaY } };

    const phase = arrival ? "armed" : prev.phase;
    const already = phase === "skidding" ? prev.absorbed : 0;
    if (phase === "spent") return { state: { ...base, phase, absorbed: already }, action: { kind: "forward", deltaY: input.deltaY } };

    if (input.notches !== null) {
        // Absorb up to `limit` notches in all, and pass on any further
        // notches this event coalesced (wheelDeltaY ±360 is three of them).
        const take = Math.max(0, Math.min(input.notches, input.limit - already));
        const absorbed = already + take;
        const rest = (input.deltaY * (input.notches - take)) / input.notches;
        const state: WheelSkid = { ...base, phase: absorbed >= input.limit ? "spent" : "skidding", absorbed };
        if (rest === 0) return { state, action: { kind: "absorb" } };
        return { state, action: take > 0 ? { kind: "forward", deltaY: rest, absorbed: true } : { kind: "forward", deltaY: rest } };
    }
    // Continuous: absorb until the gesture pauses, then the next event moves the pane.
    if (phase === "skidding" && input.timeStamp - prev.lastAt >= SKID_GESTURE_IDLE_MS) {
        return { state: { ...base, phase: "spent", absorbed: already }, action: { kind: "forward", deltaY: input.deltaY } };
    }
    return { state: { ...base, phase: "skidding", absorbed: already }, action: { kind: "absorb" } };
}

/** Whole notches in a wheel event, or null when it's continuous input. */
function wheelNotches(e: WheelEvent): number | null {
    // Non-standard, but present in Chromium (our only engine); absent → continuous.
    const raw = (e as WheelEvent & { wheelDeltaY?: number }).wheelDeltaY;
    if (typeof raw === "number" && raw !== 0 && raw % WHEEL_NOTCH === 0) return Math.abs(raw) / WHEEL_NOTCH;
    if (e.deltaMode !== WheelEvent.DOM_DELTA_PIXEL) return 1;
    return null;
}

// One skid state per pane (there's one pointer), and the boxes taking part.
const skids = new WeakMap<HTMLElement, WheelSkid>();
const boxes = new WeakSet<Element>();
const watchedPanes = new WeakSet<HTMLElement>();

/** A wheel over the pane but outside every box resets the skid, so coming
 *  back to the same box is an arrival again. Passive: it never cancels. */
function watchPane(pane: HTMLElement): void {
    if (watchedPanes.has(pane)) return;
    watchedPanes.add(pane);
    pane.addEventListener(
        "wheel",
        (e: WheelEvent) => {
            if (e.ctrlKey || e.metaKey || e.deltaY === 0) return;
            for (let n = e.target as Element | null; n && n !== pane; n = n.parentElement) {
                if (boxes.has(n)) return;
            }
            skids.set(pane, SKID_IDLE);
        },
        { capture: true, passive: true },
    );
}

export interface ScrollHandoffOptions {
    /** Skid even when the content fits, so the box has no scrollbar. A box that
     *  fits is at both edges at once, so every arrival over it skids. */
    skidWhenFits?: boolean;
}

/**
 * The ref callback for a capped transcript preview box: a tool's log, a jekt's
 * body or raw payload, a context delivery's text. It hands scroll to the pane at
 * the box's edges and skids there, also when the content fits, so every preview
 * holds the wheel the same way
 * (REPORT_JEKT_COLLAPSE_AND_PREVIEW_SKID_2026_10_07.md §2.3). The box takes the
 * `transcript-preview-box` mixin (_document-nodes.scss) for its cap and
 * `overscroll-behavior: contain`. Call it in a component, so the hand-off is
 * removed with it.
 */
export function previewBox(opts: ScrollHandoffOptions = {}): (el: HTMLElement) => void {
    return (el) => onCleanup(attachScrollHandoff(el, { skidWhenFits: true, ...opts }));
}

/** Returns the cleanup. The box must carry `overscroll-behavior: contain`. */
export function attachScrollHandoff(el: HTMLElement, opts: ScrollHandoffOptions = {}): () => void {
    boxes.add(el);
    el.classList.add("scroll-handoff-box");
    const initialPane = el.closest<HTMLElement>(".agent-document");
    if (initialPane) watchPane(initialPane);

    let flashTimer: ReturnType<typeof setTimeout> | undefined;
    const flash = (dir: -1 | 1): void => {
        el.classList.toggle("scroll-handoff-skid--top", dir < 0);
        el.classList.toggle("scroll-handoff-skid--bottom", dir > 0);
        clearTimeout(flashTimer);
        flashTimer = setTimeout(() => el.classList.remove("scroll-handoff-skid--top", "scroll-handoff-skid--bottom"), SKID_FLASH_MS);
    };

    const onWheel = (e: WheelEvent): void => {
        // Ctrl/⌘+wheel is a ZOOM gesture (the pane zoom in app.tsx, which
        // takes either), never a scroll; deltaY 0 is a horizontal scroll inside the box.
        if (e.ctrlKey || e.metaKey || e.deltaY === 0) return;
        const pane = el.closest<HTMLElement>(".agent-document");
        if (!pane) return;
        watchPane(pane);
        const dir = e.deltaY < 0 ? -1 : 1;
        // 1 px slack: fractional layout at non-100% zoom leaves a true bottom
        // a hair short of scrollHeight.
        const overflows = el.scrollHeight - el.clientHeight > 1;
        const canSkid = opts.skidWhenFits === true || overflows;
        const atEdge = dir < 0 ? el.scrollTop <= 0 : el.scrollTop + el.clientHeight >= el.scrollHeight - 1;
        const { state, action } = nextSkid(skids.get(pane) ?? SKID_IDLE, {
            box: el,
            dir,
            deltaY: e.deltaY,
            canSkid,
            atEdge,
            // Read at each wheel event, so a change applies to panes already open.
            limit: resolveSkidNotches(getSettingsKeyAtom(SKID_NOTCHES_SETTING)()),
            notches: wheelNotches(e),
            timeStamp: e.timeStamp,
        });
        skids.set(pane, state);
        if (action.kind === "absorb") {
            // `contain` keeps an overflowing box's edge from relaying, so nothing
            // moves. A box that fits has nothing for `contain` to act on, so
            // cancel the event there rather than trust the browser not to chain.
            if (!overflows) e.preventDefault();
            flash(dir);
        } else if (action.kind === "forward") {
            if (action.absorbed) flash(dir);
            e.preventDefault();
            scrollTranscriptBy(pane, action.deltaY);
        }
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => {
        el.removeEventListener("wheel", onWheel);
        clearTimeout(flashTimer);
        boxes.delete(el);
        el.classList.remove("scroll-handoff-box", "scroll-handoff-skid--top", "scroll-handoff-skid--bottom");
    };
}
