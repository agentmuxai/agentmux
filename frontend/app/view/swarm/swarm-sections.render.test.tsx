// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm's two sections, Broadcast and Supervise, through the whole view: each a header that opens
 * and closes its body, both open at once. The agent list is rendered once: opening or closing a section
 * changes the area above it, never its elements, so a press on a row is not lost because the pressed
 * element was replaced before release (#5208).
 */
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: () => {} }));
vi.mock("@/app/store/pane-content-holds", () => ({
    trackPaneContent: () => () => {},
    holdPaneContent: () => () => {},
}));

import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { NO_BACKGROUND_TASKS } from "./swarm-background";
import type { AgentTreeNode, SwarmViewModel } from "./swarm-model";
import type { SwarmOtherInstances } from "./swarm-remote";
import { SwarmView } from "./swarm-root";

afterEach(() => {
    cleanup();
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
    const view = render(() => <SwarmView model={model} ctx={ctx} />);
    const head = (name: string) =>
        [...view.container.querySelectorAll<HTMLElement>(".swarm-section-block__head")].find((h) => h.textContent!.trim() === name)!;
    const body = (id: string) => view.container.querySelector(`#swarm-section-${id}`);
    return { ...view, head, body, written };
}

describe("the Swarm's sections", () => {
    it("lists Broadcast and Supervise as headers, both closed to start, with no tabs", () => {
        const { head, body, container } = setup();
        const labels = [...container.querySelectorAll(".swarm-section-block__head")].map((h) => h.textContent!.trim());
        expect(labels).toEqual(["Broadcast", "Supervise"]);
        expect(container.querySelector("[role=tab]")).toBeNull();
        expect(head("Broadcast").getAttribute("aria-expanded")).toBe("false");
        expect(head("Supervise").getAttribute("aria-expanded")).toBe("false");
        expect(body("broadcast")).toBeNull();
        expect(body("supervisor")).toBeNull();
        expect(container.querySelector(".swarm-tree")!.getAttribute("data-density")).toBe("full");
    });

    it("opens on the sections the pane's meta lists, and the one a link names", () => {
        expect(setup({ "swarm:open": ["broadcast"] }).head("Broadcast").getAttribute("aria-expanded")).toBe("true");
        cleanup();
        const both = setup({ "swarm:open": ["broadcast", "supervisor"] });
        expect(both.head("Broadcast").getAttribute("aria-expanded")).toBe("true");
        expect(both.head("Supervise").getAttribute("aria-expanded")).toBe("true");
        cleanup();
        // A link that names a section opens it (the settings page names the Supervisor).
        expect(setup({ "swarm:section": "supervisor" }).head("Supervise").getAttribute("aria-expanded")).toBe("true");
        cleanup();
        // A saved Swarm on its old Kanban section, or on a section it does not have, opens none.
        expect(setup({ "swarm:section": "kanban" }).head("Supervise").getAttribute("aria-expanded")).toBe("false");
        cleanup();
        expect(setup({ "swarm:section": "nope" }).head("Broadcast").getAttribute("aria-expanded")).toBe("false");
    });

    it("opens a section on a press, shows its body, and writes the open ones to the pane's meta", () => {
        const { head, body, written } = setup();
        fireEvent.click(head("Broadcast"));
        expect(written).toEqual([{ "swarm:open": ["broadcast"], "swarm:section": null }]);
        expect(head("Broadcast").getAttribute("aria-expanded")).toBe("true");
        expect(body("broadcast")!.querySelector(".swarm-broadcast-input")).not.toBeNull();
        expect(body("supervisor")).toBeNull();
    });

    it("keeps both open at once, and closes one without closing the other", () => {
        const { head, body, written } = setup();
        fireEvent.click(head("Broadcast"));
        fireEvent.click(head("Supervise"));
        expect(written.at(-1)).toEqual({ "swarm:open": ["broadcast", "supervisor"], "swarm:section": null });
        expect(body("broadcast")).not.toBeNull();
        expect(body("supervisor")).not.toBeNull();
        fireEvent.click(head("Broadcast"));
        expect(written.at(-1)).toEqual({ "swarm:open": ["supervisor"], "swarm:section": null });
        expect(body("broadcast")).toBeNull();
        expect(body("supervisor")).not.toBeNull();
    });

    it("has Stop and Mode in the Supervise section, and a selection brings no bar out above the list", () => {
        const { container, head, body } = setup({}, true);
        fireEvent.click(head("Supervise"));
        expect(body("supervisor")!.textContent).toContain("Stop 0");
        expect(body("supervisor")!.textContent).toContain("Mode");
        fireEvent.click(container.querySelector(".swarm-agent-select-checkbox")!);
        // Its count is beside Select all, and Stop is already there.
        expect(container.querySelector(".swarm-select-toggle__stats")!.textContent!.trim()).toBe("1 selected");
        expect(body("supervisor")!.textContent).toContain("Stop 1");
        expect(container.querySelectorAll(".swarm-fleet-toolbar")).toHaveLength(1);
        expect(container.querySelector(".swarm-compartment")!.contains(container.querySelector(".swarm-fleet-toolbar"))).toBe(true);
    });

    it("never remounts the list or its rows when a section opens or closes, and the selection survives it", () => {
        const { head, container } = setup();
        const list = container.querySelector(".swarm-tree")!;
        const rows = [...container.querySelectorAll(".swarm-remote-agent-group")];
        expect(rows).toHaveLength(2);
        const box = container.querySelector<HTMLInputElement>(".swarm-remote-agent input[type=checkbox]")!;
        fireEvent.click(box);
        expect(box.checked).toBe(true);

        fireEvent.click(head("Broadcast"));
        expect(container.querySelector(".swarm-tree")).toBe(list);
        expect(list.getAttribute("data-density")).toBe("compact");
        expect([...container.querySelectorAll(".swarm-remote-agent-group")]).toEqual(rows);
        expect(container.querySelector<HTMLInputElement>(".swarm-remote-agent input[type=checkbox]")).toBe(box);
        expect(box.checked).toBe(true);

        fireEvent.click(head("Supervise"));
        expect(container.querySelector(".swarm-tree")).toBe(list);
        expect(list.getAttribute("data-density")).toBe("full");
        expect([...container.querySelectorAll(".swarm-remote-agent-group")]).toEqual(rows);
        expect(box.checked).toBe(true);
    });

    it("keeps the same card element for a local agent as sections open and close", () => {
        const { head, container } = setup({}, true);
        const card = container.querySelector(".swarm-agent-card");
        expect(card).not.toBeNull();
        fireEvent.click(head("Broadcast"));
        expect(container.querySelector(".swarm-agent-card")).toBe(card);
        fireEvent.click(head("Broadcast"));
        expect(container.querySelector(".swarm-agent-card")).toBe(card);
    });

    it("holds the sections and the list in one scroll container, and nothing inside it scrolls on its own", () => {
        const { head, container } = setup({}, true);
        fireEvent.click(head("Broadcast"));
        fireEvent.click(head("Supervise"));
        const scroller = container.querySelector(".swarm-agents-pane")!;
        for (const part of [".swarm-compartment", ".swarm-supervisor-settings", ".swarm-broadcast-dock", ".swarm-tree"]) {
            expect(scroller.contains(container.querySelector(part)), part).toBe(true);
        }
        // The stylesheet is the layout: only the pane scrolls, and nothing in it is capped or scrolls itself.
        const scss = readFileSync(resolve(__dirname, "swarm-view.scss"), "utf8");
        const rule = (selector: string) => {
            const at = scss.indexOf(`\n${selector} {`);
            return at < 0 ? "" : scss.slice(at, scss.indexOf("\n}", at));
        };
        expect(rule(".swarm-agents-pane")).toMatch(/overflow-y:\s*auto/);
        for (const selector of [".swarm-tree", ".swarm-broadcast-dock", ".swarm-fleet-result-rows", ".swarm-broadcast-recent"]) {
            expect(rule(selector), selector).not.toMatch(/overflow(-y)?:\s*(auto|scroll)|max-height:\s*(?!none)\S/);
        }
        expect(rule(".swarm-compartment")).not.toMatch(/max-height:\s*(?!none)\S|overflow-y:\s*auto/);
    });
});
