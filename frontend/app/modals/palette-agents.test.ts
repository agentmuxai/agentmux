// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Selecting an agent in the palette: show its pane (this window's, or another tab's or window's), open it
// when it has none and say why when srv refuses; and the list follows the agents.

import { createRoot } from "solid-js";
import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    local: new Map<string, string>(),
    panes: [] as { block_id: string; agent_id: string; window_ids: string[]; tab_name?: string }[],
    panesFail: false,
    open: vi.fn(async (): Promise<string | undefined> => undefined),
    reveal: vi.fn(async () => ({})),
    notify: vi.fn(),
}));

vi.mock("@/app/store/agent-pane-state-store", () => ({ getOpenDefinitionMap: () => h.local }));
vi.mock("@/app/store/agent-runtime-live", async () => {
    const { createSignal } = await import("solid-js");
    const [rows, setRows] = createSignal<unknown[]>([]);
    return { openAgentInActiveTab: h.open, sharedAgentRuntime: () => ({ rows }), __setRows: setRows };
});
vi.mock("@/app/store/global", () => ({ pushNotification: h.notify }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentOpenPanesCommand: async () => {
            if (h.panesFail) throw new Error("no srv");
            return { panes: h.panes };
        },
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/window-identity", () => ({ windowId: () => "w1" }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: h.reveal }));
vi.mock("@/app/view/agent/components/agent-definition-hooks", async () => {
    const { createSignal } = await import("solid-js");
    const [defs, setDefs] = createSignal<unknown[]>([]);
    return { useAgentDefinitions: () => [defs, () => false], __setDefs: setDefs };
});

import * as live from "@/app/store/agent-runtime-live";
import * as definitions from "@/app/view/agent/components/agent-definition-hooks";
import { goToAgent, usePaletteAgents } from "./palette-agents";

const setRows = (live as unknown as { __setRows: (r: unknown[]) => void }).__setRows;
const setDefs = (definitions as unknown as { __setDefs: (d: unknown[]) => void }).__setDefs;

beforeEach(() => {
    h.local = new Map();
    h.panes = [];
    h.panesFail = false;
    setRows([]);
    setDefs([]);
    vi.clearAllMocks();
});

describe("goToAgent", () => {
    it("shows the pane of an agent that is open in this window", async () => {
        h.local.set("k", "blk-1");
        await goToAgent({ agent_id: "k", name: "Korp" });
        expect(h.reveal).toHaveBeenCalledWith("blk-1", { source: "palette" });
        expect(h.open).not.toHaveBeenCalled();
    });

    it("shows the pane of an agent open in another tab or window, as srv reports it", async () => {
        h.panes = [{ block_id: "blk-9", agent_id: "k", window_ids: ["w2"], tab_name: "Team" }];
        await goToAgent({ agent_id: "k", name: "Korp" });
        expect(h.reveal).toHaveBeenCalledWith("blk-9", { source: "palette" });
        expect(h.open).not.toHaveBeenCalled();
    });

    it("opens an agent that has no pane in the active tab", async () => {
        await goToAgent({ agent_id: "k", name: "Korp" });
        expect(h.open).toHaveBeenCalledWith({ agent_id: "k", name: "Korp" });
        expect(h.reveal).not.toHaveBeenCalled();
        expect(h.notify).not.toHaveBeenCalled();
    });

    it("goes on with this window's panes when srv cannot say where the agents are", async () => {
        h.panesFail = true;
        h.local.set("k", "blk-1");
        await goToAgent({ agent_id: "k", name: "Korp" });
        expect(h.reveal).toHaveBeenCalledWith("blk-1", { source: "palette" });
    });

    it("says why when srv refuses to open it", async () => {
        h.open.mockResolvedValueOnce("This agent is open in another window");
        await goToAgent({ agent_id: "k", name: "Korp" });
        expect(h.notify).toHaveBeenCalledWith(
            expect.objectContaining({ title: "Couldn't open Korp", message: "This agent is open in another window", type: "error" })
        );
    });
});

describe("usePaletteAgents", () => {
    const row = (agent_id: string, name: string) => ({ agent_id, name, state: "idle", provider: "claude", icon: "robot", desired: "running" });

    it("lists the runtime rows with their seats, and follows both as they change", () => {
        createRoot((dispose) => {
            const entries = usePaletteAgents();
            expect(entries()).toEqual([]);

            setRows([row("k", "Korp")]);
            expect(entries().map((e) => [e.label, e.detail])).toEqual([["Korp", "idle"]]);

            setDefs([{ id: "k", seat: { role: "specialist", area: "browser engine" } }]);
            expect(entries()[0].detail).toBe("Specialist · browser engine · idle");

            setRows([row("k", "Korp-2"), row("n", "Newcomer")]); // a rename and a new agent
            expect(entries().map((e) => e.label)).toEqual(["Korp-2", "Newcomer"]);
            dispose();
        });
    });

    it("an entry's selection goes to that agent", async () => {
        h.local.set("k", "blk-1");
        await createRoot(async (dispose) => {
            setRows([row("k", "Korp")]);
            await usePaletteAgents()()[0].execute();
            dispose();
        });
        expect(h.reveal).toHaveBeenCalledWith("blk-1", { source: "palette" });
    });
});
