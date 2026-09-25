// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { busyMembers, closesWithShutdownLog, describeBusyMember, type PaneCloseProbe } from "./pane-close-guard";

function probe(state: Record<string, { turn?: boolean | null; procs?: number; fail?: boolean }>): PaneCloseProbe {
    return {
        turnActive: async (id) => {
            if (state[id]?.fail) throw new Error("rpc down");
            return state[id]?.turn ?? false;
        },
        processCount: async (id) => {
            if (state[id]?.fail) throw new Error("rpc down");
            return state[id]?.procs ?? 0;
        },
        name: (id) => id.toUpperCase(),
    };
}

describe("pane close guard", () => {
    it("reports only members with a turn running or tracked processes", async () => {
        const busy = await busyMembers(["a", "b", "c"], probe({ a: { turn: true }, b: {}, c: { procs: 2 } }));
        expect(busy.map((m) => m.blockId)).toEqual(["a", "c"]);
    });

    it("an idle pane needs no prompt", async () => {
        expect(await busyMembers(["a", "b"], probe({ a: {}, b: { turn: null } }))).toEqual([]);
    });

    it("a failing status lookup never blocks the close", async () => {
        expect(await busyMembers(["a"], probe({ a: { fail: true } }))).toEqual([]);
    });

    it("describes what each busy agent would lose", () => {
        expect(describeBusyMember({ blockId: "p", name: "Posa", turnActive: true, processCount: 2 })).toBe(
            "Posa — mid-turn, 2 processes running"
        );
        expect(describeBusyMember({ blockId: "m", name: "Manoz", turnActive: false, processCount: 1 })).toBe(
            "Manoz — 1 process running"
        );
    });
});

describe("closesWithShutdownLog (SPEC_AGENT_SELF_QUIT §5.5)", () => {
    const isAgent = (id: string) => id.startsWith("agent");
    it("keeps a pane that holds any agent on screen for its shutdown log", () => {
        expect(closesWithShutdownLog(["agent-1"], isAgent)).toBe(true);
        expect(closesWithShutdownLog(["term-1", "agent-2"], isAgent)).toBe(true);
    });
    it("closes terminal/editor/browser panes at once, as before", () => {
        expect(closesWithShutdownLog(["term-1", "editor-1"], isAgent)).toBe(false);
        expect(closesWithShutdownLog([], isAgent)).toBe(false);
    });
});
