// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * When this window began opening a pane, kept until the pane's view asks for it. The host's
 * `[pane-open]` line starts at the create request; this is the part before it: the wait for the
 * backend to create the block, then the pane's code, view and first layout.
 */

/** Opens kept at once. One that no view asks about (a pane type that logs nothing) is dropped. */
const MAX_KEPT = 32;

const marks = new Map<string, { startedAt: number; blockAt: number }>();

/** The backend has just created `blockId`, for an open that began at `startedAt`. */
export function notePaneOpen(blockId: string, startedAt: number, now = performance.now()) {
    marks.set(blockId, { startedAt, blockAt: now });
    if (marks.size > MAX_KEPT) marks.delete(marks.keys().next().value as string);
}

/**
 * The window's part of opening `blockId`, to end a log line with, once:
 * ` [pane-open:fe] create_block=41ms mount=18ms total=59ms`. Empty for a pane this window did not
 * just open (one restored at startup, woken, or asked about a second time).
 */
export function takePaneOpenTiming(blockId: string, now = performance.now()): string {
    const m = marks.get(blockId);
    if (!m) return "";
    marks.delete(blockId);
    const ms = (n: number) => `${Math.round(n)}ms`;
    return ` [pane-open:fe] create_block=${ms(m.blockAt - m.startedAt)} mount=${ms(now - m.blockAt)} total=${ms(now - m.startedAt)}`;
}
