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

function harness(exitedSubBlockId: string | undefined) {
    const calls = {
        clearTermWrite: vi.fn(),
        collapseDrawer: vi.fn(),
        setMeta: vi.fn(),
        deleteSubBlock: vi.fn(),
    };
    collapseDrawerOnShellExit({
        parentBlockId: "agent-block-1",
        exitedSubBlockId,
        makeORef: (otype, oid) => `${otype}:${oid}`,
        ...calls,
    });
    return calls;
}

describe("collapseDrawerOnShellExit", () => {
    it("collapses the drawer", () => {
        expect(harness("shell-1").collapseDrawer).toHaveBeenCalledTimes(1);
    });

    /** Without this the pointer names a dead block, and the next drawer open
     *  resyncs a STATUS_DONE controller — the respawn loop
     *  SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md fixed, reintroduced through
     *  a different door. */
    it("clears the parent's term:shellsubblockid pointer", () => {
        const { setMeta } = harness("shell-1");
        expect(setMeta).toHaveBeenCalledWith({
            oref: "block:agent-block-1",
            meta: { "term:shellsubblockid": null },
        });
    });

    it("deletes the exited sub-block", () => {
        expect(harness("shell-1").deleteSubBlock).toHaveBeenCalledWith({ blockid: "shell-1" });
    });

    /** The parent holds a `Terminal.write` closure over an instance that is
     *  about to be disposed; TermWrap.dispose() doesn't null it out. */
    it("drops the parent's terminal write closure", () => {
        expect(harness("shell-1").clearTermWrite).toHaveBeenCalledTimes(1);
    });

    /** A pane with no shell id (exit already handled, or never opened) must
     *  still collapse — an open drawer around nothing is the state being
     *  fixed — but must not delete or clear anything, since the ids it would
     *  act on may since belong to a different, live shell. */
    it("still collapses with no sub-block id, but touches nothing else", () => {
        const { collapseDrawer, setMeta, deleteSubBlock } = harness(undefined);
        expect(collapseDrawer).toHaveBeenCalledTimes(1);
        expect(setMeta).not.toHaveBeenCalled();
        expect(deleteSubBlock).not.toHaveBeenCalled();
    });
});
