// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The `[pane-cover]` log: one line per change of a block's cover phase, so a pane that stayed under the logo
 * cover (or came back under it) says when and from which source in `muxlog fe grep pane-cover`. The cover has two
 * sources, the readiness controller (assembly, once) and the block's own re-cover (every subagent "started" event);
 * before this log nothing recorded the second, and a forced reveal was the only line the first one wrote.
 */
import { createEffect, on, untrack } from "solid-js";
import type { PaneReadinessPhase } from "@/app/store/pane-readiness";

export type CoverSource = "controller" | "re-cover";

/** `[pane-cover] block=abc1234 assembling→revealing src=controller t=1103ms` (without the tag). */
export function coverLogLine(
    blockId: string,
    prev: PaneReadinessPhase | undefined,
    next: PaneReadinessPhase,
    source: CoverSource,
    sinceMountMs: number
): string {
    return `block=${blockId.slice(0, 7)} ${prev ?? "mount"}→${next} src=${source} t=${Math.round(sinceMountMs)}ms`;
}

/**
 * Logs every change of either source's phase, each under its own source: the controller's from mount, the
 * re-cover's from its first change (it starts out `live`, which is not a change). Reading the source from
 * "which one is live now" would call the controller's own last step (revealing to live) a re-cover.
 * Call during component setup (it creates effects).
 */
export function logCoverPhases(
    blockId: string,
    phases: { controller: () => PaneReadinessPhase; reCover: () => PaneReadinessPhase }
): void {
    const mountedAt = performance.now();
    const log = (source: CoverSource, prev: PaneReadinessPhase | undefined, next: PaneReadinessPhase) => {
        console.info("[pane-cover]", coverLogLine(blockId, prev, next, source, performance.now() - mountedAt));
    };
    createEffect(on(phases.controller, (next, prev) => log("controller", prev, next)));
    // `on` hands a deferred effect no previous value on its first run, so the re-cover keeps its own.
    let reCoverPrev = untrack(phases.reCover);
    createEffect(
        on(
            phases.reCover,
            (next) => {
                log("re-cover", reCoverPrev, next);
                reCoverPrev = next;
            },
            { defer: true }
        )
    );
}
