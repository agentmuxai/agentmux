// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SPEC_AGENT_PANE_SHELL_EXIT_COLLAPSES_DRAWER_2026_09_15.md §4, cases 3 and 5.
 *
 * Asserts the three effects of a clean shell exit SEPARATELY rather than as
 * one "it collapsed" — each has its own distinct failure mode (dangling
 * pointer → respawn loop; undeleted sub-block → leak; open drawer → the
 * original bug), so a single combined assertion would let two of them
 * regress silently behind the third.
 */

import { describe, expect, it, vi } from "vitest";
import { collapseDrawerOnShellExit } from "./shell-exit-collapse";

async function harness(exitedSubBlockId: string | undefined) {
    const calls = {
        clearTermWrite: vi.fn(),
        collapseDrawer: vi.fn(),
        setMeta: vi.fn(() => Promise.resolve()),
        deleteSubBlock: vi.fn(() => Promise.resolve()),
    };
    await collapseDrawerOnShellExit({
        parentBlockId: "agent-block-1",
        exitedSubBlockId,
        makeORef: (otype, oid) => `${otype}:${oid}`,
        ...calls,
    });
    return calls;
}

describe("collapseDrawerOnShellExit", () => {
    it("collapses the drawer", async () => {
        expect((await harness("shell-1")).collapseDrawer).toHaveBeenCalledTimes(1);
    });

    /** Without this the pointer names a dead block, and the next drawer open
     *  resyncs a STATUS_DONE controller — the respawn loop
     *  SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md fixed, reintroduced through
     *  a different door. */
    it("clears the parent's term:shellsubblockid pointer", async () => {
        const { setMeta } = await harness("shell-1");
        expect(setMeta).toHaveBeenCalledWith({
            oref: "block:agent-block-1",
            meta: { "term:shellsubblockid": null },
        });
    });

    it("deletes the exited sub-block", async () => {
        expect((await harness("shell-1")).deleteSubBlock).toHaveBeenCalledWith({ blockid: "shell-1" });
    });

    /** The parent holds a `Terminal.write` closure over an instance that is
     *  about to be disposed; TermWrap.dispose() doesn't null it out. */
    it("drops the parent's terminal write closure", async () => {
        expect((await harness("shell-1")).clearTermWrite).toHaveBeenCalledTimes(1);
    });

    /** A pane with no shell id (exit already handled, or never opened) must
     *  still collapse — an open drawer around nothing is the state being
     *  fixed — but must not delete or clear anything, since the ids it would
     *  act on may since belong to a different, live shell. */
    it("still collapses with no sub-block id, but touches nothing else", async () => {
        const { collapseDrawer, setMeta, deleteSubBlock } = await harness(undefined);
        expect(collapseDrawer).toHaveBeenCalledTimes(1);
        expect(setMeta).not.toHaveBeenCalled();
        expect(deleteSubBlock).not.toHaveBeenCalled();
    });

    /**
     * Codex P2 on PR #3253. `DeleteSubBlockCommand` read-modify-writes the
     * parent to drop the child from `subblockids`; fired concurrently with
     * the pointer clear, it can read a pre-clear snapshot and write it back
     * afterwards, RESTORING the dead pointer. The delete must not start until
     * the clear has resolved.
     */
    it("does not start the delete until the pointer clear has resolved", async () => {
        const order: string[] = [];
        let releaseSetMeta: (() => void) | undefined;
        const setMetaGate = new Promise<void>((resolve) => {
            releaseSetMeta = resolve;
        });

        const setMeta = vi.fn(() => {
            order.push("setMeta:start");
            return setMetaGate.then(() => void order.push("setMeta:done"));
        });
        const deleteSubBlock = vi.fn(() => {
            order.push("delete:start");
            return Promise.resolve();
        });

        const done = collapseDrawerOnShellExit({
            parentBlockId: "agent-block-1",
            exitedSubBlockId: "shell-1",
            clearTermWrite: vi.fn(),
            collapseDrawer: vi.fn(),
            setMeta,
            deleteSubBlock,
            makeORef: (otype, oid) => `${otype}:${oid}`,
        });

        // The clear is in flight and deliberately unresolved — the delete
        // must not have been issued yet.
        await Promise.resolve();
        expect(deleteSubBlock).not.toHaveBeenCalled();

        releaseSetMeta!();
        await done;

        expect(order).toEqual(["setMeta:start", "setMeta:done", "delete:start"]);
    });
});
