// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Atlas pane through its component: regions and nodes, the card, the folded chip, the layout toggle, zoom, and
 * selecting agents on the map for the Swarm's own Broadcast and Stop.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";

const reveal = vi.hoisted(() => vi.fn(async () => true));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: reveal }));
vi.mock("@/app/store/agent-runtime-live", () => ({
    openAgentInActiveTab: vi.fn(async () => undefined),
    startAgent: vi.fn(async () => undefined),
    sharedAgentRuntime: vi.fn(),
}));
vi.mock("@/app/store/contextmenu", () => ({ ContextMenuModel: { showContextMenu: vi.fn() } }));
// The Swarm's own stop confirmation, as a title and its button: what it asks is the Swarm's to test.
vi.mock("@/app/element/confirm-modal", () => ({
    ConfirmModal: (props: { open: boolean; title: string; confirmLabel?: string; onConfirm: () => void }) => () =>
        props.open ? (
            <div data-testid="confirm">
                {props.title}
                <button onClick={() => props.onConfirm()}>confirm: {props.confirmLabel}</button>
            </div>
        ) : null,
}));

import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { buildAtlasGraph, type AtlasAgent, type AtlasMachine } from "./atlas-graph";
import type { AtlasFlow } from "./atlas-flow";
import { layoutAtlas } from "./atlas-layout";
import type { AtlasData } from "./atlas-model";
import { AtlasView } from "./atlas-view";

afterEach(cleanup);

// The test DOM has no pointer capture; a drag on the map takes it.
beforeAll(() => {
    HTMLElement.prototype.setPointerCapture ??= () => {};
});

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

/** The Swarm's model as far as the map's selection reaches it: the selection itself, and the fleet actions as spies. */
function modelStub() {
    const [selected, setSelected] = createSignal<Set<string>>(new Set());
    const [result] = createSignal(null);
    const change = (keys: string[], on: boolean) =>
        setSelected((prev) => {
            const next = new Set(prev);
            for (const key of keys) (on ? next.add(key) : next.delete(key));
            return next;
        });
    return {
        selectedBlockIdsAtom: selected,
        otherInstancesAtom: () => null,
        fleetActionInFlightAtom: () => false,
        lastFleetResultAtom: result,
        fleetTargets: () => new Map(),
        setManySelected: vi.fn(change),
        toggleSelected: vi.fn((key: string) => change([key], !selected().has(key))),
        selectAll: vi.fn((keys: string[]) => setSelected(new Set(keys))),
        clearSelection: vi.fn(() => setSelected(new Set<string>())),
        broadcastToSelection: vi.fn(async () => ({ succeeded: [...selected()], failed: [], aborted_early: false, pending: [] })),
        bulkStopSelection: vi.fn(async () => ({ succeeded: [], failed: [], aborted_early: false, pending: [] })),
        superviseSelection: vi.fn(),
        dismissFleetResult: vi.fn(),
    };
}

