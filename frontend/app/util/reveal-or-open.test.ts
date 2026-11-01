// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import {
    outcomeBlockId,
    raiseVerdict,
    revealOrOpen,
    type GoResult,
    type PaneElsewhere,
    type RevealDeps,
    type RevealOutcome,
    type RevealRequest,
} from "./reveal-or-open";

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

/** What the host answers when a window is raised: it came forward, or the system refused. */
const FOREGROUND = { found: true, raised: true, restored: false, moved: false, foreground: true } as FocusWindowResult;
const KEPT_BEHIND = { ...FOREGROUND, foreground: false } as FocusWindowResult;

const agentPane = (over: Partial<PaneElsewhere> = {}): PaneElsewhere => ({
    blockId: "far",
    windowId: "w2",
    tabId: "t9",
    workspaceId: "ws2",
    name: "Parlo",
    immovable: true,
    ...over,
});

/** A request for an `app` scope pane, with a fake host that says what the other window and the OS do. */
function runApp(host: { found: PaneElsewhere | null; go: GoResult }, req: Partial<RevealRequest> = {}) {
    const calls: string[] = [];
    const deps: RevealDeps = {
        recordOpened: () => void calls.push("record"),
        findMounted: () => ({}),
        findInWindow: async () => null,
        setMeta: async (id) => void calls.push(`meta:${id}`),
        revealInWindow: async () => ({ ok: true }),
        create: async () => {
            calls.push("create");
            return "new-block";
        },
        app: {
            findInOtherWindow: async () => host.found,
            goTo: async (f) => {
                calls.push(`go:${f.blockId}`);
                return host.go;
            },
            offer: (f, why) => void calls.push(`offer:${f.blockId}:${why}`),
        },
    };
    const request: RevealRequest = { view: "agent", source: "test", scope: "app", ...req };
    return revealOrOpen(request, deps).then((outcome) => ({ outcome, calls }));
}

describe("revealOrOpen, scope app", () => {
    it("goes to the pane another window holds, and says so once the raise is confirmed", async () => {
        const got = await runApp({ found: agentPane(), go: { shown: "revealed", raise: FOREGROUND } });
        expect(got.outcome).toEqual({ kind: "went_to", blockId: "far", windowId: "w2" });
        expect(got.calls).toEqual(["record", "go:far"]);
    });

    it("tells the person where the pane is when the system keeps its window behind", async () => {
        const got = await runApp({ found: agentPane(), go: { shown: "revealed", raise: KEPT_BEHIND } });
        expect(got.outcome).toEqual({ kind: "unverified", blockId: "far", windowId: "w2" });
        expect(got.calls).toEqual(["record", "go:far", "offer:far:refused"]);
    });

    it("does not claim success when the host never answered", async () => {
        const silent = await runApp({ found: agentPane(), go: { shown: "revealed", raise: { ...FOREGROUND, timed_out: true } } });
        expect(silent.outcome.kind).toBe("unverified");
        expect(silent.calls).toContain("offer:far:unknown");
        const noWindow = await runApp({ found: agentPane(), go: { shown: "revealed" } });
        expect(noWindow.outcome.kind).toBe("unverified");
        const slow = await runApp({ found: agentPane(), go: { shown: "timeout", raise: FOREGROUND } });
        expect(slow.outcome.kind).toBe("unverified");
    });

    it("applies the focus meta to the pane it goes to", async () => {
        const got = await runApp({ found: agentPane(), go: { shown: "revealed", raise: FOREGROUND } }, { focusMeta: { "settings:section": "x" } });
        expect(got.calls).toEqual(["record", "meta:far", "go:far"]);
    });

    it("opens a pane here when no other window holds one", async () => {
        const got = await runApp({ found: null, go: { shown: "revealed", raise: FOREGROUND } });
        expect(got.outcome).toEqual({ kind: "opened_here", blockId: "new-block" });
        expect(got.calls).toEqual(["record", "create"]);
    });

    it("opens a pane here when the other window's pane was just closed", async () => {
        const got = await runApp({ found: agentPane(), go: { shown: "failed", reason: "not-found" } });
        expect(got.outcome).toEqual({ kind: "opened_here", blockId: "new-block" });
    });

    it("reports a window that could not show its pane, and where it is", async () => {
        const got = await runApp({ found: agentPane(), go: { shown: "failed", reason: "tab-timeout" } });
        expect(got.outcome).toEqual({ kind: "failed", reason: "tab-timeout", blockId: "far" });
        expect(got.calls).toContain("offer:far:unknown");
    });

    it("looks in this window first, and never looks elsewhere for scope window or a forced new pane", async () => {
        const here = await runApp({ found: agentPane(), go: { shown: "revealed", raise: FOREGROUND } }, { scope: "window" });
        expect(here.calls).toEqual(["record", "create"]);
        const forced = await runApp({ found: agentPane(), go: { shown: "revealed", raise: FOREGROUND } }, { forceNew: true });
        expect(forced.calls).toEqual(["record", "create"]);
    });
});

describe("raiseVerdict", () => {
    it("is verified only for a raised window that is the foreground one when read back", () => {
        expect(raiseVerdict(FOREGROUND)).toBe("verified");
        expect(raiseVerdict(KEPT_BEHIND)).toBe("refused");
        expect(raiseVerdict({ ...FOREGROUND, found: false, raised: false, foreground: false })).toBe("refused");
    });

    it("knows nothing without an answer or on a timeout", () => {
        expect(raiseVerdict(undefined)).toBe("unknown");
        expect(raiseVerdict({ ...FOREGROUND, timed_out: true })).toBe("unknown");
    });
});

describe("revealOrOpen, the pane of one agent", () => {
    it("asks this window for the pane with the requested identity, not for any pane of the view", async () => {
        const asked: unknown[][] = [];
        const deps: RevealDeps = {
            recordOpened: () => {},
            findMounted: (...args) => (asked.push(["mounted", ...args]), { here: undefined, elsewhere: undefined }),
            findInWindow: async (...args) => (asked.push(["window", ...args]), null),
            setMeta: async () => {},
            revealInWindow: async () => ({ ok: true }),
            create: async () => "new-block",
        };
        const def = { meta: { view: "agent", agentId: "b" } } as BlockDef;
        await revealOrOpen({ view: "agent", blockDef: def, source: "test", scope: "app" }, deps);
        expect(asked).toEqual([
            ["mounted", "agent", "agent:b"],
            ["window", "agent", "agent:b"],
        ]);
    });
});
