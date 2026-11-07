// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The `[pane-cover]` log: one line per change of a block's cover phase, so a pane that stayed under the logo
 * cover (or came back under it) says when and from which source in `muxlog fe grep pane-cover`. The cover has two
 * sources, the readiness controller (assembly, once) and the block's own re-cover (every subagent "started" event);
 * before this log nothing recorded the second, and a forced reveal was the only line the first one wrote.
 */
import { createEffect, on } from "solid-js";
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

/** Logs every change of `phase`. Call during component setup (it creates an effect). */
export function logCoverPhases(blockId: string, phase: () => PaneReadinessPhase, source: () => CoverSource): void {
    const mountedAt = performance.now();
    createEffect(
        on(phase, (next, prev) => {
            console.info("[pane-cover]", coverLogLine(blockId, prev, next, source(), performance.now() - mountedAt));
        })
    );
}
