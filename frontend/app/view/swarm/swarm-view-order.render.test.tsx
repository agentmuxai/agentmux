// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm view's agent order, through the whole view: working agents first, a change
 * waiting for the settle time, and no move while the pointer is over the list.
 * docs/specs/SPEC_SWARM_ORDER_WORKING_FIRST_THEN_RECENT_2026_10_23.md.
 */
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: () => {} }));
vi.mock("@/app/store/pane-content-holds", () => ({
    trackPaneContent: () => () => {},
    holdPaneContent: () => () => {},
}));

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import type { SwarmViewModel } from "./swarm-model";
import { SWARM_SETTLE_MS } from "./swarm-order";
import type { RemoteAgentState, SwarmOtherInstances } from "./swarm-remote";
import { SwarmView } from "./swarm-view";

// jsdom has no :hover; the page says whether the pointer is over the list, as a browser would.
let hovered = false;
const realMatches = Element.prototype.matches;

beforeEach(() => {
    vi.useFakeTimers();
    hovered = false;
    vi.spyOn(Element.prototype, "matches").mockImplementation(function (this: Element, selector: string) {
        return selector === ":hover" ? hovered : realMatches.call(this, selector);
    });
});
afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    vi.useRealTimers();
});

function others(states: Record<string, RemoteAgentState>): SwarmOtherInstances {
    return {
        hostname: "narko",
        channel: "stable",
        hosts: [
            {
                host_id: "lan:area54#1",
                display_name: "Area54",
                tier: "lan",
                channels: [
                    {
                        channel: "stable",
                        seen_at_ms: Date.now(),
                        stale: false,
                        os: "macos",
                        agents: Object.entries(states).map(([name, status]) => ({ name, block_id: "", status })),
                    },
                ],
            },
        ],
    };
}

function setup(initial: Record<string, RemoteAgentState>) {
    const [data, setData] = createSignal(others(initial));
    const model = {
        blockId: "swarm-1",
        loadingAtom: () => false,
        buildTree: () => [],
        otherInstancesAtom: data,
        pruneSelection: () => {},
        selectedBlockIdsAtom: () => new Set<string>(),
        isSelected: () => false,
        toggleSelected: () => {},
        setManySelected: () => {},
        fleetActionInFlightAtom: () => false,
        lastFleetResultAtom: () => null,
        dismissFleetResult: () => {},
        clearSelection: () => {},
        selectAll: () => {},
    } as unknown as SwarmViewModel;
    const ctx = { meta: () => ({}), setMeta: async () => {} } as unknown as PaneTabHostContext;
    const view = render(() => <SwarmView model={model} ctx={ctx} />);
    const names = () => [...view.container.querySelectorAll(".swarm-remote-agent-name")].map((el) => el.textContent);
    return { ...view, names, change: (states: Record<string, RemoteAgentState>) => setData(others(states)) };
}

describe("Swarm order", () => {
    it("lists working agents above idle ones, each band in the order the agents came in", () => {
        // Other instances' agents come sorted by name; none of them shares a last-active time.
        const { names } = setup({ Idle1: "idle", Busy: "working", Idle2: "stopped", Asking: "waiting" });
        expect(names()).toEqual(["Asking", "Busy", "Idle1", "Idle2"]);
    });

    it("moves an agent that starts working to the top after the settle time", () => {
        const { names, change } = setup({ A: "idle", B: "idle", C: "idle" });
        expect(names()).toEqual(["A", "B", "C"]);
        change({ A: "idle", B: "idle", C: "working" });
        expect(names()).toEqual(["A", "B", "C"]);
        vi.advanceTimersByTime(SWARM_SETTLE_MS);
        expect(names()).toEqual(["C", "A", "B"]);
    });

    it("does not move the rows while the pointer is over the list, and moves them after it leaves", () => {
        const { names, change, container } = setup({ A: "idle", B: "idle", C: "idle" });
        const tree = container.querySelector(".swarm-tree")!;
        hovered = true;
        fireEvent.pointerEnter(tree);
        change({ A: "idle", B: "idle", C: "working" });
        vi.advanceTimersByTime(60_000);
        expect(names()).toEqual(["A", "B", "C"]);
        hovered = false;
        fireEvent.pointerLeave(tree);
        vi.advanceTimersByTime(SWARM_SETTLE_MS - 1);
        expect(names()).toEqual(["A", "B", "C"]);
        vi.advanceTimersByTime(1);
        expect(names()).toEqual(["C", "A", "B"]);
    });

    it("holds while focus is inside the list and while a button is down in it", () => {
        const { names, change, container } = setup({ A: "idle", B: "idle" });
        const tree = container.querySelector(".swarm-tree")!;
        const box = container.querySelector<HTMLInputElement>(".swarm-remote-agent input")!;
        box.focus();
        change({ A: "idle", B: "working" });
        vi.advanceTimersByTime(10_000);
        expect(names()).toEqual(["A", "B"]);
        box.blur();
        fireEvent.pointerDown(tree);
        vi.advanceTimersByTime(10_000);
        expect(names()).toEqual(["A", "B"]);
        fireEvent.pointerUp(window);
        vi.advanceTimersByTime(SWARM_SETTLE_MS);
        expect(names()).toEqual(["B", "A"]);
    });

    it("moves the same row element, so focus and a ticked checkbox follow the agent", () => {
        // Rows are keyed by agent, not by place: a move reorders the elements, it does not rebuild them.
        const { container, change } = setup({ A: "idle", B: "idle" });
        const rowOf = (name: string) =>
            [...container.querySelectorAll<HTMLElement>(".swarm-remote-agent-group")].find((el) =>
                el.textContent?.includes(name)
            )!;
        const before = rowOf("B");
        change({ A: "idle", B: "working" });
        vi.advanceTimersByTime(SWARM_SETTLE_MS);
        expect(rowOf("B")).toBe(before);
        expect(container.querySelector(".swarm-remote-agent-group")).toBe(before);
    });
});
