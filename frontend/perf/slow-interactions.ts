// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Slow interactions in the page log.
 *
 * `[perf] long-frame` says what filled a slow frame, and nothing about what the person did to start it. The Event
 * Timing entries (`click`, `keydown`, `pointerdown`, `focusin`...) are the other half: one `[perf] slow-interaction`
 * line per interaction that took `SLOW_INTERACTION_MS` or more says which event it was, how its time divided (input
 * delay, processing, presentation) and what was acted on, so a long frame in the log can be matched to a click on a
 * pane's view, and a click that feels slow can be found without a lab run.
 *
 * The target is described without content: the pane's short block id and view type, the element's tag and role, and
 * a marker from a short list of stable attributes (`data-strip-button`, `data-role`...). Never text, a title, a
 * path or an href. A marker value that looks like an id or a name someone typed is left out.
 *
 * Lines are rate limited like the long-frame lines: one per interaction id, at most `MAX_LINES` a minute, and the
 * ones over that are counted into the next line.
 */

const PERF_LOG_PREFIX = "[perf]";
/** An interaction at or over this (Event Timing rounds to 8 ms) is logged. */
export const SLOW_INTERACTION_MS = 48;
/** At most this many interactions are described in each `WINDOW_MS`. */
const MAX_LINES = 20;
const WINDOW_MS = 60_000;
/** How many interaction ids are remembered, so the other events of one interaction are not logged again. */
const SEEN_IDS = 64;
/** Ancestors searched for a marker, from the element hit. */
const MARKER_DEPTH = 6;
/** Attributes whose value says what an element is, in the order they are looked for. */
const MARKER_ATTRS = ["data-strip-button", "data-role", "data-testid", "data-tool", "data-kind"];
const SHORT_ID = 8;

/** The parts of a `PerformanceEventTiming` entry the line uses. */
export interface SlowInteraction {
    name: string;
    startTime: number;
    duration: number;
    processingStart: number;
    processingEnd: number;
    interactionId?: number;
    target?: Node | null;
}

const ms = (n: number): string => `${Math.round(n)}ms`;

/** A marker value is kept when it is a short name; a long run of digits or hex is an id, and mixed case or spaces is text. */
export function isStableName(value: string): boolean {
    return /^[a-z][a-z0-9_-]{0,31}$/.test(value) && !/\d{4,}|[0-9a-f]{8,}/.test(value);
}

function markerOf(el: Element): string {
    for (const attr of MARKER_ATTRS) {
        if (!el.hasAttribute(attr)) continue;
        const value = el.getAttribute(attr) ?? "";
        if (value === "") return `[${attr}]`;
        if (isStableName(value)) return `[${attr}=${value}]`;
    }
    return "";
}

/** What was acted on: the element's tag, role and nearest stable marker, never its content. */
export function describeTarget(target: Node | null | undefined): string {
    const el: Element | null = target instanceof Element ? target : (target?.parentElement ?? null);
    if (!el) return "none";
    const role = el.getAttribute("role");
    let marker = "";
    for (let cur: Element | null = el, i = 0; cur && i < MARKER_DEPTH && !marker; cur = cur.parentElement, i++) {
        marker = markerOf(cur);
    }
    return `${el.tagName.toLowerCase()}${role && isStableName(role) ? `[role=${role}]` : ""}${marker ? ` ${marker}` : ""}`;
}

/** The pane around the target: its short block id and view type, or nothing when it is outside every pane. */
export function describePane(target: Node | null | undefined, viewOf: (blockId: string) => string | undefined): string {
    const el = target instanceof Element ? target : (target?.parentElement ?? null);
    const blockId = el?.closest("[data-blockid]")?.getAttribute("data-blockid");
    if (!blockId) return "block=- view=-";
    const view = viewOf(blockId);
    return `block=${blockId.slice(0, SHORT_ID)} view=${view && isStableName(view) ? view : "-"}`;
}

/** One line for an interaction: the event, how its time divides, the window, the pane and the target. */
export function describeInteraction(
    entry: SlowInteraction,
    ctx: { windowLabel: string; viewOf: (blockId: string) => string | undefined }
): string {
    const delay = Math.max(0, entry.processingStart - entry.startTime);
    const processing = Math.max(0, entry.processingEnd - entry.processingStart);
    const presentation = Math.max(0, entry.startTime + entry.duration - entry.processingEnd);
    return (
        `slow-interaction ${entry.name} ${ms(entry.duration)} ` +
        `(input delay ${ms(delay)}, processing ${ms(processing)}, presentation ${ms(presentation)}) ` +
        `window=${ctx.windowLabel} ${describePane(entry.target, ctx.viewOf)} target=${describeTarget(entry.target)}`
    );
}

/** Logs each slow interaction once, up to `MAX_LINES` a `WINDOW_MS`; the ones over that are counted into the next line. */
export function createSlowInteractionLogger(opts: {
    log: (line: string) => void;
    windowLabel: string;
    viewOf: (blockId: string) => string | undefined;
    now?: () => number;
}): (entry: SlowInteraction) => void {
    const now = opts.now ?? (() => performance.now());
    const seen = new Set<number>();
    let windowStart = -Infinity;
    let inWindow = 0;
    let skipped = 0;
    return (entry) => {
        // An entry with no interaction id is not input (scroll start, a hover repaint).
        if (!entry.interactionId || entry.duration < SLOW_INTERACTION_MS) return;
        if (seen.has(entry.interactionId)) return;
        seen.add(entry.interactionId);
        if (seen.size > SEEN_IDS) seen.delete(seen.values().next().value as number);
        const t = now();
        if (t - windowStart >= WINDOW_MS) {
            windowStart = t;
            inWindow = 0;
        }
        if (inWindow >= MAX_LINES) {
            skipped++;
            return;
        }
        inWindow++;
        const dropped = skipped ? ` (+${skipped} slow interactions not logged)` : "";
        skipped = 0;
        opts.log(`${PERF_LOG_PREFIX} ${describeInteraction(entry, opts)}${dropped}`);
    };
}
