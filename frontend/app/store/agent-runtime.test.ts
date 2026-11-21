// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { createEffect, createRoot } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import { createAgentRuntimeStore, runtimeCounts } from "./agent-runtime";

const row = (agent_id: string, state: AgentRuntime["state"], extra: Partial<AgentRuntime> = {}): AgentRuntime => ({
    agent_id,
    state,
    desired: state === "stopped" || state === "error" ? "stopped" : "running",
    name: agent_id,
    provider: "claude",
    icon: "",
    runtime: "host",
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

describe("what an event wakes", () => {
    const AGENTS = 25;
    const ids = Array.from({ length: AGENTS }, (_, i) => `a${i}`);
    const start = async (dispose: () => void) => {
        const f = fakeDeps(ids.map((id, i) => row(id, "idle", { block_id: `blk${i}` })));
        const store = createAgentRuntimeStore(f.deps);
        await store.load();
        return { f, store, dispose };
    };

    it("wakes the readers of the agent that changed, and no other agent's", async () => {
        await createRoot(async (dispose) => {
            const { f, store } = await start(dispose);
            const runs = new Map<string, number>();
            ids.forEach((id) =>
                createEffect(() => {
                    store.rowOf(id);
                    runs.set(id, (runs.get(id) ?? 0) + 1);
                })
            );
            await Promise.resolve();
            f.handlers.get("a3")!(row("a3", "working", { block_id: "blk3" }));
            await Promise.resolve();
            expect(runs.get("a3")).toBe(2);
            for (const id of ids.filter((x) => x !== "a3")) expect(runs.get(id), id).toBe(1);
            dispose();
        });
    });

    it("wakes nothing for an event that says what the row already says", async () => {
        await createRoot(async (dispose) => {
            const { f, store } = await start(dispose);
            let one = 0;
            let all = 0;
            createEffect(() => (store.rowOf("a1"), one++));
            createEffect(() => (store.rows(), all++));
            await Promise.resolve();
            f.handlers.get("a1")!(row("a1", "idle", { block_id: "blk1" }));
            await Promise.resolve();
            expect([one, all]).toEqual([1, 1]);
            dispose();
        });
    });

    it("wakes the readers of the list once per event, and a pane's reader only for its own agent", async () => {
        await createRoot(async (dispose) => {
            const { f, store } = await start(dispose);
            let all = 0;
            let pane = 0;
            createEffect(() => (store.rows(), all++));
            createEffect(() => (store.rowOfBlock("blk7"), pane++));
            await Promise.resolve();
            f.handlers.get("a2")!(row("a2", "working", { block_id: "blk2" }));
            await Promise.resolve();
            expect([all, pane]).toEqual([2, 1]);
            f.handlers.get("a7")!(row("a7", "working", { block_id: "blk7" }));
            await Promise.resolve();
            expect([all, pane]).toEqual([3, 2]);
            dispose();
        });
    });

    it("moves a pane's reader to the agent now in that pane", async () => {
        await createRoot(async (dispose) => {
            const { f, store } = await start(dispose);
            const seen: (string | undefined)[] = [];
            createEffect(() => seen.push(store.rowOfBlock("blk-new")?.agent_id));
            await Promise.resolve();
            f.handlers.get("a4")!(row("a4", "idle", { block_id: "blk-new" }));
            await Promise.resolve();
            expect(seen).toEqual([undefined, "a4"]);
            dispose();
        });
    });

    it("says nothing for an agent the list does not have, then has it when the list does", async () => {
        const f = fakeDeps([row("a", "idle")]);
        await createRoot(async (dispose) => {
            const store = createAgentRuntimeStore(f.deps);
            await store.load();
            const seen: (string | undefined)[] = [];
            createEffect(() => seen.push(store.rowOf("later")?.state));
            await Promise.resolve();
            f.setRows([row("a", "idle"), row("later", "working")]);
            await store.load();
            await Promise.resolve();
            expect(seen).toEqual([undefined, "working"]);
            f.setRows([row("a", "idle")]);
            await store.load();
            await Promise.resolve();
            expect(seen).toEqual([undefined, "working", undefined]);
            dispose();
        });
    });

    // The frame budget (Loap-3): a message costs at most 16 ms, and a burst of one from every agent at most 50 ms, with
    // 25 agents streaming and a reader on each (a row of My Agents, a Swarm card), the status bar's and the pane's.
    it("costs under 16 ms a message and 50 ms a burst with 25 agents streaming", async () => {
        await createRoot(async (dispose) => {
            const { f, store } = await start(dispose);
            ids.forEach((id, i) => {
                createEffect(() => store.rowOf(id));
                createEffect(() => store.rowOfBlock(`blk${i}`));
            });
            createEffect(() => runtimeCounts(store.rows()));
            await Promise.resolve();
            const states: AgentRuntime["state"][] = ["working", "idle", "waiting_on_user", "working"];
            let worstMessage = 0;
            let worstBurst = 0;
            for (let round = 0; round < 40; round++) {
                const burst = performance.now();
                ids.forEach((id, i) => {
                    const t = performance.now();
                    f.handlers.get(id)!(row(id, states[(round + i) % states.length], { block_id: `blk${i}`, detail: `step ${round}` }));
                    worstMessage = Math.max(worstMessage, performance.now() - t);
                });
                worstBurst = Math.max(worstBurst, performance.now() - burst);
            }
            expect(worstMessage).toBeLessThan(16);
            expect(worstBurst).toBeLessThan(50);
            dispose();
        });
    });
});
