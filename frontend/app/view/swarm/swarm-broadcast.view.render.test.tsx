// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Broadcast dock's wording and its clock: the placeholder, the note for agents the filter hides, and the
 * ages in the recent list that tick every 30 seconds. swarm-broadcast.render.test.tsx has the sending and the box.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BroadcastDock } from "./swarm-broadcast";
import type { AgentTreeNode, SwarmViewModel } from "./swarm-model";

// The filter's own state is covered in swarm-filter.render.test.tsx: here "hidden" is the one agent it hides.
vi.mock("./swarm-filter-state", () => ({
    actionableKeys: (_model: object, selected: Iterable<string>) => [...selected].filter((k) => k !== "hidden"),
}));

afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

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
const sendButton = (c: HTMLElement) => c.querySelector<HTMLButtonElement>(".swarm-fleet-btn--primary")!;
const meta = (c: HTMLElement) => c.querySelector(".swarm-broadcast-recent-meta")!.textContent;

describe("BroadcastDock wording", () => {
    it("asks for a selection while nothing is selected", () => {
        const { container } = render(() => <BroadcastDock model={modelStub([])} agents={agents} />);
        expect(box(container).placeholder).toBe("Select agents to message…");
        expect(sendButton(container).textContent).toContain("Send to 0");
    });

    it("names the recipient count in the placeholder, singular for one", () => {
        const one = render(() => <BroadcastDock model={modelStub(["a"])} agents={agents} />);
        expect(box(one.container).placeholder).toBe("Message to send to 1 agent…");
        one.unmount();
        const two = render(() => <BroadcastDock model={modelStub(["a", "b"])} agents={agents} />);
        expect(box(two.container).placeholder).toBe("Message to send to 2 agents…");
    });

    it("says how many selected agents the filter hides and leaves them out of the count", () => {
        const { container } = render(() => <BroadcastDock model={modelStub(["a", "hidden"])} agents={agents} />);
        expect(container.querySelector(".swarm-fleet-broadcast-gap")!.textContent).toBe("1 hidden by the filter, not included");
        expect(sendButton(container).textContent).toContain("Send to 1");
        expect(box(container).placeholder).toBe("Message to send to 1 agent…");
    });

    it("shows no hidden note when the filter hides nothing selected", () => {
        const { container } = render(() => <BroadcastDock model={modelStub(["a", "b"])} agents={agents} />);
        expect(container.querySelector(".swarm-fleet-broadcast-gap")).toBeNull();
    });
});

describe("BroadcastDock recent list ages", () => {
    beforeEach(() => vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "Date"] }));

    it("ticks from just now to minutes to hours, every 30 seconds", async () => {
        const { container } = render(() => <BroadcastDock model={modelStub(["a"])} agents={agents} />);
        fireEvent.input(box(container), { target: { value: "hi" } });
        fireEvent.click(sendButton(container));
        await Promise.resolve();
        await Promise.resolve();
        expect(meta(container)).toBe("just now · 1 sent · 1 delivered");
        vi.advanceTimersByTime(30_000);
        expect(meta(container)).toContain("just now");
        vi.advanceTimersByTime(30_000);
        expect(meta(container)).toContain("1 min ago");
        vi.advanceTimersByTime(59 * 60_000);
        expect(meta(container)).toContain("1 h ago");
        vi.advanceTimersByTime(2 * 3600_000);
        expect(meta(container)).toContain("3 h ago");
    });

    it("stops ticking when the dock goes away", () => {
        const { unmount } = render(() => <BroadcastDock model={modelStub(["a"])} agents={agents} />);
        expect(vi.getTimerCount()).toBe(1);
        unmount();
        expect(vi.getTimerCount()).toBe(0);
    });
});
