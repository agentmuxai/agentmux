// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm's sections through the whole view. The agent list is rendered once: switching
 * section changes the area above it, never its elements, so a press on a row is not lost
 * because the pressed element was replaced before release (#5208).
 */
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: () => {} }));
vi.mock("@/app/store/pane-content-holds", () => ({
    trackPaneContent: () => () => {},
    holdPaneContent: () => () => {},
}));

// The Kanban section stands in for itself: the Swarm's contract with it is `SwarmKanbanSlotProps`.
const kanban = vi.hoisted(() => ({ mounts: 0, last: null as null | Record<string, any> }));
vi.mock("@/app/view/warden-kanban/kanban-section", () => ({
    KanbanSection: (props: Record<string, any>) => {
        kanban.mounts++;
        kanban.last = props;
        return <div data-testid="kanban" data-visible={String(props.visible)} data-open={String(props.openRequest)} data-lanes={String(props.lanes)} />;
    },
}));

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { NO_BACKGROUND_TASKS } from "./swarm-background";
import type { AgentTreeNode, SwarmViewModel } from "./swarm-model";
import type { SwarmOtherInstances } from "./swarm-remote";
import { SwarmView } from "./swarm-root";

afterEach(() => {
    cleanup();
    kanban.mounts = 0;
    kanban.last = null;
});

const others: SwarmOtherInstances = {
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
                    agents: [
                        { name: "Manoz", block_id: "" },
                        { name: "Lazo", block_id: "" },
                    ],
                },
            ],
        },
    ],
};

/** One agent of this instance, the kind of row whose press selects it (#5208). */
const localNode = {
    blockId: "b1",
    agentName: "AgentX",
    agentProvider: "claude",
    activitySummary: null,
    line: { text: "No activity yet", source: "status" },
    contextTokens: null,
    agentStatus: "running",
    agentToolRows: [],
    workflowRows: [],
    shellRows: [],
    cronRows: [],
    todoRows: [],
    todosTruncated: 0,
    todosPartial: false,
    currentTool: null,
    backgroundTasks: NO_BACKGROUND_TASKS,
} as unknown as AgentTreeNode;

function setup(initial: Record<string, unknown> = {}, local = false, onKanbanNeeds?: (n: number) => void) {
    const [meta, setMeta] = createSignal<Record<string, unknown>>(initial);
    const [selected, setSelected] = createSignal<Set<string>>(new Set());
    const model = {
        blockId: "swarm-1",
        loadingAtom: () => false,
        buildTree: () => (local ? [localNode] : []),
        isAgentCollapsed: () => true,
        toggleAgentCollapsed: () => {},
        otherInstancesAtom: () => others,
        pruneSelection: () => {},
        selectedBlockIdsAtom: selected,
        isSelected: (k: string) => selected().has(k),
        toggleSelected: (k: string) =>
            setSelected((prev) => {
                const next = new Set(prev);
                if (next.has(k)) next.delete(k);
                else next.add(k);
                return next;
            }),
        setManySelected: () => {},
        fleetActionInFlightAtom: () => false,
        lastFleetResultAtom: () => null,
        dismissFleetResult: () => {},
        clearSelection: () => setSelected(new Set<string>()),
        selectAll: (keys: string[]) => setSelected(new Set(keys)),
        fleetTargets: () => new Map(),
        broadcastToSelection: async () => ({ succeeded: [], failed: [], aborted_early: false }),
    } as unknown as SwarmViewModel;
    const written: Record<string, unknown>[] = [];
    const ctx = {
        meta,
        // Like the host: the patch lands in the pane's meta.
        setMeta: async (patch: Record<string, unknown>) => {
            written.push(patch);
            setMeta((prev) => ({ ...prev, ...patch }));
        },
        visibility: () => "active",
    } as unknown as PaneTabHostContext;
    const view = render(() => <SwarmView model={model} ctx={ctx} onKanbanNeeds={onKanbanNeeds} />);
    const tab = (name: string) =>
        [...view.container.querySelectorAll<HTMLElement>("[role=tab]")].find((t) => t.getAttribute("aria-label") === name)!;
    return { ...view, tab, written };
}

