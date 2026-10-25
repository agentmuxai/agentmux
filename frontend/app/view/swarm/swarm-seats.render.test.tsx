// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Seats in the Swarm, through the whole view (SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §4):
 * the role badge after the name, the "Group by" control kept in the pane's meta, and the
 * grouped list, whose rows stay direct children of `.swarm-tree` in the order's sequence
 * within each group, so the slide (swarm-order-motion.ts) still finds and moves them.
 */
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { DockNodeStatusCommand: () => Promise.resolve() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: () => {} }));
vi.mock("@/app/store/pane-content-holds", () => ({
    trackPaneContent: () => () => {},
    holdPaneContent: () => () => {},
}));

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { MOS } from "@/app/store/global";
import type { MuxObjUpdate } from "@/types/rpc/MuxObjUpdate";
import type { SeatSummary } from "@/types/rpc/SeatSummary";
import { NO_BACKGROUND_TASKS } from "./swarm-background";
import type { AgentTreeNode, SwarmViewModel } from "./swarm-model";
import { SWARM_SETTLE_MS } from "./swarm-order";
import type { SwarmOtherInstances } from "./swarm-remote";
import { SwarmView } from "./swarm-view";

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

let version = 0;
function seedBlock(blockId: string, meta: Record<string, unknown>): void {
    MOS.updateMuxObject({
        updatetype: "update",
        otype: "block",
        oid: blockId,
        obj: { otype: "block", oid: blockId, version: ++version, meta } as unknown as Block,
    } as MuxObjUpdate);
}

const LEADER: SeatSummary = { role: "leader", qualifier: "product", lead: null, area: "agent pane, identity" };
const WORKER: SeatSummary = { role: "worker", qualifier: null, lead: { id: "uid-camper", name: "Camper" }, area: null };

/** Four agents, the newest active first: W1, Free, Camper, Opaz. */
function seedAgents(seats = true) {
    const now = Date.now();
    const agent = (id: string, name: string, ago: number, seat: SeatSummary | null) =>
        seedBlock(`b-${id}`, {
            view: "agent",
            agentId: `uid-${id}`,
            agentName: name,
            "session:last_activity_ms": now - ago,
            ...(seats && seat ? { "agent:seat": seat } : {}),
        });
    agent("w1", "W1", 1_000, WORKER);
    agent("free", "Free", 2_000, null);
    agent("camper", "Camper", 3_000, LEADER);
    agent("opaz", "Opaz", 4_000, { ...LEADER, qualifier: null });
}

function node(id: string, name: string): AgentTreeNode {
    return {
        blockId: `b-${id}`,
        agentName: name,
        agentProvider: "claude",
        activitySummary: null,
        line: { text: "No activity yet", source: "status" },
        contextTokens: null,
        agentStatus: "idle",
        agentToolRows: [],
        workflowRows: [],
        shellRows: [],
        cronRows: [],
        todoRows: [],
        todosTruncated: 0,
        todosPartial: false,
        currentTool: null,
        backgroundTasks: NO_BACKGROUND_TASKS,
    } as AgentTreeNode;
}

const OTHERS: SwarmOtherInstances = {
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
                    agents: [{ name: "Far", block_id: "", status: "idle" }],
                },
            ],
        },
    ],
};

function setup(groupBy: "lead" | null, others: SwarmOtherInstances = OTHERS) {
    const [meta, setMetaSignal] = createSignal<Record<string, unknown>>(groupBy ? { "swarm:groupby": groupBy } : {});
    const setMeta = vi.fn(async (patch: Record<string, unknown>) => {
        const next = { ...meta(), ...patch };
        for (const [k, v] of Object.entries(patch)) if (v == null) delete next[k];
        setMetaSignal(next);
    });
    const model = {
        blockId: "swarm-1",
        loadingAtom: () => false,
        buildTree: () => [node("w1", "W1"), node("free", "Free"), node("camper", "Camper"), node("opaz", "Opaz")],
        otherInstancesAtom: () => others,
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
        isAgentCollapsed: () => true,
        toggleAgentCollapsed: () => {},
        countdownStateAtom: () => new Map(),
    } as unknown as SwarmViewModel;
    const ctx = { meta, setMeta, visibility: () => "active" } as unknown as PaneTabHostContext;
    const view = render(() => <SwarmView model={model} ctx={ctx} />);
    const tree = () => view.container.querySelector(".swarm-tree")!;
    /** The list's direct children, in order: "# heading" or the row's key. */
    const children = () =>
        [...tree().children]
            .filter((el) => !el.classList.contains("swarm-seat-indent"))
            .map((el) =>
                el.classList.contains("swarm-seat-heading")
                    ? `# ${el.querySelector(".swarm-seat-heading__label")!.textContent}`
                    : (el as HTMLElement).dataset.swarmKey
            );
    const rowKeys = () =>
        [...tree().querySelectorAll<HTMLElement>(":scope > [data-swarm-key]")].map((el) => el.dataset.swarmKey);
    return { ...view, setMeta, tree, children, rowKeys };
}

