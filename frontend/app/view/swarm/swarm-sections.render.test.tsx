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

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { NO_BACKGROUND_TASKS } from "./swarm-background";
import type { AgentTreeNode, SwarmViewModel } from "./swarm-model";
import type { SwarmOtherInstances } from "./swarm-remote";
import { SwarmView } from "./swarm-root";

afterEach(() => cleanup());

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

function setup(initial: Record<string, unknown> = {}, local = false) {
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
    const view = render(() => <SwarmView model={model} ctx={ctx} />);
    const tab = (name: string) =>
        [...view.container.querySelectorAll<HTMLElement>("[role=tab]")].find((t) => t.getAttribute("aria-label") === name)!;
    return { ...view, tab, written };
}

describe("the Swarm's sections", () => {
    it("lists Supervisor then Broadcast, opening on the Supervisor", () => {
        const { tab, container } = setup();
        const labels = [...container.querySelectorAll("[role=tab]")].map((t) => t.getAttribute("aria-label"));
        expect(labels).toEqual(["Supervisor", "Broadcast"]);
        expect(tab("Supervisor").getAttribute("aria-selected")).toBe("true");
        expect(container.querySelector(".swarm-tree")!.getAttribute("data-density")).toBe("full");
    });

    it("opens on the section the pane's meta names, and on the default for one it does not have", () => {
        expect(setup({ "swarm:section": "broadcast" }).tab("Broadcast").getAttribute("aria-selected")).toBe("true");
        cleanup();
        expect(setup({ "swarm:section": "kanban" }).tab("Supervisor").getAttribute("aria-selected")).toBe("true");
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
});
