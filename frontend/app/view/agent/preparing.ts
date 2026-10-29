// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Preparing…" for a container agent whose container is being started in the background (the
 * pre-start, `prestartContainerAgent`): an image pull, the container coming up, the CLI installed
 * inside it. srv reports each step as a `cli_notice` row that updates in place (`cli-notice.ts`), so
 * the start's own state is in the document: the newest start-up row, while it is in progress.
 *
 * This is not the turn phase. A start-up row never counts as a working turn
 * (`stream-flush-queue.ts` `idleNoticeOnly`), and this does not change that: it is a separate line
 * above the composer, shown for the length of the pre-start and gone when it ends or fails (the row
 * itself then says which).
 */
import { cliNoticeText } from "./cli-notice";
import type { CliNoticeNode, DocumentNode } from "./types";

/** A start-up row still "in progress" after this long is a start that died without saying so. */
export const PREPARING_MAX_AGE_MS = 15 * 60 * 1000;
/** How often a showing line is checked against that age. */
export const PREPARING_RECHECK_MS = 30 * 1000;

export interface Preparing {
    /** Where the start is, in the row's own words ("Downloading the agent image… 42%"). */
    phase: string;
}

function inProgress(n: CliNoticeNode): boolean {
    if (n.kind === "container_start") return n.startState === "pulling" || n.startState === "starting";
    if (n.kind === "install") return n.state === "installing";
    return false;
}

/**
 * What the pre-start is doing, or null. Reads from the newest row back: the first start-up row
 * decides (in progress: preparing; ready, installed, failed: not), and a message or a reply after it
 * means the agent is already up.
 */
export function preparingNow(nodes: readonly DocumentNode[], now: number): Preparing | null {
    for (let i = nodes.length - 1; i >= 0; i--) {
        const n = nodes[i];
        if (n.type === "user_message" || n.type === "agent_message" || n.type === "markdown" || n.type === "tool") return null;
        if (n.type !== "cli_notice") continue;
        if (n.kind !== "container_start" && n.kind !== "install") continue;
        if (!inProgress(n)) return null;
        if (now - n.timestamp > PREPARING_MAX_AGE_MS) return null;
        return { phase: cliNoticeText(n).label };
    }
    return null;
}
