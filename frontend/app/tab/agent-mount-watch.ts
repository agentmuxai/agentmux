// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Names the step that did not happen when an agent pane never starts. After an
 * upgrade restart, four agent panes in one tab never resumed: their blocks
 * loaded, but no log line said whether the pane's view was ever created. A pane
 * that exists in the layout registers here; the agent view reports when it
 * mounts; one that has not after {@link MOUNT_WATCH_MS} logs what is known about
 * it, once.
 */

export const MOUNT_WATCH_MS = 20_000;

/** Clock and log sink, swappable in tests. */
export const mountWatchEnv = {
    now: (): number => performance.now(),
    log: (line: string): void => console.warn(line),
};

const waiting = new Map<string, { timer: ReturnType<typeof setTimeout>; since: number }>();
/** Blocks whose wait ended with a mount (or that are not agents): not watched again until forgotten. */
const settled = new Set<string>();

/** Start waiting for `blockId`'s agent view to mount. `describe` is read when the wait runs out. */
export function expectAgentMount(blockId: string, describe: () => string): void {
    if (!blockId || waiting.has(blockId) || settled.has(blockId)) return;
    const since = mountWatchEnv.now();
    const timer = setTimeout(() => {
        waiting.delete(blockId);
        settled.add(blockId); // said once; a later run of the pane's effect does not say it again
        const seconds = Math.round((mountWatchEnv.now() - since) / 1000);
        mountWatchEnv.log(`[pane-mount] block=${blockId.slice(0, 8)} agent view has not started after ${seconds} s: ${describe()}`);
    }, MOUNT_WATCH_MS);
    waiting.set(blockId, { timer, since });
}

/** The agent view mounted, or the block is not an agent (or gone): stop waiting. */
export function agentMountSettled(blockId: string): void {
    settled.add(blockId);
    const entry = waiting.get(blockId);
    if (!entry) return;
    clearTimeout(entry.timer);
    waiting.delete(blockId);
}

/** The block left its pane: stop waiting, and watch it afresh if it comes back. */
export function agentMountForgotten(blockId: string): void {
    agentMountSettled(blockId);
    settled.delete(blockId);
}