function rig(agents: AtlasAgent[], meta: Record<string, unknown> = {}, flows: AtlasFlow[] = []) {
    const machines = [machine("local", "desk"), machine("lab", "lab-1")];
    const model = modelStub();
    const selectable = agents.flatMap((a) => (a.remote ? [a.key] : a.blockId ? [a.blockId] : []));
    const [m, setM] = createSignal(meta);
    const setMeta = vi.fn(async (patch: Record<string, unknown>) => {
        setM((prev) => ({ ...prev, ...patch }));
    });
    const data: AtlasData = {
        graph: () => buildAtlasGraph(machines, agents),
        model: model as unknown as AtlasData["model"],
        swarm: {
            tree: () => agents.flatMap((a) => (a.blockId ? [{ blockId: a.blockId, agentName: a.name }] : [])),
            shownSelectableKeys: () => selectable,
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
        flows: () => flows,
    };
    const ctx = { meta: m, setMeta } as unknown as PaneTabHostContext;
    const utils = render(() => <AtlasView data={data} ctx={ctx} />);
    /** A drag over the map from one pane point to another (the test pane's map starts 16 px in, at life size). */
    const drag = (from: [number, number], to: [number, number], init: Record<string, unknown> = {}) => {
        const viewport = utils.getByTestId("atlas-viewport");
        fireEvent.pointerDown(viewport, { button: 0, buttons: 1, pointerId: 1, clientX: from[0], clientY: from[1], ...init });
        fireEvent.pointerMove(viewport, { buttons: 1, pointerId: 1, clientX: to[0], clientY: to[1], ...init });
        return () => fireEvent.pointerUp(viewport, { button: 0, pointerId: 1, clientX: to[0], clientY: to[1], ...init });
    };
    /** Where a node stands in the test pane. */
    const at = (key: string): [number, number] => {
        const p = layoutAtlas(buildAtlasGraph(machines, agents)).nodes.get(key)!;
        return [p.x + 16, p.y + 16];
    };
    return { ...utils, setMeta, model, drag, at };
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

describe("selecting agents on the map", () => {
    const fleet = () => [
        agent("Ann", "local", { blockId: "b1" }),
        agent("Bo", "local", { blockId: "b2" }),
        agent("Cy", "lab", { key: "remote:lab/Cy" }),
    ];
    const world = (c: HTMLElement) => c.querySelector<HTMLElement>(".atlas-world")!;
    const picked = (c: HTMLElement) => [...c.querySelectorAll(".atlas-node--picked")].map((n) => n.querySelector(".atlas-node__name")!.textContent);

    it("shows nothing of the fleet actions until an agent is selected", () => {
        const { queryByTestId } = rig(fleet());
        expect(queryByTestId("atlas-fleet")).toBeNull();
    });

    it("selects the agents a box dragged with Shift touches, by the Swarm's own keys, and does not move the map", () => {
        const { container, drag, at, model, getByTestId, queryByTestId } = rig(fleet());
        const before = world(container).style.transform;
        const [x, y] = at("local/Ann");
        const [x2, y2] = at("local/Bo");
        const release = drag([x - 4, y - 4], [x2 + 10, y2 + 10], { shiftKey: true });
        expect(getByTestId("atlas-lasso")).toBeTruthy();
        expect(picked(container)).toEqual(["Ann", "Bo"]);
        expect(model.setManySelected).not.toHaveBeenCalled();
        release();
        expect(model.setManySelected).toHaveBeenCalledWith(["b1", "b2"], true);
        expect(queryByTestId("atlas-lasso")).toBeNull();
        expect(world(container).style.transform).toBe(before);
        expect(getByTestId("atlas-fleet").textContent).toContain("2 selected");
        expect(getByTestId("atlas-fleet").textContent).toContain("Stop 2");
        expect(getByTestId("atlas-fleet").textContent).toContain("Send to 2");
    });

    it("adds to the selection with a second box, an agent of another machine included", () => {
        const { container, drag, at, model, getByTestId } = rig(fleet());
        model.setManySelected(["b1"], true);
        const [x, y] = at("remote:lab/Cy");
        drag([x - 4, y - 4], [x + 10, y + 10], { shiftKey: true })();
        expect(picked(container)).toEqual(["Ann", "Cy"]);
        expect(getByTestId("atlas-fleet").textContent).toContain("2 selected");
    });

    it("pans with a plain drag, and selects nothing", () => {
        const { container, drag, model } = rig(fleet());
        const before = world(container).style.transform;
        drag([300, 300], [340, 320])();
        expect(world(container).style.transform).not.toBe(before);
        expect(model.setManySelected).not.toHaveBeenCalled();
    });

    it("drops the box on Escape, selecting nothing", () => {
        const { drag, at, model, getByTestId, queryByTestId } = rig(fleet());
        const [x, y] = at("local/Ann");
        const release = drag([x - 4, y - 4], [x + 10, y + 10], { shiftKey: true });
        fireEvent.keyDown(getByTestId("atlas"), { key: "Escape" });
        expect(queryByTestId("atlas-lasso")).toBeNull();
        release();
        expect(model.setManySelected).not.toHaveBeenCalled();
    });

    it("toggles one agent with a Shift click and leaves its card closed; a plain click still opens the card", () => {
        const { container, getAllByTestId, queryByTestId, model } = rig(fleet());
        const ann = () => getAllByTestId("atlas-node")[0];
        fireEvent.click(ann(), { shiftKey: true });
        expect(model.toggleSelected).toHaveBeenCalledWith("b1");
        expect(picked(container)).toEqual(["Ann"]);
        expect(ann().getAttribute("aria-label")).toBe("Ann, Idle, selected");
        expect(queryByTestId("atlas-card")).toBeNull();
        fireEvent.click(ann(), { shiftKey: true });
        expect(picked(container)).toEqual([]);
        fireEvent.click(ann());
        expect(queryByTestId("atlas-card")).not.toBeNull();
    });

    it("with Select on, a click and a plain drag select, and Shift and drag moves the map", () => {
        const { container, getAllByTestId, getByLabelText, drag, at, model } = rig(fleet());
        fireEvent.click(getByLabelText("Select agents (or hold Shift and drag)"));
        expect(container.querySelector(".atlas-viewport--selecting")).not.toBeNull();
        expect(container.querySelector("[data-testid=atlas-fleet]")!.textContent).toContain("Drag a box around agents");
        fireEvent.click(getAllByTestId("atlas-node")[0]);
        expect(picked(container)).toEqual(["Ann"]);
        const [x, y] = at("local/Bo");
        drag([x - 4, y - 4], [x + 10, y + 10])();
        expect(picked(container)).toEqual(["Ann", "Bo"]);
        const before = world(container).style.transform;
        model.setManySelected.mockClear();
        drag([300, 300], [340, 320], { shiftKey: true })();
        expect(world(container).style.transform).not.toBe(before);
        expect(model.setManySelected).not.toHaveBeenCalled();
    });

    it("leaves out an agent the Swarm cannot select: one with no pane opens its card instead", () => {
        const { container, getAllByTestId, queryByTestId, drag, at, model } = rig([agent("Ann", "local"), agent("Bo", "local", { blockId: "b2" })]);
        const [x, y] = at("local/Ann");
        const [x2, y2] = at("local/Bo");
        drag([x - 4, y - 4], [x2 + 10, y2 + 10], { shiftKey: true })();
        expect(model.setManySelected).toHaveBeenCalledWith(["b2"], true);
        expect(picked(container)).toEqual(["Bo"]);
        fireEvent.click(getAllByTestId("atlas-node")[0], { shiftKey: true });
        expect(queryByTestId("atlas-card")).not.toBeNull();
    });

    it("Select all takes the nodes drawn, and Select none clears them", () => {
        const { container, getByText, getAllByTestId, model } = rig(fleet());
        fireEvent.click(getAllByTestId("atlas-node")[0], { shiftKey: true });
        fireEvent.click(getByText("Select all"));
        expect(model.selectAll).toHaveBeenCalledWith(["b1", "b2", "remote:lab/Cy"]);
        expect(picked(container)).toEqual(["Ann", "Bo", "Cy"]);
        fireEvent.click(getByText("Select none"));
        expect(picked(container)).toEqual([]);
    });

    it("broadcasts to the selection through the Swarm's own composer", async () => {
        const { container, getAllByTestId, model } = rig(fleet());
        fireEvent.click(getAllByTestId("atlas-node")[0], { shiftKey: true });
        fireEvent.input(container.querySelector(".swarm-broadcast-input")!, { target: { value: "ship it" } });
        fireEvent.click(container.querySelector(".swarm-fleet-btn--primary")!);
        await Promise.resolve();
        expect(model.broadcastToSelection).toHaveBeenCalledWith("ship it");
    });

    it("stops the selection only after the Swarm's own confirmation", async () => {
        const { getAllByTestId, getByText, getByTestId, queryByTestId, model } = rig(fleet());
        fireEvent.click(getAllByTestId("atlas-node")[0], { shiftKey: true });
        fireEvent.click(getAllByTestId("atlas-node")[1], { shiftKey: true });
        expect(queryByTestId("confirm")).toBeNull();
        fireEvent.click(getByText(/Stop 2/));
        expect(model.bulkStopSelection).not.toHaveBeenCalled();
        expect(getByTestId("confirm").textContent).toContain("Stop 2 agents?");
        fireEvent.click(getByText("confirm: Stop 2"));
        await Promise.resolve();
        expect(model.bulkStopSelection).toHaveBeenCalled();
    });
});

describe("messages and tasks on the map", () => {
    const fleet = () => [agent("Ann", "local"), agent("Bo", "local"), agent("Cy", "lab")];
    const flow = (over: Partial<AtlasFlow>): AtlasFlow => ({
        id: "f1",
        kind: "message",
        from: { agent: "ann", host: "desk" },
        to: { agent: "bo", host: "desk" },
        tone: "delivered",
        label: null,
        atMs: 0,
        leaving: false,
        ...over,
    });
    const marks = (c: HTMLElement) => [...c.querySelectorAll<SVGGElement>("[data-testid=atlas-flow]")];

    it("draws nothing with nothing happening", () => {
        const { container } = rig(fleet());
        expect(marks(container)).toHaveLength(0);
    });

    it("draws a message as a line from its sender to its receiver, in the colour of what became of it", () => {
        const { container } = rig(fleet(), {}, [flow({}), flow({ id: "f2", to: { agent: "cy", host: "lab-1" }, tone: "waiting" })]);
        const [first, second] = marks(container);
        expect(first.classList.contains("atlas-flow--delivered")).toBe(true);
        expect(first.querySelector(".atlas-flow__line")!.getAttribute("d")).toMatch(/^M[\d.]+,[\d.]+ Q/);
        expect(first.querySelector(".atlas-flow__end")).not.toBeNull();
        expect(first.querySelector(".atlas-flow__ping")).toBeNull();
        expect(second.classList.contains("atlas-flow--waiting")).toBe(true);
    });

    it("rings the end that is on the map when the other is not", () => {
        const { container } = rig(fleet(), {}, [flow({ from: { agent: "github-consumer" } })]);
        const [mark] = marks(container);
        expect(mark.querySelector(".atlas-flow__ping")).not.toBeNull();
        expect(mark.querySelector(".atlas-flow__line")).toBeNull();
    });

    it("draws a task handed over as a numbered line, and nothing for one with an end off the map", () => {
        const task = flow({ id: "t1", kind: "task", tone: "task", label: "#12" });
        const { container } = rig(fleet(), {}, [task, flow({ id: "t2", kind: "task", tone: "task", label: "#13", to: { agent: "nobody" } })]);
        const [mark, ...rest] = marks(container);
        expect(rest).toHaveLength(0);
        expect(mark.dataset.kind).toBe("task");
        expect(mark.querySelector(".atlas-flow__label")!.textContent).toBe("#12");
        expect(mark.querySelector(".atlas-flow__line")!.hasAttribute("pathLength")).toBe(false);
    });

    it("marks a line in its last second as leaving", () => {
        const { container } = rig(fleet(), {}, [flow({ leaving: true })]);
        expect(marks(container)[0].classList.contains("atlas-flow--leaving")).toBe(true);
    });
});
