// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the agent pane does when its drawer shell's process exits cleanly —
 * SPEC_AGENT_PANE_SHELL_EXIT_COLLAPSES_DRAWER_2026_09_15.md §3.2.
 *
 * Extracted from `agent-view.tsx` purely so it can be tested: that file is a
 * ~3000-line component with no render harness, and these three effects are
 * exactly the kind of thing that silently stops happening. Each one failing
 * leaves a DIFFERENT broken state, which is why they're asserted separately
 * rather than as "it collapsed":
 *
 *   - **pointer not cleared** → `term:shellsubblockid` dangles at a dead
 *     block, and the next drawer open resyncs a `STATUS_DONE` controller
 *     straight back into the respawn loop
 *     `SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md` fixed;
 *   - **sub-block not deleted** → the block leaks until the whole pane
 *     closes, and the pane-level cleanup can't get it either since it reads
 *     the very pointer we just cleared;
 *   - **drawer not collapsed** → the original bug: a dead terminal sitting
 *     open, silently accepting keystrokes that go nowhere.
 */

export interface ShellExitCollapseOptions {
    /** The agent block that owns the drawer. */
    parentBlockId: string;
    /**
     * The sub-block that just exited, or `undefined` if the pane never had
     * one (or already handled its exit). When `undefined` the drawer is still
     * collapsed — the shell is gone either way and an open drawer around
     * nothing is the state being fixed — but nothing is deleted or cleared.
     */
    exitedSubBlockId: string | undefined;
    /** Drop the parent's `Terminal.write` closure (the disposed-terminal guard). */
    clearTermWrite: () => void;
    /** Dispatch `DetailsCollapse` on the pane model. */
    collapseDrawer: () => void;
    setMeta: (args: { oref: string; meta: Record<string, unknown> }) => Promise<unknown>;
    deleteSubBlock: (args: { blockid: string }) => Promise<unknown>;
    makeORef: (otype: string, oid: string) => string;
}

/**
 * The two RPCs are AWAITED IN ORDER, not fired concurrently (Codex P2 on PR
 * #3253). The server engine spawns each request independently, and
 * `DeleteSubBlockCommand` read-modify-writes the parent to drop the child
 * from `subblockids` — so if its read lands before `SetMetaCommand` clears
 * `term:shellsubblockid` but its write lands after, it writes its stale
 * snapshot back and RESTORES the dead pointer. The pane would then point at a
 * deleted block, sending the next drawer open through the stale-reference
 * recovery path for no reason.
 *
 * The UI half (`clearTermWrite`/`collapseDrawer`) runs first and
 * synchronously: it needs no round trip, and the human should see the drawer
 * go the instant their shell does.
 */
export async function collapseDrawerOnShellExit(opts: ShellExitCollapseOptions): Promise<void> {
    opts.clearTermWrite();
    opts.collapseDrawer();
    if (!opts.exitedSubBlockId) return;
    await opts.setMeta({
        oref: opts.makeORef("block", opts.parentBlockId),
        meta: { "term:shellsubblockid": null },
    });
    await opts.deleteSubBlock({ blockid: opts.exitedSubBlockId });
}
