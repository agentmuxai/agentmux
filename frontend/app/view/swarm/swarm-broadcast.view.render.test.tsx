// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Broadcast input's wording for agents the filter hides, and that it takes no room while there is nothing to say.
 * swarm-broadcast.render.test.tsx has the sending and the box.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { BroadcastDock } from "./swarm-broadcast";
import type { AgentTreeNode, SwarmViewModel } from "./swarm-model";

// The filter's own state is covered in swarm-filter.render.test.tsx: here "hidden" is the one agent it hides.
vi.mock("./swarm-filter-state", () => ({
    actionableKeys: (_model: object, selected: Iterable<string>) => [...selected].filter((k) => k !== "hidden"),
}));

afterEach(() => cleanup());

const agents = { tree: () => ["a", "b", "hidden"].map((k) => ({ blockId: k, agentName: k.toUpperCase() })) as unknown as AgentTreeNode[] };

function modelStub(selectedKeys: string[]) {
    const [selected] = createSignal<Set<string>>(new Set(selectedKeys));
    return {
        selectedBlockIdsAtom: selected,
        otherInstancesAtom: () => null,
        fleetActionInFlightAtom: () => false,
        broadcastToSelection: vi.fn(async () => ({ succeeded: selectedKeys, failed: [], aborted_early: false })),
    } as unknown as SwarmViewModel;
}

const box = (c: HTMLElement) => c.querySelector<HTMLTextAreaElement>(".swarm-broadcast-input")!;

describe("BroadcastDock wording", () => {
    it("says how many selected agents the filter hides and leaves them out of the count", () => {
        const { container } = render(() => <BroadcastDock model={modelStub(["a", "hidden"])} agents={agents} />);
        expect(container.querySelector(".swarm-fleet-broadcast-gap")!.textContent).toBe("1 hidden by the filter, not included");
        expect(box(container).placeholder).toBe("Broadcast to 1 agent");
    });

    it("shows no note under the box when the filter hides nothing selected and nothing was sent", () => {
        const { container } = render(() => <BroadcastDock model={modelStub(["a", "b"])} agents={agents} />);
        expect(container.querySelector(".swarm-fleet-broadcast-gap")).toBeNull();
        expect(container.querySelector(".swarm-broadcast-note")).toBeNull();
    });
});