const FAR = "lan:area54#1|stable|Far";

describe("Swarm seats", () => {
    it("shows the role badge after the name, with the whole seat on hover", () => {
        seedAgents();
        const { container } = setup(null);
        const row = container.querySelector<HTMLElement>('[data-swarm-key="b-camper"]')!;
        const badge = row.querySelector(".swarm-seat-badge")!;
        expect(badge.textContent).toBe("Leader · product");
        expect(badge.getAttribute("title")).toBe("Leader · product — reports to the operator — agent pane, identity");
        expect(badge.previousElementSibling?.classList.contains("swarm-agent-label")).toBe(true);
        expect(container.querySelector('[data-swarm-key="b-free"] .swarm-seat-badge')).toBeNull();
    });

    it("shows the seat another instance reports on its row, and groups the row by it", () => {
        seedAgents();
        const seated: SwarmOtherInstances = {
            ...OTHERS,
            hosts: [
                {
                    ...OTHERS.hosts[0],
                    channels: [
                        {
                            ...OTHERS.hosts[0].channels[0],
                            agents: [
                                { name: "Far", block_id: "", status: "idle", seat: { ...LEADER, area: "builds" } },
                                {
                                    name: "Near",
                                    block_id: "",
                                    seat: { role: "worker", qualifier: null, lead: { id: "u", name: "Far" }, area: null },
                                },
                            ],
                        },
                    ],
                },
            ],
        };
        const { container, children } = setup("lead", seated);
        const far = container.querySelector<HTMLElement>('[data-swarm-key*="Far"]')!;
        expect(far.querySelector(".swarm-seat-badge")!.textContent).toBe("Leader · product");
        expect(far.querySelector(".swarm-seat-badge")!.getAttribute("title")).toBe(
            "Leader · product — reports to the operator — builds"
        );
        expect(children()).not.toContain("# Other instances");
        expect(children().slice(0, 5)).toEqual(["# The operator", "b-camper", "b-w1", "b-opaz", far.dataset.swarmKey]);
    });

    it("lists the rows in one list when not grouped", () => {
        seedAgents();
        const { children } = setup(null);
        expect(children()).toEqual(["b-w1", "b-free", "b-camper", "b-opaz", expect.stringContaining("Far")]);
    });

    it("groups by lead with rows as direct children of the list, each group in the list's order", () => {
        seedAgents();
        const { children, rowKeys, container } = setup("lead");
        const far = rowKeys().at(-1)!;
        expect(far).toContain("Far");
        expect(children()).toEqual([
            "# The operator",
            "b-camper",
            "b-w1",
            "b-opaz",
            "# No seat",
            "b-free",
            "# Other instances",
            far,
        ]);
        // Every row is still found the way the slide finds them, and nothing else is.
        expect(rowKeys()).toEqual(["b-camper", "b-w1", "b-opaz", "b-free", far]);
        // The worker sits one step in, after the marker the stylesheet indents it by.
        const worker = container.querySelector('[data-swarm-key="b-w1"]')!;
        expect(worker.previousElementSibling?.classList.contains("swarm-seat-indent--d1")).toBe(true);
        expect(
            container
                .querySelector('[data-swarm-key="b-camper"]')!
                .previousElementSibling?.classList.contains("swarm-seat-heading")
        ).toBe(true);
    });

    it("follows the order within its group when an agent moves up, moving the same row element", () => {
        seedAgents();
        const { rowKeys, container } = setup("lead");
        const opaz = container.querySelector('[data-swarm-key="b-opaz"]');
        seedBlock("b-opaz", {
            view: "agent",
            agentId: "uid-opaz",
            agentName: "Opaz",
            "session:last_activity_ms": Date.now(),
            "agent:seat": { ...LEADER, qualifier: null },
        });
        vi.advanceTimersByTime(SWARM_SETTLE_MS);
        expect(rowKeys().slice(0, 4)).toEqual(["b-opaz", "b-camper", "b-w1", "b-free"]);
        expect(container.querySelector('[data-swarm-key="b-opaz"]')).toBe(opaz);
    });

    it("keeps the choice in the pane's meta, and clears it for None", async () => {
        seedAgents();
        const { getByRole, setMeta, children } = setup(null);
        fireEvent.click(getByRole("radio", { name: "Lead" }));
        expect(setMeta).toHaveBeenLastCalledWith({ "swarm:groupby": "lead" });
        await Promise.resolve();
        expect(children()[0]).toBe("# The operator");
        fireEvent.click(getByRole("radio", { name: "None" }));
        expect(setMeta).toHaveBeenLastCalledWith({ "swarm:groupby": null });
    });

    it("offers no control when no agent here has a seat, unless the list is grouped", () => {
        seedAgents(false);
        expect(setup(null).container.querySelector(".swarm-group-by")).toBeNull();
        cleanup();
        expect(setup("lead").container.querySelector(".swarm-group-by")).not.toBeNull();
    });
});
