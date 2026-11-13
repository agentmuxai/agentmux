// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Atlas pane through its component: regions and nodes, the card, the folded chip, the layout toggle, zoom. */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const reveal = vi.hoisted(() => vi.fn(async () => true));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: reveal }));
vi.mock("@/app/store/agent-runtime-live", () => ({
    openAgentInActiveTab: vi.fn(async () => undefined),
    startAgent: vi.fn(async () => undefined),
    sharedAgentRuntime: vi.fn(),
}));
vi.mock("@/app/store/contextmenu", () => ({ ContextMenuModel: { showContextMenu: vi.fn() } }));

import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { buildAtlasGraph, type AtlasAgent, type AtlasMachine } from "./atlas-graph";
import type { AtlasData } from "./atlas-model";
import { AtlasView } from "./atlas-view";

afterEach(cleanup);

const machine = (key: string, title: string): AtlasMachine => ({
    key,
    title,
    badge: key === "local" ? "this machine" : "LAN",
    platform: null,
    stale: false,
    seenAtMs: null,
});

function agent(name: string, m: string, over: Partial<AtlasAgent> = {}): AtlasAgent {
    return {
        key: `${m}/${name}`,
        name,
        provider: "claude",
        state: "idle",
        line: null,
        machine: m,
        agentId: `id-${name}`,
        seat: null,
        contextTokens: null,
        currentTool: null,
        blockId: null,
        runtimeId: null,
        remote: m !== "local",
        stale: false,
        ...over,
    };
}

function rig(agents: AtlasAgent[], meta: Record<string, unknown> = {}) {
    const machines = [machine("local", "desk"), machine("lab", "lab-1")];
    const [m, setM] = createSignal(meta);
    const setMeta = vi.fn(async (patch: Record<string, unknown>) => {
        setM((prev) => ({ ...prev, ...patch }));
    });
    const data: AtlasData = {
        graph: () => buildAtlasGraph(machines, agents),
        swarm: {
            filter: {
                text: () => "",
                setText: () => {},
                clear: () => {},
                query: () => ({ words: [], scope: "both" }),
                shown: () => null,
                stopped: () => [],
                counts: () => ({ shown: agents.length, total: agents.length }),
            },
        } as unknown as AtlasData["swarm"],
        agents: () => new Map(agents.map((a) => [a.key, a] as const)),
        machines: () => new Map(machines.map((x) => [x.key, x] as const)),
    };
    const ctx = { meta: m, setMeta } as unknown as PaneTabHostContext;
    const utils = render(() => <AtlasView data={data} ctx={ctx} />);
    return { ...utils, setMeta };
}

describe("AtlasView", () => {
    it("says so when there is nothing to show", () => {
        const { getByText } = rig([]);
        expect(getByText("No agents to show yet.")).toBeTruthy();
    });

    it("draws a region per machine and a node per running agent, with the state in its name", () => {
        const { getAllByTestId } = rig([
            agent("Ann", "local", { state: "working" }),
            agent("Bo", "local"),
            agent("Cy", "lab"),
        ]);
        expect(getAllByTestId("atlas-region")).toHaveLength(2);
        const nodes = getAllByTestId("atlas-node");
        expect(nodes.map((n) => n.getAttribute("aria-label"))).toEqual(["Ann, Working", "Bo, Idle", "Cy, Idle"]);
        expect(nodes[0].className).toContain("atlas-node--working");
    });

    it("folds the stopped agents into a chip that lists them", () => {
        const { getByText, queryByText } = rig([
            agent("Ann", "local"),
            agent("Old", "local", { state: "stopped", runtimeId: "r1" }),
        ]);
        expect(queryByText("Old")).toBeNull();
        fireEvent.click(getByText(/1 stopped/));
        expect(getByText("Old")).toBeTruthy();
        expect(getByText("Start")).toBeTruthy();
    });

    it("opens a card for a node, and Open focuses its pane; Escape closes the card", () => {
        const { getAllByTestId, getByTestId, queryByTestId, getByText } = rig([
            agent("Ann", "local", { state: "working", line: "Fixing the cursor", blockId: "b1", currentTool: "Edit" }),
        ]);
        expect(queryByTestId("atlas-card")).toBeNull();
        fireEvent.click(getAllByTestId("atlas-node")[0]);
        const card = getByTestId("atlas-card");
        expect(card.textContent).toContain("Fixing the cursor");
        expect(card.textContent).toContain("Edit");
        fireEvent.click(getByText("Open"));
        expect(reveal).toHaveBeenCalledWith("b1", { source: "atlas" });
        fireEvent.keyDown(getByTestId("atlas"), { key: "Escape" });
        expect(queryByTestId("atlas-card")).toBeNull();
    });

    it("gives an agent of another machine a card with no Open", () => {
        const { getAllByTestId, queryByText } = rig([agent("Cy", "lab", { state: "waiting" })]);
        fireEvent.click(getAllByTestId("atlas-node")[0]);
        expect(queryByText("Open")).toBeNull();
    });

    it("keeps the layout in the pane's meta, and the By seat layout has no regions", async () => {
        const { getByText, queryAllByTestId, setMeta } = rig([agent("Ann", "local"), agent("Cy", "lab")]);
        expect(queryAllByTestId("atlas-region")).toHaveLength(2);
        fireEvent.click(getByText("By seat"));
        expect(setMeta).toHaveBeenCalledWith({ "atlas:layout": "seat" });
        await Promise.resolve();
        expect(queryAllByTestId("atlas-region")).toHaveLength(0);
        expect(queryAllByTestId("atlas-node")).toHaveLength(2);
    });

    it("zooms with the wheel", () => {
        const { getByTestId } = rig([agent("Ann", "local")]);
        const world = () => getByTestId("atlas-viewport").querySelector<HTMLElement>(".atlas-world")!;
        const before = world().style.transform;
        fireEvent.wheel(getByTestId("atlas-viewport"), { deltaY: -200, clientX: 10, clientY: 10 });
        expect(world().style.transform).not.toBe(before);
    });
});
