// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The top of the Swarm through the whole view: the Filter, the Broadcast input, the row with Select all and what acts
 * on the selection, then the agents, in one column with no sections. The agent list is rendered once: nothing above
 * it replaces its elements, so a press on a row is not lost because the pressed element was replaced before
 * release (#5208).
 */
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: () => {} }));
const settings = vi.hoisted(() => ({ openSettingsAt: vi.fn(async () => {}) }));
vi.mock("@/app/view/settings/open-settings", () => ({ openSettingsAt: settings.openSettingsAt }));
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
    return { ...view, written };
}

const before = (a: Element | null, b: Element | null) => !!a && !!b && !!(a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING);

describe("the top of the Swarm", () => {
    it("has the Filter first, then the Broadcast input, then the row with Select all, then the agents", () => {
        const { container } = setup({}, true);
        const parts = [".swarm-filter-bar", ".swarm-broadcast-dock", ".swarm-list-tools", ".swarm-tree"].map((s) => container.querySelector(s));
        expect(parts.every(Boolean)).toBe(true);
        for (let i = 0; i + 1 < parts.length; i++) expect(before(parts[i], parts[i + 1]), `${i}`).toBe(true);
        expect(parts[2]!.textContent).toContain("Select all");
    });

    it("has no section headers, no collapse buttons and no tabs, and no longer reads the old section meta", () => {
        const { container } = setup({ "swarm:open": ["broadcast", "supervisor"], "swarm:section": "supervisor" }, true);
        expect(container.querySelector(".swarm-section-block__head")).toBeNull();
        expect(container.querySelector("[aria-expanded]")).toBeNull();
        expect(container.querySelector("[role=tab]")).toBeNull();
        expect(container.querySelector(".swarm-supervisor-settings")).toBeNull();
        expect(container.querySelector(".swarm-broadcast-input")).not.toBeNull();
        expect(container.querySelector(".swarm-broadcast-dock")!.textContent).toBe("");
    });

    it("shows Stop and Mode on the selection's row only once an agent is selected", () => {
        const { container } = setup({}, true);
        expect(container.querySelector(".swarm-fleet-toolbar")).toBeNull();
        fireEvent.click(container.querySelector(".swarm-agent-select-checkbox")!);
        // Its count is beside Select all, and Stop and Mode are on the same row.
        expect(container.querySelector(".swarm-select-toggle__stats")!.textContent!.trim()).toBe("1 selected");
        const bar = container.querySelector(".swarm-fleet-toolbar")!;
        expect(container.querySelector(".swarm-list-tools")!.contains(bar)).toBe(true);
        expect(bar.textContent).toContain("Stop 1");
        expect(bar.textContent).toContain("Mode");
        expect(container.querySelectorAll(".swarm-fleet-toolbar")).toHaveLength(1);
        fireEvent.click(container.querySelector(".swarm-agent-select-checkbox")!);
        expect(container.querySelector(".swarm-fleet-toolbar")).toBeNull();
    });

    it("keeps no switch and no settings panel; while the Supervisor is off one line links to the Settings pane", () => {
        const { container } = setup({}, true);
        expect(container.querySelector("[role=switch]")).toBeNull();
        expect(container.querySelector(".swarm-fleet-supervisor-off")).toBeNull();
        fireEvent.click(container.querySelector(".swarm-agent-select-checkbox")!);
        const line = container.querySelector(".swarm-fleet-supervisor-off")!;
        expect(line.textContent).toBe("Supervisor is off. Turn it on in Settings.");
        fireEvent.click(line.querySelector("button")!);
        expect(settings.openSettingsAt).toHaveBeenCalledWith("supervisor");
    });

    it("never remounts the list or its rows as the selection changes, and the selection survives it", () => {
        const { container } = setup();
        const list = container.querySelector(".swarm-tree")!;
        const rows = [...container.querySelectorAll(".swarm-remote-agent-group")];
        expect(rows).toHaveLength(2);
        const box = container.querySelector<HTMLInputElement>(".swarm-remote-agent input[type=checkbox]")!;
        fireEvent.click(box);
        expect(box.checked).toBe(true);
        expect(container.querySelector(".swarm-tree")).toBe(list);
        expect([...container.querySelectorAll(".swarm-remote-agent-group")]).toEqual(rows);
        expect(container.querySelector<HTMLInputElement>(".swarm-remote-agent input[type=checkbox]")).toBe(box);
        expect(box.checked).toBe(true);
    });

    it("keeps the same card element for a local agent as it is selected and not", () => {
        const { container } = setup({}, true);
        const card = container.querySelector(".swarm-agent-card");
        expect(card).not.toBeNull();
        fireEvent.click(container.querySelector(".swarm-agent-select-checkbox")!);
        expect(container.querySelector(".swarm-agent-card")).toBe(card);
        fireEvent.click(container.querySelector(".swarm-agent-select-checkbox")!);
        expect(container.querySelector(".swarm-agent-card")).toBe(card);
    });

    it("holds the Filter, the Broadcast input, the selection's row and the list in one scroll container, and nothing inside it scrolls on its own", () => {
        const { container } = setup({}, true);
        const scroller = container.querySelector(".swarm-agents-pane")!;
        for (const part of [".swarm-filter-bar", ".swarm-broadcast-dock", ".swarm-list-tools", ".swarm-tree"]) {
            expect(scroller.contains(container.querySelector(part)), part).toBe(true);
        }
        // The stylesheet is the layout: only the pane scrolls, and nothing in it is capped or scrolls itself.
        const scss = readFileSync(resolve(__dirname, "swarm-view.scss"), "utf8");
        const rule = (selector: string) => {
            const at = scss.indexOf(`\n${selector} {`);
            return at < 0 ? "" : scss.slice(at, scss.indexOf("\n}", at));
        };
        expect(rule(".swarm-agents-pane")).toMatch(/overflow-y:\s*auto/);
        for (const selector of [".swarm-tree", ".swarm-broadcast-dock", ".swarm-fleet-result-rows"]) {
            expect(rule(selector), selector).not.toMatch(/overflow(-y)?:\s*(auto|scroll)|max-height:\s*(?!none)\S/);
        }
    });
});
