// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { createRoot } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import { createAgentRuntimeStore, stoppedAgents } from "./swarm-runtime";

const row = (agent_id: string, state: AgentRuntime["state"], extra: Partial<AgentRuntime> = {}): AgentRuntime => ({
    agent_id,
    state,
    desired: state === "stopped" || state === "error" ? "stopped" : "running",
    name: agent_id,
    provider: "claude",
    icon: "",
    ...extra,
});

function fakeDeps(initial: AgentRuntime[]) {
    let rows = initial;
    const handlers = new Map<string, (r: AgentRuntime) => void>();
    let agentsChanged: () => void = () => {};
    const unsubscribed: string[] = [];
    const deps = {
        list: vi.fn(async () => rows),
        subscribe: vi.fn((id: string, h: (r: AgentRuntime) => void) => {
            handlers.set(id, h);
            return () => {
                handlers.delete(id);
                unsubscribed.push(id);
            };
        }),
        onAgentsChanged: vi.fn((h: () => void) => {
            agentsChanged = h;
            return () => {};
        }),
    };
    return {
        deps,
        handlers,
        unsubscribed,
        setRows: (next: AgentRuntime[]) => (rows = next),
        changeAgents: () => agentsChanged(),
    };
}

describe("the agent runtime store", () => {
    it("lists every agent and follows each one's changes", async () => {
        const f = fakeDeps([row("a", "stopped"), row("b", "idle")]);
        await createRoot(async (dispose) => {
            const store = createAgentRuntimeStore(f.deps);
            await store.load();
            expect(store.rows().map((r) => r.agent_id)).toEqual(["a", "b"]);
            expect([...f.handlers.keys()]).toEqual(["a", "b"]);
            f.handlers.get("a")!(row("a", "working", { block_id: "blk" }));
            expect(store.rows().find((r) => r.agent_id === "a")).toMatchObject({ state: "working", block_id: "blk" });
            dispose();
        });
    });

    it("reads the list again when the agents change, following a new agent and dropping a deleted one", async () => {
        const f = fakeDeps([row("a", "stopped"), row("b", "stopped")]);
        await createRoot(async (dispose) => {
            const store = createAgentRuntimeStore(f.deps);
            await store.load();
            f.setRows([row("b", "stopped"), row("c", "stopped")]);
            f.changeAgents();
            await store.load();
            expect(store.rows().map((r) => r.agent_id)).toEqual(["b", "c"]);
            expect(f.unsubscribed).toEqual(["a"]);
            expect(f.deps.subscribe).toHaveBeenCalledTimes(3);
            dispose();
        });
    });

    it("keeps what it has when the list fails, and stops following when disposed", async () => {
        const f = fakeDeps([row("a", "stopped")]);
        await createRoot(async (dispose) => {
            const store = createAgentRuntimeStore(f.deps);
            await store.load();
            f.deps.list.mockRejectedValueOnce(new Error("older srv"));
            await store.load();
            expect(store.rows().map((r) => r.agent_id)).toEqual(["a"]);
            store.dispose();
            expect(f.unsubscribed).toEqual(["a"]);
            dispose();
        });
    });

    it("reads once more, not in parallel, for a change that arrives while it reads", async () => {
        const f = fakeDeps([row("a", "stopped")]);
        await createRoot(async (dispose) => {
            const store = createAgentRuntimeStore(f.deps);
            const first = store.load();
            const second = store.load();
            await Promise.all([first, second]);
            expect(f.deps.list).toHaveBeenCalledTimes(2);
            dispose();
        });
    });
});

describe("stoppedAgents", () => {
    it("lists the agents with no process that no live row shows, by name", () => {
        const rows = [
            row("z", "stopped", { name: "Zed" }),
            row("a", "error", { name: "alpha" }),
            row("m", "idle", { name: "mid", block_id: "b-live" }),
            row("p", "stopped", { name: "parked", block_id: "b-parked" }),
            row("l", "stopped", { name: "listed", block_id: "b-listed" }),
        ];
        expect(stoppedAgents(rows, new Set(["b-listed"])).map((r) => r.name)).toEqual(["alpha", "parked", "Zed"]);
    });
});
