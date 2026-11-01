// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { outcomeBlockId, revealOrOpen, type RevealDeps, type RevealOutcome, type RevealRequest } from "./reveal-or-open";

type Reveal = Awaited<ReturnType<RevealDeps["revealInWindow"]>>;

/** What the window holds: a pane of the view in the shown layout, in another tab (mounted or not), or none. */
interface World {
    inLayout?: string;
    mountedElsewhere?: string;
    unmounted?: string;
    reveal?: Reveal;
}

function run(world: World, req: Partial<RevealRequest> = {}) {
    const calls: string[] = [];
    const deps: RevealDeps = {
        recordOpened: () => void calls.push("record"),
        findMounted: () => ({
            here: world.inLayout
                ? { blockId: world.inLayout, showAndFocus: () => void calls.push(`show:${world.inLayout}`) }
                : undefined,
            elsewhere: world.mountedElsewhere,
        }),
        findInWindow: async () => world.unmounted ?? null,
        setMeta: async (id) => void calls.push(`meta:${id}`),
        revealInWindow: async (id) => {
            calls.push(`reveal:${id}`);
            return world.reveal ?? { ok: true };
        },
        create: async () => {
            calls.push("create");
            return "new-block";
        },
    };
    return revealOrOpen({ view: "settings", source: "test", ...req }, deps).then((outcome) => ({ outcome, calls }));
}

describe("revealOrOpen", () => {
    // [name, world, request, outcome, calls]
    const table: [string, World, Partial<RevealRequest>, RevealOutcome, string[]][] = [
        ["a pane in the shown tab is shown and focused", { inLayout: "b1" }, {}, { kind: "shown_here", blockId: "b1" }, ["record", "show:b1"]],
        [
            "a pane in the shown tab gets the focus meta before it is shown",
            { inLayout: "b1" },
            { focusMeta: { "settings:section": "browser" } },
            { kind: "shown_here", blockId: "b1" },
            ["record", "meta:b1", "show:b1"],
        ],
        [
            "a pane mounted in another tab of this window is gone to",
            { mountedElsewhere: "b2" },
            {},
            { kind: "shown_here", blockId: "b2" },
            ["record", "reveal:b2"],
        ],
        [
            "a pane that is not mounted (an inactive stack member) is found in the window's tabs and gone to",
            { unmounted: "b3" },
            { focusMeta: { "settings:section": "browser" } },
            { kind: "shown_here", blockId: "b3" },
            ["record", "meta:b3", "reveal:b3"],
        ],
        ["no pane open: one is opened here", {}, {}, { kind: "opened_here", blockId: "new-block" }, ["record", "create"]],
        [
            "a pane that was just closed is replaced, not reported as missing",
            { mountedElsewhere: "b2", reveal: { ok: false, reason: "gone" } as Reveal },
            {},
            { kind: "opened_here", blockId: "new-block" },
            ["record", "reveal:b2", "create"],
        ],
        [
            "a pane that is not found is replaced",
            { unmounted: "b3", reveal: { ok: false, reason: "not-found" } as Reveal },
            {},
            { kind: "opened_here", blockId: "new-block" },
            ["record", "reveal:b3", "create"],
        ],
        [
            "a slow tab leaves the pane where it is: a second one would be a copy",
            { mountedElsewhere: "b2", reveal: { ok: false, reason: "tab-timeout" } as Reveal },
            {},
            { kind: "failed", reason: "tab-timeout", blockId: "b2" },
            ["record", "reveal:b2"],
        ],
        [
            "a newer click taking over also leaves the pane",
            { mountedElsewhere: "b2", reveal: { ok: false, reason: "superseded" } as Reveal },
            {},
            { kind: "failed", reason: "superseded", blockId: "b2" },
            ["record", "reveal:b2"],
        ],
        [
            "forceNew opens another even when one is shown",
            { inLayout: "b1" },
            { forceNew: true },
            { kind: "opened_here", blockId: "new-block" },
            ["record", "create"],
        ],
        [
            "scope app has no other window to look in yet: it opens here like scope window",
            {},
            { scope: "app" },
            { kind: "opened_here", blockId: "new-block" },
            ["record", "create"],
        ],
    ];

    it.each(table)("%s", async (_name, world, req, outcome, calls) => {
        const got = await run(world, req);
        expect(got.outcome).toEqual(outcome);
        expect(got.calls).toEqual(calls);
    });

    it("records the opening with the block definition it was given", async () => {
        const recordOpened = vi.fn();
        const def = { meta: { view: "settings", "settings:section": "x" } } as BlockDef;
        await revealOrOpen(
            { view: "settings", blockDef: def, source: "test" },
            {
                recordOpened,
                findMounted: () => ({}),
                findInWindow: async () => null,
                setMeta: async () => {},
                revealInWindow: async () => ({ ok: true }),
                create: async () => "n",
            }
        );
        expect(recordOpened).toHaveBeenCalledWith(def);
    });

    it("every outcome names the pane the person has, when there is one", () => {
        expect(outcomeBlockId({ kind: "opened_here", blockId: "a" })).toBe("a");
        expect(outcomeBlockId({ kind: "failed", reason: "x" })).toBeUndefined();
    });
});