describe("the Swarm's sections", () => {
    it("lists Supervisor, Broadcast then Kanban, opening on the Supervisor", () => {
        const { tab, container } = setup();
        const labels = [...container.querySelectorAll("[role=tab]")].map((t) => t.getAttribute("aria-label"));
        expect(labels).toEqual(["Supervisor", "Broadcast", "Kanban"]);
        expect(tab("Supervisor").getAttribute("aria-selected")).toBe("true");
        expect(container.querySelector(".swarm-tree")!.getAttribute("data-density")).toBe("full");
    });

    it("opens on the section the pane's meta names, and on the default for one it does not have", () => {
        expect(setup({ "swarm:section": "broadcast" }).tab("Broadcast").getAttribute("aria-selected")).toBe("true");
        cleanup();
        expect(setup({ "swarm:section": "kanban" }).tab("Kanban").getAttribute("aria-selected")).toBe("true");
        cleanup();
        expect(setup({ "swarm:section": "nope" }).tab("Supervisor").getAttribute("aria-selected")).toBe("true");
    });

    it("writes the section to the pane's meta when a tab is pressed", () => {
        const { tab, written } = setup();
        fireEvent.click(tab("Broadcast"));
        expect(written).toEqual([{ "swarm:section": "broadcast" }]);
        expect(tab("Broadcast").getAttribute("aria-selected")).toBe("true");
    });

    it("never remounts the list or its rows on a switch, and the selection survives it", () => {
        const { tab, container } = setup();
        const list = container.querySelector(".swarm-tree")!;
        const rows = [...container.querySelectorAll(".swarm-remote-agent-group")];
        expect(rows).toHaveLength(2);
        const box = container.querySelector<HTMLInputElement>(".swarm-remote-agent input[type=checkbox]")!;
        fireEvent.click(box);
        expect(box.checked).toBe(true);

        fireEvent.click(tab("Broadcast"));
        expect(container.querySelector(".swarm-tree")).toBe(list);
        expect(list.getAttribute("data-density")).toBe("compact");
        expect([...container.querySelectorAll(".swarm-remote-agent-group")]).toEqual(rows);
        expect(container.querySelector<HTMLInputElement>(".swarm-remote-agent input[type=checkbox]")).toBe(box);
        expect(box.checked).toBe(true);

        fireEvent.click(tab("Supervisor"));
        expect(container.querySelector(".swarm-tree")).toBe(list);
        expect(list.getAttribute("data-density")).toBe("full");
        expect([...container.querySelectorAll(".swarm-remote-agent-group")]).toEqual(rows);
        expect(box.checked).toBe(true);
    });

    it("keeps the same card element for a local agent across a switch and back", () => {
        const { tab, container } = setup({}, true);
        const card = container.querySelector(".swarm-agent-card");
        expect(card).not.toBeNull();
        fireEvent.click(tab("Broadcast"));
        expect(container.querySelector(".swarm-agent-card")).toBe(card);
        fireEvent.click(tab("Supervisor"));
        expect(container.querySelector(".swarm-agent-card")).toBe(card);
    });

    describe("the Kanban section", () => {
        it("mounts the Kanban only the first time its section is shown, then keeps it mounted", () => {
            const { tab, queryByTestId } = setup();
            expect(queryByTestId("kanban")).toBeNull();
            expect(kanban.mounts).toBe(0);
            fireEvent.click(tab("Kanban"));
            expect(queryByTestId("kanban")!.dataset.visible).toBe("true");
            fireEvent.click(tab("Broadcast"));
            // Still mounted, told it is hidden so it pauses; not mounted a second time.
            expect(queryByTestId("kanban")!.dataset.visible).toBe("false");
            fireEvent.click(tab("Kanban"));
            expect(kanban.mounts).toBe(1);
            expect(queryByTestId("kanban")!.dataset.visible).toBe("true");
        });

        it("opens on the Kanban when the pane's meta says so, mounted at once", () => {
            const { queryByTestId, container } = setup({ "swarm:section": "kanban" });
            expect(queryByTestId("kanban")!.dataset.visible).toBe("true");
            expect(container.querySelector(".swarm-agents-pane")!.classList.contains("is-hidden")).toBe(true);
        });

        it("hides the agent list while the Kanban shows, without remounting it or its rows", () => {
            const { tab, container } = setup({}, true);
            const list = container.querySelector(".swarm-tree")!;
            const card = container.querySelector(".swarm-agent-card");
            const pane = container.querySelector(".swarm-agents-pane")!;
            expect(pane.classList.contains("is-hidden")).toBe(false);
            fireEvent.click(tab("Kanban"));
            expect(pane.classList.contains("is-hidden")).toBe(true);
            expect(container.querySelector(".swarm-kanban-slot")!.classList.contains("is-hidden")).toBe(false);
            fireEvent.click(tab("Supervisor"));
            expect(pane.classList.contains("is-hidden")).toBe(false);
            expect(container.querySelector(".swarm-kanban-slot")!.classList.contains("is-hidden")).toBe(true);
            expect(container.querySelector(".swarm-tree")).toBe(list);
            expect(container.querySelector(".swarm-agent-card")).toBe(card);
        });

        it("passes the task to open and the lanes from the pane's meta, and clears the request once handled", () => {
            const { queryByTestId, written } = setup({ "swarm:section": "kanban", "kanban:open": "t1", "kanban:lanes": true });
            expect(queryByTestId("kanban")!.dataset.open).toBe("t1");
            expect(queryByTestId("kanban")!.dataset.lanes).toBe("true");
            kanban.last!.onOpenHandled();
            expect(written).toContainEqual({ "kanban:open": null });
            expect(queryByTestId("kanban")!.dataset.open).toBe("null");
            kanban.last!.onLanes(false);
            expect(written).toContainEqual({ "kanban:lanes": false });
        });

        it("a card's agent switches to the Supervisor and selects that row", () => {
            const { tab, written } = setup({ "swarm:section": "kanban" }, true);
            kanban.last!.onOpenAgent("b1");
            expect(written).toContainEqual({ "swarm:section": "supervisor" });
            expect(tab("Supervisor").getAttribute("aria-selected")).toBe("true");
        });

        it("hands the Needs-you count up for the pane's title", () => {
            const counts: number[] = [];
            const view = setup({ "swarm:section": "kanban" }, false, (n) => counts.push(n));
            kanban.last!.onNeedsCount(2);
            expect(counts).toEqual([2]);
            expect(view.container).toBeTruthy();
        });
    });
});
