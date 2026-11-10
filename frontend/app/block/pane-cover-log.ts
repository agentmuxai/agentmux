// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The `[pane-cover]` log: one line per change of a block's cover phase, so a pane that stayed under the logo
 * cover (or came back under it) says when and from which source in `muxlog fe grep pane-cover`. The cover has two
 * sources, the readiness controller (assembly, once) and the block's own re-cover (every subagent "started" event);
 * before this log nothing recorded the second, and a forced reveal was the only line the first one wrote.
 *
 * Every line names the agent (a block id alone can't be matched to the agent someone saw go black), and three more
 * lines say what the phase changes can't: `still-covered` once a second while a cover is up, with what the
 * controller waits on; `unmount` when the pane's view goes away, with the block's state at that moment; and, after a
 * cover that lasted `SLOW_COVER_MS` or more, one `settled` line with what the pane looks like two seconds after it
 * lifted (size, element count, the tallest scroller's position), for a pane that is live and still looks black.
 *
 * Two more things put the logo, or nothing, in front of a live agent pane, and neither is a phase of the cover:
 * the block's Suspense fallback (its view suspended after the cover lifted) and the transcript's own show-cover
 * (hidden for up to `SHOW_SETTLE_MAX_MS` after the pane is shown again). Each writes `shown`, `still-shown` once a
 * second and `cleared` lines under the same tag, so a pane that went blank says which of the three it was.
 */
import { createEffect, createMemo, on, onCleanup, untrack } from "solid-js";
import { isParkedMeta } from "@/app/store/agent-parked";
import type { PaneReadiness, PaneReadinessPhase } from "@/app/store/pane-readiness";

export type CoverSource = "controller" | "re-cover";

/** A cover up this long gets a `settled` line when it lifts. */
export const SLOW_COVER_MS = 3000;
/** How long after the cover lifts the `settled` line looks at the pane. */
export const SETTLED_AFTER_MS = 2000;
/** The most `still-covered` lines one cover writes. */
const STILL_COVERED_MAX = 60;

/** What the log reads from the block, each when the line is written (the agent's name arrives with the block's meta). */
export interface CoverLogContext {
    agent?: () => string | undefined;
    /** The gates the readiness controller is still waiting on. */
    pending?: () => string[];
    /** The block's state as its view goes away: whether the block still exists, is parked, is in a shown window tab. */
    why?: () => string;
    /** The block's root element, for the `settled` line. */
    root?: () => Element | null;
}

/** The agent's name from a block's meta, when it has one. */
export function agentNameOf(block: { meta?: Record<string, unknown> } | undefined): string | undefined {
    const name = block?.meta?.agentName;
    return typeof name === "string" ? name : undefined;
}

/** What the log reads from a block: its agent's name, why its view went away, and where to look at it, each when a line is written. */
export function blockCoverContext(
    blockId: string,
    block: () => { meta?: Record<string, unknown> } | undefined,
    tabDisplayed: () => boolean,
    pending: () => string[]
): CoverLogContext {
    return {
        agent: () => agentNameOf(block()),
        pending,
        why: () =>
            `block=${block() ? "present" : "gone"} parked=${isParkedMeta(block()?.meta)} ` +
            `tab=${tabDisplayed() ? "shown" : "hidden"} view=${block()?.meta?.view ?? "?"}`,
        root: () => document.querySelector(`[data-blockid="${blockId}"]`),
    };
}

const head = (blockId: string, agent?: string): string =>
    `block=${blockId.slice(0, 7)}${agent ? ` agent=${JSON.stringify(agent)}` : ""}`;

/** `[pane-cover] block=abc1234 agent="Lazo" assembling→revealing src=controller t=1103ms` (without the tag). */
export function coverLogLine(
    blockId: string,
    prev: PaneReadinessPhase | undefined,
    next: PaneReadinessPhase,
    source: CoverSource,
    sinceMountMs: number,
    agent?: string
): string {
    return `${head(blockId, agent)} ${prev ?? "mount"}→${next} src=${source} t=${Math.round(sinceMountMs)}ms`;
}

/** The line written once a second while a cover is up: which source holds it, for how long, what it waits on. */
export function stillCoveredLine(
    blockId: string,
    source: CoverSource,
    phase: PaneReadinessPhase,
    forMs: number,
    pending: string[],
    agent?: string
): string {
    return `${head(blockId, agent)} still-covered src=${source} phase=${phase} for=${Math.round(forMs)}ms pending=${pending.join(",") || "none"}`;
}

/** The line written as the pane's view goes away: both sources' phases, how long it was mounted, why. */
export function unmountLine(
    blockId: string,
    phases: { controller: PaneReadinessPhase; reCover: PaneReadinessPhase },
    mountedMs: number,
    why: string,
    agent?: string
): string {
    return `${head(blockId, agent)} unmount controller=${phases.controller} re-cover=${phases.reCover} after=${Math.round(mountedMs)}ms ${why}`;
}

/** What a pane looks like: its size, how many elements it holds, and the tallest scroller's position in it. */
export function paneShapeOf(root: Element | null): string {
    if (!root) return "root=missing";
    const rect = root.getBoundingClientRect();
    let nodes = 0;
    let tallest: Element | null = null;
    for (const el of Array.from(root.querySelectorAll("*"))) {
        nodes++;
        if (el.clientHeight > 0 && el.scrollHeight > el.clientHeight + 1 && (!tallest || el.scrollHeight > tallest.scrollHeight)) tallest = el;
    }
    const scroller = tallest ? `${Math.round(tallest.scrollTop)}/${tallest.scrollHeight}/${tallest.clientHeight}` : "none";
    return `size=${Math.round(rect.width)}x${Math.round(rect.height)} nodes=${nodes} scroller(top/height/view)=${scroller}`;
}

/** The line written two seconds after a slow cover lifted. */
export function settledLine(blockId: string, coveredMs: number, root: Element | null, agent?: string): string {
    return `${head(blockId, agent)} settled after-cover=${Math.round(coveredMs)}ms ${paneShapeOf(root)}`;
}

/**
 * Logs every change of either source's phase, each under its own source: the controller's from mount, the
 * re-cover's from its first change (it starts out `live`, which is not a change). Reading the source from
 * "which one is live now" would call the controller's own last step (revealing to live) a re-cover.
 * Call during component setup (it creates effects).
 */
export function logCoverPhases(
    blockId: string,
    phases: { controller: () => PaneReadinessPhase; reCover: () => PaneReadinessPhase },
    ctx: CoverLogContext = {}
): void {
    const mountedAt = performance.now();
    const agent = () => ctx.agent?.() || undefined;
    const log = (line: string) => console.info("[pane-cover]", line);
    const logChange = (source: CoverSource, prev: PaneReadinessPhase | undefined, next: PaneReadinessPhase) =>
        log(coverLogLine(blockId, prev, next, source, performance.now() - mountedAt, agent()));
    createEffect(on(phases.controller, (next, prev) => logChange("controller", prev, next)));
    // `on` hands a deferred effect no previous value on its first run, so the re-cover keeps its own.
    let reCoverPrev = untrack(phases.reCover);
    createEffect(
        on(
            phases.reCover,
            (next) => {
                logChange("re-cover", reCoverPrev, next);
                reCoverPrev = next;
            },
            { defer: true }
        )
    );

    const sourceNow = (): CoverSource => (phases.controller() !== "live" ? "controller" : "re-cover");
    // A memo, so the effect below runs when the cover goes up or comes down, not on each phase change in between.
    const covered = createMemo(() => phases.controller() !== "live" || phases.reCover() !== "live");
    let coveredSince = -1;
    createEffect(() => {
        if (covered()) {
            coveredSince = performance.now();
            let lines = 0;
            const timer = setInterval(() => {
                if (++lines > STILL_COVERED_MAX) return clearInterval(timer);
                const source = sourceNow();
                const phase = source === "controller" ? phases.controller() : phases.reCover();
                log(stillCoveredLine(blockId, source, phase, performance.now() - coveredSince, ctx.pending?.() ?? [], agent()));
            }, 1000);
            onCleanup(() => clearInterval(timer));
            return;
        }
        const coveredMs = coveredSince >= 0 ? performance.now() - coveredSince : 0;
        if (coveredMs < SLOW_COVER_MS) return;
        const timer = setTimeout(() => log(settledLine(blockId, coveredMs, ctx.root?.() ?? null, agent())), SETTLED_AFTER_MS);
        onCleanup(() => clearTimeout(timer));
    });
    onCleanup(() =>
        log(
            unmountLine(
                blockId,
                { controller: phases.controller(), reCover: phases.reCover() },
                performance.now() - mountedAt,
                ctx.why?.() ?? "",
                agent()
            )
        )
    );
}

/** What `thing` writes: `shown`, then `still-shown` once a second, then `cleared` (`thing` is the covering element's name). */
export function shownLine(blockId: string, thing: string, state: "shown" | "still-shown" | "cleared", forMs: number, agent?: string): string {
    const time = state === "shown" ? "" : ` ${state === "cleared" ? "after" : "for"}=${Math.round(forMs)}ms`;
    return `${head(blockId, agent)} ${thing} ${state}${time}`;
}

/**
 * Logs how long something in front of a pane stays up: `shown` now, `still-shown` once a second (at most
 * `STILL_COVERED_MAX` lines), `cleared` when this scope is cleaned up. Call during setup of the covering component,
 * or in an effect that runs only while the covering thing is up.
 */
export function logWhileShown(blockId: string, thing: string, agent: () => string | undefined): void {
    const since = performance.now();
    const log = (state: "shown" | "still-shown" | "cleared") => console.info("[pane-cover]", shownLine(blockId, thing, state, performance.now() - since, agent()));
    log("shown");
    let lines = 0;
    const timer = setInterval(() => {
        if (++lines > STILL_COVERED_MAX) return clearInterval(timer);
        log("still-shown");
    }, 1000);
    onCleanup(() => {
        clearInterval(timer);
        log("cleared");
    });
}

/** The transcript's show-cover (`data-show-cover`) for `blockId`: logged while it is up, as `show-cover`. */
export function logShowCover(blockId: string | undefined, covered: () => boolean, agent: () => string | undefined = () => undefined): void {
    if (!blockId) return;
    const up = createMemo(covered);
    createEffect(() => {
        if (up()) logWhileShown(blockId, "show-cover", agent);
    });
}

/** What a Block calls: the log for its block's cover, reading what it needs from the block when a line is written. */
export function logBlockCover(
    blockId: string,
    readiness: PaneReadiness,
    reCover: () => PaneReadinessPhase,
    block: () => { meta?: Record<string, unknown> } | undefined,
    tabDisplayed: () => boolean
): void {
    logCoverPhases(blockId, { controller: readiness.phase, reCover }, blockCoverContext(blockId, block, tabDisplayed, readiness.pendingGates));
}
