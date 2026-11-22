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
// What an open card reads from srv and the Kanban (atlas-work-store.test.ts): here, which pane it is asked about.
const workFor = vi.hoisted(() => ({ asked: [] as (string | null)[] }));
vi.mock("./atlas-work-store", () => ({
    useAtlasWork: (blockId: () => string | null) => () => {
        workFor.asked.push(blockId());
        return blockId() ? { repo: "agentmuxai/agentmux", branch: `branch-of-${blockId()}`, tasks: [] } : null;
    },
}));
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
import { buildAtlasGraph, type AtlasAgent, type AtlasExtras, type AtlasMachine } from "./atlas-graph";
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
        broadcastToSelection: vi.fn(async () => ({ succeeded: [...selected()], failed: [], aborted_early: false, pending: [], held: [] })),
        bulkStopSelection: vi.fn(async () => ({ succeeded: [], failed: [], aborted_early: false, pending: [], held: [] })),
        superviseSelection: vi.fn(),
        dismissFleetResult: vi.fn(),
    };
}

/** What a test puts on the map besides agents, and what it says of this machine and of the cloud. */
interface RigExtras extends AtlasExtras {
    localNote?: string;
    cloudUnavailable?: boolean;
}

function rig(agents: AtlasAgent[], meta: Record<string, unknown> = {}, flows: AtlasFlow[] = [], extras: RigExtras = {}) {
    const machines = [
        { ...machine("local", "desk"), note: extras.localNote },
        machine("lab", "lab-1"),
        { ...machine("far", "far-1"), badge: "cloud", cloud: true },
    ];
    const model = modelStub();
    // The agents as the Swarm reports them now: a test replaces the list, as a live update does.
    const [live, setLive] = createSignal(agents);
    const selectable = agents.flatMap((a) => (a.remote ? [a.key] : a.blockId ? [a.blockId] : []));
    const [m, setM] = createSignal(meta);
    const setMeta = vi.fn(async (patch: Record<string, unknown>) => {
        setM((prev) => ({ ...prev, ...patch }));
    });
    const data: AtlasData = {
        graph: () => buildAtlasGraph(machines, live(), extras),
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
        agents: () => new Map(live().map((a) => [a.key, a] as const)),
        machines: () => new Map(machines.map((x) => [x.key, x] as const)),
        flows: () => flows,
        cloudUnavailable: () => !!extras.cloudUnavailable,
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
        const p = layoutAtlas(buildAtlasGraph(machines, agents, extras)).nodes.get(key)!;
        return [p.x + 16, p.y + 16];
    };
    return { ...utils, setMeta, model, drag, at, setAgents: setLive };
}

