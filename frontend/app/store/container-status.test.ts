// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { ContainerRuntimeStatus } from "@/types/rpc/ContainerRuntimeStatus";
import type { ContainersList } from "@/types/rpc/ContainersList";
import type { ContainerStatus } from "@/types/rpc/ContainerStatus";
import { createRoot } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import { createContainerStatusStore, withContainer } from "./container-status";

const box = (name: string, state: ContainerStatus["state"] = "running", agent: string | null = name): ContainerStatus => ({
    name: `agentmux-${name}`,
    agent_id: agent,
    agent_name: agent && agent.toUpperCase(),
    state,
    image: "agentmux-agent:1",
    since_ms: 1,
});
const runtime = (available: boolean): ContainerRuntimeStatus => ({ available, at_ms: 1 });

function fake(list: ContainersList) {
    let onContainer: (s: ContainerStatus) => void = () => {};
    let onRuntime: (s: ContainerRuntimeStatus) => void = () => {};
    let tick: () => void = () => {};
    let stopped = 0;
    const read = vi.fn(async () => list);
    const deps = {
        read,
        onContainer: (h: (s: ContainerStatus) => void) => {
            onContainer = h;
            return () => void stopped++;
        },
        onRuntime: (h: (s: ContainerRuntimeStatus) => void) => {
            onRuntime = h;
            return () => void stopped++;
        },
        every: (_ms: number, t: () => void) => {
            tick = t;
            return () => void stopped++;
        },
    };
    return { deps, read, emit: (s: ContainerStatus) => onContainer(s), emitRuntime: (s: ContainerRuntimeStatus) => onRuntime(s), tick: () => tick(), stopped: () => stopped };
}

describe("withContainer", () => {
    it("replaces a container's row, adds a new one in name order, and drops one that is gone", () => {
        const list = [box("a"), box("c")];
        expect(withContainer(list, box("b")).map((c) => c.name)).toEqual(["agentmux-a", "agentmux-b", "agentmux-c"]);
        expect(withContainer(list, box("a", "stopped"))[0].state).toBe("stopped");
        expect(withContainer(list, box("a", "gone")).map((c) => c.name)).toEqual(["agentmux-c"]);
        expect(withContainer(list, box("zz", "gone"))).toHaveLength(2);
    });
});

describe("createContainerStatusStore", () => {
    it("reads the list once, applies each event, and keeps an orphan with no agent", async () => {
        const f = fake({ runtime: runtime(true), containers: [box("a"), box("o", "stopped", null)] });
        await createRoot(async (dispose) => {
            const store = createContainerStatusStore(f.deps);
            await store.load();
            expect(store.containers().map((c) => c.agent_id)).toEqual(["a", null]);
            expect(store.runtime().available).toBe(true);
            f.emit(box("a", "stopped"));
            f.emit(box("o", "gone", null));
            expect(store.containers().map((c) => [c.name, c.state])).toEqual([["agentmux-a", "stopped"]]);
            f.emitRuntime(runtime(false));
            expect(store.runtime().available).toBe(false);
            dispose();
        });
        expect(f.read).toHaveBeenCalledTimes(1);
    });

    it("reads again every minute, and an event that came while a read was out is not undone by it", async () => {
        const f = fake({ runtime: runtime(true), containers: [box("a")] });
        await createRoot(async (dispose) => {
            const store = createContainerStatusStore(f.deps);
            await store.load();
            // The next read answers with an older list than the event that arrives while it is out.
            let release: (l: ContainersList) => void = () => {};
            f.read.mockImplementationOnce(() => new Promise((r) => (release = r)));
            f.tick();
            f.emit(box("a", "stopped"));
            release({ runtime: runtime(true), containers: [box("a", "running")] });
            await Promise.resolve();
            await Promise.resolve();
            expect(store.containers()[0].state).toBe("stopped");
            dispose();
        });
    });

    it("keeps what it has when the read fails, and stops following when disposed", async () => {
        const f = fake({ runtime: runtime(true), containers: [box("a")] });
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        await createRoot(async (dispose) => {
            const store = createContainerStatusStore(f.deps);
            await store.load();
            f.read.mockRejectedValueOnce(new Error("down"));
            f.tick();
            await Promise.resolve();
            await Promise.resolve();
            expect(store.containers()).toHaveLength(1);
            dispose();
            store.dispose();
        });
        expect(f.stopped()).toBe(3);
        warn.mockRestore();
    });
});
