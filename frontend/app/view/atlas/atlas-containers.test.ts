// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** This machine's containers as the map reads them: each agent's own, the ones no agent uses, and the runtime's note. */

import { describe, expect, it } from "vitest";
import type { ContainerStatus } from "@/types/rpc/ContainerStatus";
import { containerRuntimeNote, containersByAgent, orphanContainers } from "./atlas-containers";
import type { AtlasAgent } from "./atlas-graph";

const container = (name: string, over: Partial<ContainerStatus> = {}): ContainerStatus => ({
    name,
    agent_id: null,
    agent_name: null,
    state: "running",
    image: "agentmux/agent:1",
    since_ms: 1_000,
    ...over,
});

const agent = (over: Partial<AtlasAgent> = {}): AtlasAgent => ({
    key: "local/Ann",
    name: "Ann",
    provider: null,
    state: "idle",
    line: null,
    machine: "local",
    agentId: null,
    seat: null,
    contextTokens: null,
    currentTool: null,
    blockId: null,
    runtimeId: null,
    remote: false,
    stale: false,
    ...over,
});

describe("containersByAgent", () => {
    it("gives each agent its container, and leaves out one that is gone or has no agent", () => {
        const map = containersByAgent([
            container("agentmux-a1", { agent_id: "a1", agent_name: "Ann", state: "starting" }),
            container("agentmux-a2", { agent_id: "a2", agent_name: "Bo", state: "gone" }),
            container("agentmux-x"),
        ]);
        expect([...map.keys()]).toEqual(["a1"]);
        expect(map.get("a1")).toEqual({ name: "agentmux-a1", state: "starting", image: "agentmux/agent:1", sinceMs: 1_000 });
    });
});

describe("orphanContainers", () => {
    it("are the containers no agent uses, by name, as nodes of the machine", () => {
        const orphans = orphanContainers(
            [container("agentmux-z"), container("agentmux-a1", { agent_id: "a1" }), container("agentmux-b", { state: "stopped" })],
            "local"
        );
        expect(orphans.map((o) => `${o.key} ${o.machine} ${o.state}`)).toEqual([
            "container:agentmux-b local stopped",
            "container:agentmux-z local running",
        ]);
    });

    it("leave out one that is gone", () => {
        expect(orphanContainers([container("agentmux-z", { state: "gone" })], "local")).toEqual([]);
    });
});

describe("containerRuntimeNote", () => {
    const down = { available: false, at_ms: 5_000 };

    it("says nothing while the runtime runs, or before its first report", () => {
        expect(containerRuntimeNote({ available: true, at_ms: 5_000 }, [agent({ runtime: "container" })], [])).toBeUndefined();
        expect(containerRuntimeNote({ available: false, at_ms: 0 }, [agent({ runtime: "container" })], [])).toBeUndefined();
    });

    it("says nothing on a machine that uses no containers", () => {
        expect(containerRuntimeNote(down, [agent({ runtime: "host" })], [])).toBeUndefined();
        expect(containerRuntimeNote(down, [agent({ runtime: "container", remote: true })], [])).toBeUndefined();
    });

    it("says the runtime is not running when an agent of this machine runs in a container, or one is listed", () => {
        expect(containerRuntimeNote(down, [agent({ runtime: "container" })], [])).toBe("container runtime not running");
        expect(containerRuntimeNote(down, [], [container("agentmux-x")])).toBe("container runtime not running");
    });
});
