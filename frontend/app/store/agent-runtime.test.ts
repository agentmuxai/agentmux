// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { createRoot } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import { createAgentRuntimeStore, runtimeCounts } from "./agent-runtime";

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
    let tick: () => void = () => {};
    let ticking = false;
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
        every: vi.fn((_ms: number, t: () => void) => {
            tick = t;
            ticking = true;
            return () => {
                ticking = false;
            };
        }),
    };
    return {
        deps,
        handlers,
        unsubscribed,
        setRows: (next: AgentRuntime[]) => (rows = next),
        changeAgents: () => agentsChanged(),
        tick: () => tick(),
        isTicking: () => ticking,
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

    it("reads the list again on its interval, for an agent created or deleted with no agents:changed", async () => {
        const f = fakeDeps([row("a", "stopped")]);
        await createRoot(async (dispose) => {
            const store = createAgentRuntimeStore(f.deps);
            await store.load();
            f.setRows([row("a", "stopped"), row("n", "stopped")]);
            f.tick();
            await store.load();
            expect(store.rows().map((r) => r.agent_id)).toEqual(["a", "n"]);
            store.dispose();
            expect(f.isTicking()).toBe(false);
            dispose();
        });
    });

    it("keeps what an event said while a read was out, which is newer than the list", async () => {
        const f = fakeDeps([row("a", "idle")]);
        await createRoot(async (dispose) => {
            const store = createAgentRuntimeStore(f.deps);
            await store.load();
            // The next read answers with a snapshot taken before the agent stopped.
            let release: (rows: AgentRuntime[]) => void = () => {};
            f.deps.list.mockImplementationOnce(() => new Promise<AgentRuntime[]>((resolve) => (release = resolve)));
            const reading = store.load();
            f.handlers.get("a")!(row("a", "stopped"));
            release([row("a", "idle")]);
            await reading;
            expect(store.rows().find((r) => r.agent_id === "a")!.state).toBe("stopped");
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

describe("runtimeCounts", () => {
    it("counts the agents with a process (a limit or a sign-in holds one, it does not end it), and the ones waiting on the user among them", () => {
        const rows = [
            row("a", "stopped"),
            row("b", "error"),
            row("c", "limited"),
            row("d", "signed_out"),
            row("e", "starting"),
            row("f", "idle"),
            row("g", "working"),
            row("h", "waiting_on_user"),
            row("i", "waiting_on_user"),
        ];
        expect(runtimeCounts(rows)).toEqual({ running: 7, waiting: 2 });
        expect(runtimeCounts([])).toEqual({ running: 0, waiting: 0 });
    });
});
