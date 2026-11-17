// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Wheel diagnostic for the agent transcript: when dev mode is on, one `[wheel-diag]` line for each wheel
 * event over the transcript that moved nothing although something under the pointer could have moved.
 * Off by default (`devModeOn()`); it adds one passive listener and does no work while dev mode is off.
 *
 * Why it exists (task #47925): a person sees the wheel go dead over parts of a real conversation that no
 * scripted wheel could reproduce. The line says what the pointer was over, which scrollers sat under it and
 * where each was, whether the box's hand-off absorbed or passed the notch, whether the event was cancelled,
 * and the follow state. The renderer's console lines land in the host's `cef-debug.log`.
 *
 * A run of dead events logs its first, third and tenth event, then every fiftieth, so one stuck gesture is a
 * few lines. The first of a run also carries the 12 events before it.
 */

import { lastHandoff } from "../components/scroll-handoff";

export const WHEEL_DIAG_TAG = "[wheel-diag]";
/** Events kept to show what led up to a dead one. */
export const WHEEL_DIAG_RING = 12;
/** How long after the event the scrollers are read again: past the frame that applies a native scroll. */
export const WHEEL_DIAG_SETTLE_MS = 80;
/** Dead lines for the Nth consecutive dead event of a run. */
export function logsRunEvent(n: number): boolean {
    return n === 1 || n === 3 || n === 10 || (n > 10 && n % 50 === 0);
}

export interface ScrollerState {
    label: string;
    top: number;
    max: number;
}

export interface WheelRecord {
    t: number;
    dy: number;
    mode: number;
    wdy: number | null;
    x: number;
    y: number;
    under: string;
    scrollers: ScrollerState[];
    follow: string | null;
    hand: string | null;
    panel: string | null;
}

const labelOf = (el: Element): string => {
    const c: unknown = el.className; // an SVG element's is an object
    const name = typeof c === "string" ? c : ((c as { baseVal?: string } | null)?.baseVal ?? "");
    return `${el.tagName.toLowerCase()}.${name.split(/\s+/)[0]}`;
};

/** The element under the pointer and up to `max` ancestors, innermost first. */
export function describeChain(el: Element | null, stop: Element | null, max = 5): string {
    const out: string[] = [];
    for (let e = el; e && e !== stop && out.length < max; e = e.parentElement) out.push(labelOf(e));
    return out.join(" < ");
}

/** Every ancestor of `el` that has content to scroll, innermost first, with where it is. */
export function scrollersAt(el: Element | null): ScrollerState[] {
    const out: ScrollerState[] = [];
    for (let e = el; e && e !== document.body; e = e.parentElement) {
        const h = e as HTMLElement;
        if (h.scrollHeight <= h.clientHeight + 1) continue;
        const oy = getComputedStyle(h).overflowY;
        if (oy !== "auto" && oy !== "scroll") continue;
        out.push({ label: labelOf(h), top: Math.round(h.scrollTop), max: Math.round(h.scrollHeight - h.clientHeight) });
    }
    return out;
}

const signature = (s: ScrollerState[]): string => s.map((x) => x.top).join(",");

/** Some scroller under the pointer has room to move in the wheel's direction. */
export function canMove(s: ScrollerState[], dir: -1 | 1): boolean {
    return s.some((x) => (dir < 0 ? x.top > 0 : x.top < x.max));
}

export interface WheelDiagDeps {
    enabled: () => boolean;
    log: (...args: unknown[]) => void;
    later: (fn: () => void, ms: number) => void;
}

/** The listener. Pure of globals except the DOM it is handed, so a test drives it with fake events. */
export function createWheelDiag(deps: WheelDiagDeps): (e: WheelEvent) => void {
    const ring: WheelRecord[] = [];
    let run = 0;
    return (e) => {
        if (!deps.enabled() || !e.isTrusted || e.ctrlKey || e.metaKey || e.deltaY === 0) return;
        const target = e.target instanceof Element ? e.target : null;
        const doc = target?.closest<HTMLElement>(".agent-document[data-follow-state]") ?? null;
        const inPanel = !!target?.closest(".agent-node-peek-overlay");
        if (!doc && !inPanel) return;
        const under = document.elementFromPoint(e.clientX, e.clientY);
        const before = scrollersAt(under);
        const dir: -1 | 1 = e.deltaY < 0 ? -1 : 1;
        const rec: WheelRecord = {
            t: Math.round(e.timeStamp),
            dy: Math.round(e.deltaY * 10) / 10,
            mode: e.deltaMode,
            wdy: (e as WheelEvent & { wheelDeltaY?: number }).wheelDeltaY ?? null,
            x: e.clientX,
            y: e.clientY,
            under: describeChain(under, doc, 5),
            scrollers: before,
            follow: doc?.dataset.followState ?? null,
            hand: null,
            panel: inPanel ? "panel" : null,
        };
        deps.later(() => {
            rec.hand = lastHandoff.at === e.timeStamp ? lastHandoff.action : null;
            const after = scrollersAt(document.elementFromPoint(e.clientX, e.clientY));
            const moved = signature(after) !== signature(before) || e.defaultPrevented;
            ring.push(rec);
            if (ring.length > WHEEL_DIAG_RING + 1) ring.shift();
            if (moved || !canMove(before, dir)) {
                run = 0;
                return;
            }
            run++;
            if (!logsRunEvent(run)) return;
            deps.log(
                WHEEL_DIAG_TAG,
                JSON.stringify({
                    dead: run,
                    ...rec,
                    after: after.map((x) => x.top),
                    prevented: e.defaultPrevented,
                    dpr: window.devicePixelRatio,
                    ...(run === 1 ? { before: ring.slice(0, -1) } : {}),
                }),
            );
        }, WHEEL_DIAG_SETTLE_MS);
    };
}

let installed = false;

/** Once per window; idempotent. The listener checks dev mode on every event, so a toggle applies at once. */
export function installWheelDiag(deps: Pick<WheelDiagDeps, "enabled">): void {
    if (installed || typeof window === "undefined") return;
    installed = true;
    const handler = createWheelDiag({ enabled: deps.enabled, log: (...a) => console.info(...a), later: (fn, ms) => void setTimeout(fn, ms) });
    window.addEventListener("wheel", handler, { capture: true, passive: true });
}
