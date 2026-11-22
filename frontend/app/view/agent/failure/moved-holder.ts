// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pane an "Open here" is closing: the agent it held is running in the pane the person picked it in, so
 * closing the old one must not announce that the agent "keeps running without a pane". A mark is set just
 * before the close and used up by the close's note (`tab/tabcontent.tsx`); one never used lapses.
 */

const MARK_MS = 10_000;
const moved = new Map<string, number>();

/** The pane `blockId` is being closed because its agent moved to another pane. */
export function markHolderMoved(blockId: string, now = Date.now()): void {
    moved.set(blockId, now + MARK_MS);
}

/** True once for a pane marked with {@link markHolderMoved}; the mark is used up. */
export function takeHolderMoved(blockId: string, now = Date.now()): boolean {
    const until = moved.get(blockId);
    moved.delete(blockId);
    return until != null && until >= now;
}