describe("AtlasView", () => {
    it("says so when there is nothing to show", () => {
        const { getByText } = rig([]);
        expect(getByText("No agents to show yet.")).toBeTruthy();
    });

    it("draws no box per host, and a node per running agent with its state and its host in its name", () => {
        const { getAllByTestId, queryAllByTestId, container } = rig([
            agent("Ann", "local", { state: "working" }),
            agent("Bo", "local"),
            agent("Cy", "lab"),
        ]);
        expect(queryAllByTestId("atlas-region")).toHaveLength(0);
        expect(container.querySelector(".atlas-region")).toBeNull();
        const nodes = getAllByTestId("atlas-node");
        expect(nodes.map((n) => n.getAttribute("aria-label"))).toEqual(["Ann, Working, on desk", "Bo, Idle, on desk", "Cy, Idle, on lab-1"]);
        expect(nodes[0].className).toContain("atlas-node--working");
        // The host in words on the node itself, so the colour is never the only way to tell.
        expect(nodes.map((n) => n.querySelector(".atlas-node__host")?.textContent)).toEqual(["desk", "desk", "lab-1"]);
    });

    it("rings the nodes of one host alike and the nodes of two hosts apart, and sets each host's ring beside its name", () => {
        const { getAllByTestId, getByTestId } = rig([agent("Ann", "local"), agent("Bo", "local"), agent("Cy", "lab")]);
        const [ann, bo, cy] = getAllByTestId("atlas-node");
        const ring = (n: HTMLElement) => `${n.getAttribute("data-host-colour")}:${n.getAttribute("data-host-line")}`;
        expect(ann.getAttribute("data-host-colour")).not.toBeNull();
        expect(ring(ann)).toBe(ring(bo));
        expect(ring(ann)).not.toBe(ring(cy));
        const legend = getByTestId("atlas-legend");
        expect(legend.textContent).toContain("desk");
        expect(legend.textContent).toContain("lab-1");
        // The legend's ring is the node's ring.
        const sample = [...legend.querySelectorAll<HTMLElement>(".atlas-legend__ring")].map(ring);
        expect(sample.sort()).toEqual([ring(ann), ring(cy)].sort());
    });

    it("draws no ring and no legend with one host, and still says the host on each node", () => {
        const { getAllByTestId, queryByTestId } = rig([agent("Ann", "local"), agent("Bo", "local")]);
        const [ann] = getAllByTestId("atlas-node");
        expect(ann.hasAttribute("data-host-colour")).toBe(false);
        expect(queryByTestId("atlas-legend")).toBeNull();
        expect(ann.querySelector(".atlas-node__host")?.textContent).toBe("desk");
    });

    it("gives a host the same ring however the agents are ordered", () => {
        const ringOf = (agents: AtlasAgent[], name: string) => {
            const { getAllByTestId, unmount } = rig(agents);
            const node = getAllByTestId("atlas-node").find((n) => n.getAttribute("aria-label")!.startsWith(name))!;
            const out = `${node.getAttribute("data-host-colour")}:${node.getAttribute("data-host-line")}`;
            unmount();
            return out;
        };
        const ann = agent("Ann", "local");
        const cy = agent("Cy", "lab");
        const zo = agent("Zo", "far");
        expect(ringOf([ann, cy, zo], "Cy")).toBe(ringOf([zo, cy, ann], "Cy"));
        expect(ringOf([ann, cy, zo], "Ann")).toBe(ringOf([zo, cy, ann], "Ann"));
    });

    it("folds the stopped agents into a chip that lists them", () => {
        const { getByText, queryByText, getByTestId } = rig([
            agent("Ann", "local"),
            agent("Old", "local", { state: "stopped", runtimeId: "r1" }),
        ]);
        expect(queryByText("Old")).toBeNull();
        fireEvent.click(getByText(/1 stopped/));
        expect(getByText("Old")).toBeTruthy();
        expect(getByText("Start")).toBeTruthy();
        // With the host each is on.
        expect(getByTestId("atlas-stopped").textContent).toContain("desk");
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
        // The host, and how its machine was found.
        expect(card.textContent).toContain("Hostdesk · this machine");
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

    it("keeps the layout in the pane's meta, and both layouts draw the same nodes with the same rings", async () => {
        const { getByText, getAllByTestId, queryAllByTestId, setMeta } = rig([agent("Ann", "local"), agent("Cy", "lab")]);
        const rings = () => getAllByTestId("atlas-node").map((n) => n.getAttribute("data-host-colour") + n.getAttribute("data-host-line"));
        const before = rings();
        expect(queryAllByTestId("atlas-region")).toHaveLength(0);
        fireEvent.click(getByText("By seat"));
        expect(setMeta).toHaveBeenCalledWith({ "atlas:layout": "seat" });
        await Promise.resolve();
        expect(queryAllByTestId("atlas-node")).toHaveLength(2);
        expect(rings()).toEqual(before);
    });

    it("zooms with the wheel", () => {
        const { getByTestId } = rig([agent("Ann", "local")]);
        const world = () => getByTestId("atlas-viewport").querySelector<HTMLElement>(".atlas-world")!;
        const before = world().style.transform;
        fireEvent.wheel(getByTestId("atlas-viewport"), { deltaY: -200, clientX: 10, clientY: 10 });
        expect(world().style.transform).not.toBe(before);
    });

    // The Swarm's data is rebuilt on every update, so each agent is a new object with the same key. A node that goes and
    // comes back under a resting pointer loses its hover label's anchor and the cursor: the label shows for a moment and
    // goes while the pointer has not moved.
    it("keeps a node's element, and its hover label, when an update rebuilds every agent", () => {
        const { getAllByTestId, setAgents } = rig([agent("Ann", "local", { line: "Fixing the map" }), agent("Bo", "local")]);
        const [ann, bo] = getAllByTestId("atlas-node");
        expect(ann.getAttribute("data-hover-tip")).toBe("Fixing the map, on desk");
        setAgents([agent("Ann", "local", { line: "Testing the map" }), agent("Bo", "local")]);
        const after = getAllByTestId("atlas-node");
        expect(after).toHaveLength(2);
        expect(after[0]).toBe(ann);
        expect(after[1]).toBe(bo);
        expect(ann.isConnected).toBe(true);
        expect(ann.getAttribute("data-hover-tip")).toBe("Testing the map, on desk");
    });

    it("draws a new node for an agent that came, drops the one that went, and keeps the rest", () => {
        const { getAllByTestId, setAgents } = rig([agent("Ann", "local"), agent("Bo", "local")]);
        const [ann, bo] = getAllByTestId("atlas-node");
        setAgents([agent("Bo", "local", { state: "working" }), agent("Cy", "local")]);
        const after = getAllByTestId("atlas-node");
        expect(after.map((n) => n.getAttribute("aria-label"))).toEqual(["Bo, Working, on desk", "Cy, Idle, on desk"]);
        expect(after[0]).toBe(bo);
        expect(after[0].className).toContain("atlas-node--working");
        expect(ann.isConnected).toBe(false);
    });

    it("keeps the selected node selected, and its element, through an update", () => {
        const { getAllByTestId, getByTestId, setAgents } = rig([agent("Ann", "local", { blockId: "b1" })]);
        const ann = getAllByTestId("atlas-node")[0];
        fireEvent.click(ann);
        expect(getByTestId("atlas-card").textContent).toContain("Ann");
        setAgents([agent("Ann", "local", { blockId: "b1", line: "New line" })]);
        expect(getAllByTestId("atlas-node")[0]).toBe(ann);
        expect(ann.getAttribute("aria-pressed")).toBe("true");
        expect(getByTestId("atlas-card").textContent).toContain("New line");
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
        expect(ann().getAttribute("aria-label")).toBe("Ann, Idle, on desk, selected");
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
        // The second argument is the id the composer follows the send's progress by.
        expect(model.broadcastToSelection).toHaveBeenCalledWith("ship it", expect.any(String));
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

describe("the card's facts", () => {
    it("shows the branch of one of this instance's agents, asked by its pane", () => {
        const { getByText, getByTestId } = rig([agent("Ann", "local", { blockId: "b-ann", runtime: "host" })]);
        fireEvent.click(getByText("Ann"));
        expect(getByTestId("atlas-card-branch").textContent).toBe("agentmuxai/agentmux · branch-of-b-ann");
        expect(getByTestId("atlas-card-runtime")).toBeTruthy();
    });

    it("asks nothing about an agent of another instance, and shows its account", () => {
        const account = { id: "claude:0123456789abcdef", label: "Work" };
        const { getByText, getByTestId, queryByTestId } = rig([
            agent("Cy", "lab", { blockId: "b-cy", account }),
            agent("Dee", "lab", { account: { ...account } }),
        ]);
        workFor.asked = [];
        fireEvent.click(getByText("Cy"));
        expect(workFor.asked.every((id) => id === null)).toBe(true);
        expect(queryByTestId("atlas-card-branch")).toBeNull();
        expect(getByTestId("atlas-card-account").textContent).toContain("Work");
        expect(getByTestId("atlas-card-account").textContent).toContain("Also on it: Dee");
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
        // On a chip in the line's own colour, behind the text, so a number on a crossing reads as that line's.
        const chip = mark.querySelector(".atlas-flow__chip")!;
        expect(chip.compareDocumentPosition(mark.querySelector(".atlas-flow__label")!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
        expect(mark.querySelector(".atlas-flow__line")!.hasAttribute("pathLength")).toBe(false);
    });

    it("runs the lines behind the nodes, so a line that passes a node does not cross its name", () => {
        const { container } = rig(fleet(), {}, [flow({})]);
        const layer = container.querySelector(".atlas-flows:not(.atlas-flows--over)")!;
        const node = container.querySelector(".atlas-node")!;
        expect(layer.compareDocumentPosition(node) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    });

    it("draws a faint copy of each line over the nodes, the same path with no number or end, so a line behind a card can be followed", () => {
        const task = flow({ id: "t1", kind: "task", tone: "task", label: "#12" });
        const { container } = rig(fleet(), {}, [flow({}), task, flow({ id: "r1", from: { agent: "github-consumer" } })]);
        const over = container.querySelector(".atlas-flows--over")!;
        const paths = [...over.querySelectorAll("[data-testid=atlas-flow-over] .atlas-flow__line")].map((p) => p.getAttribute("d"));
        const behind = [...container.querySelectorAll("[data-testid=atlas-flow] .atlas-flow__line")].map((p) => p.getAttribute("d"));
        // A message and a task have a curve; the ring of a message with one end on the map has none.
        expect(paths).toHaveLength(2);
        expect(paths).toEqual(behind);
        expect(over.querySelector(".atlas-flow__label, .atlas-flow__end, .atlas-flow__ping")).toBeNull();
    });

    it("marks the faint copy of a line in its last second as leaving, with the line", () => {
        const { container } = rig(fleet(), {}, [flow({ leaving: true })]);
        expect(container.querySelector("[data-testid=atlas-flow-over]")!.classList.contains("atlas-flow--leaving")).toBe(true);
    });

    it("marks a line in its last second as leaving", () => {
        const { container } = rig(fleet(), {}, [flow({ leaving: true })]);
        expect(marks(container)[0].classList.contains("atlas-flow--leaving")).toBe(true);
    });
});

describe("what is not an agent", () => {
    const fleet = () => [agent("Ann", "local", { blockId: "b1" }), agent("Bo", "local"), agent("Cy", "lab")];

    it("frames a container agent with its container, in the container's state, and names it on the card", () => {
        const container = { name: "agentmux-a1", state: "starting" as const, image: "agentmux/agent:1", sinceMs: 0 };
        const { getAllByTestId, getByTestId } = rig([agent("Ann", "local", { runtime: "container", container }), agent("Bo", "local", { runtime: "host" })]);
        const [ann, bo] = getAllByTestId("atlas-node");
        expect(ann.classList.contains("atlas-node--container")).toBe(true);
        expect(ann.dataset.container).toBe("starting");
        expect(ann.getAttribute("aria-label")).toBe("Ann, Idle, on desk, in a container");
        expect(bo.classList.contains("atlas-node--container")).toBe(false);
        fireEvent.click(ann);
        expect(getByTestId("atlas-card-container").textContent).toContain("agentmux-a1");
    });

    it("draws a container no agent uses as a node of its machine, with a card and no part in a selection", () => {
        const orphans = [{ key: "container:agentmux-x", machine: "local", name: "agentmux-x", state: "stopped" as const, image: "i:1", sinceMs: 0 }];
        const { getByTestId, getAllByTestId, model, drag } = rig(fleet(), {}, [], { orphans });
        const node = getByTestId("atlas-orphan");
        expect(node.textContent).toContain("agentmux-x");
        expect(node.textContent).toContain("No agent · stopped");
        // A container is a node of its host too: its name says where it is, and it has its host's ring.
        expect(node.getAttribute("aria-label")).toContain(", on desk");
        expect(node.querySelector(".atlas-node__host")?.textContent).toBe("desk");
        expect(getAllByTestId("atlas-node")).toHaveLength(3);
        fireEvent.click(node);
        expect(getByTestId("atlas-card").textContent).toContain("no agent of this machine uses it");
        // A box over the whole map takes the agents and leaves the container.
        drag([0, 0], [2000, 2000], { shiftKey: true })();
        expect(model.setManySelected.mock.calls.at(-1)![0]).toEqual(["b1", "lab/Cy"]);
    });

    it("draws no pull request node, region or line: the map is agents, their containers and what passes between them", () => {
        const { queryByTestId, container, getAllByTestId } = rig(fleet());
        expect(queryByTestId("atlas-services")).toBeNull();
        expect(queryByTestId("atlas-service")).toBeNull();
        expect(getAllByTestId("atlas-node")).toHaveLength(3);
        expect(container.querySelector('[data-kind="review"], [data-kind="checks"]')).toBeNull();
    });

    it("marks one of the account's other machines in the legend, and says when they could not be read", () => {
        const { getByTestId } = rig([agent("Ann", "local"), agent("Zo", "far")], {}, [], { cloudUnavailable: true });
        const legend = getByTestId("atlas-legend").textContent!;
        expect(legend).toContain("far-1");
        expect(legend).toContain("cloud");
        expect(legend).toContain("this machine");
        expect(getByTestId("atlas-cloud-note").textContent).toContain("could not be read");
    });

    it("says nothing of the cloud while it reads fine, and says what is wrong with this machine after its badge", () => {
        const { queryByTestId, getByTestId } = rig([agent("Ann", "local")], {}, [], { localNote: "container runtime not running" });
        expect(queryByTestId("atlas-cloud-note")).toBeNull();
        expect(getByTestId("atlas-machine-note").textContent).toBe("desk: this machine · container runtime not running");
    });
});
