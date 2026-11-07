// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** An agent's row in the Swarm (AgentRow), path by path: the group and card it draws, the classes and colours
 *  it takes, how the card's presses reach the activation handlers, the right-click copy menu, the tooltip, what
 *  it works out for the header and the lines under it (the display status, the collapsed flag, when the agent
 *  was last active, what is long-running or in the background, why it is unavailable, and the total of child
 *  rows), the fleet checkbox, the refused-open alert, and the children it mounts when expanded and in which
 *  order. The header, the lines, the hooks and every source of data are stubs that print what they were given,
 *  so what is checked is only what this component decides. Written ahead of splitting `AgentRow`. */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, type Accessor } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    blocks: {} as Record<string, { meta?: Record<string, unknown> }>,
    light: false,
    tick: (() => 0) as () => number,
    longRows: [] as { id: string; title: string; startedAt: number; sleepMs?: number }[],
    bgRows: [] as { id: string }[],
    supRows: [] as { id: string }[],
    lastMs: null as number | null,
    awaiting: false,
    unavailable: null as unknown,
    flagValue: undefined as unknown,
    flashValue: false,
    modelRef: null as unknown,
    activation: null as unknown as {
        isParked: () => boolean;
        openRefused: () => string;
        onClick: (e: Event) => void;
        onPointerDown: (e: Event) => void;
        onKeyDown: (e: Event) => void;
        onMouseDown: (e: Event) => void;
    },
    calls: {
        colors: [] as unknown[][],
        longRunning: [] as unknown[][],
        visibleBg: [] as unknown[][],
        supervisor: [] as unknown[][],
        lastMs: [] as unknown[][],
        lastLabel: [] as unknown[][],
        awaiting: [] as unknown[][],
        unavailable: [] as unknown[][],
        phase: [] as unknown[][],
        flagKey: [] as unknown[],
        activationArgs: [] as unknown[][],
        copyMenu: [] as unknown[][],
        supervisorMounts: 0,
        orefs: [] as string[],
    },
}));

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", async (orig) => {
    const real = await orig<typeof import("@/app/store/global")>();
    return {
        ...real,
        MOS: {
            ...real.MOS,
            makeORef: (type: string, id: string) => `${type}:${id}`,
            getMuxObjectAtom: (oref: string) => {
                h.calls.orefs.push(oref);
                return () => h.blocks[oref];
            },
        },
    };
});
vi.mock("@/app/block/pane-identity", async (orig) => ({
    ...(await orig<typeof import("@/app/block/pane-identity")>()),
    isLightThemeActive: () => h.light,
}));
vi.mock("./swarm-row-colors", () => ({
    swarmRowColors: (...a: unknown[]) => {
        h.calls.colors.push(a);
        const meta = a[0] as { id?: string } | undefined;
        return { active: `active(${meta?.id ?? "none"},${a[1]})`, hover: `hover(${meta?.id ?? "none"},${a[1]})` };
    },
}));
vi.mock("@/app/hook/useTick", () => ({ useTick: () => h.tick }));
vi.mock("./swarm-longrunning", () => ({
    longRunningToolRows: (...a: unknown[]) => {
        h.calls.longRunning.push(a);
        return h.longRows;
    },
}));
vi.mock("./swarm-background-tasks", () => ({
    visibleBackgroundTasks: (...a: unknown[]) => {
        h.calls.visibleBg.push(a);
        return h.bgRows;
    },
    BackgroundTaskBucket: (p: { tasks: unknown[] }) => <div data-testid="bg-bucket" data-count={p.tasks.length} />,
    SubagentBackgroundTasks: (p: { tasks: unknown[] }) => <div data-testid="sub-bg" data-count={p.tasks.length} />,
}));
vi.mock("./swarm-supervisor", () => ({
    supervisorRows: (...a: unknown[]) => {
        h.calls.supervisor.push(a);
        return h.supRows;
    },
    SupervisorBucket: (p: { rows: unknown[] }) => {
        h.calls.supervisorMounts++;
        return <div data-testid="supervisor-bucket" data-count={p.rows.length} />;
    },
}));
vi.mock("./swarm-last-active", () => ({
    lastActiveMs: (...a: unknown[]) => {
        h.calls.lastMs.push(a);
        return h.lastMs;
    },
    lastActiveLabel: (...a: unknown[]) => {
        h.calls.lastLabel.push(a);
        return `label(${a[0]},${a[1]})`;
    },
}));
vi.mock("@/app/store/swarm-line", () => ({
    isAwaitingUser: (...a: unknown[]) => {
        h.calls.awaiting.push(a);
        return h.awaiting;
    },
}));
vi.mock("@/app/store/agent-unavailable", () => ({
    unavailableOfMeta: (...a: unknown[]) => {
        h.calls.unavailable.push(a);
        return h.unavailable;
    },
}));
vi.mock("./swarm-display-status", () => ({
    phaseToDisplayStatus: (...a: unknown[]) => {
        h.calls.phase.push(a);
        return `status(${a[0]},${a[1]})`;
    },
}));
vi.mock("./swarm-row-registration", () => ({
    useRegistrationFlag: (key: () => unknown) => {
        h.calls.flagKey.push(key());
        return () => h.flagValue;
    },
    stopReceivingItems: (flag: unknown) => [{ label: `stop(${JSON.stringify(flag)})` }],
}));
vi.mock("./swarm-agent-row-hooks", () => ({
    useAgentRowActivation: (...a: unknown[]) => {
        h.calls.activationArgs.push(a);
        return h.activation;
    },
    useSummaryFlash: () => () => h.flashValue,
}));
vi.mock("@/app/store/contextmenu", () => ({
    showCopyContextMenu: (...a: unknown[]) => void h.calls.copyMenu.push(a),
}));
vi.mock("./swarm-agent-card", () => ({
    AgentCardHead: (p: Record<string, unknown> & { selectBox: unknown }) => (
        <div
            data-testid="head"
            data-collapsed={String((p.collapsed as () => boolean)())}
            data-has-children={String((p.hasChildren as () => boolean)())}
            data-total-rows={String((p.totalRows as () => number)())}
            data-parked={String((p.isParked as () => boolean)())}
            data-last-active={String((p.lastActive as () => unknown)())}
            data-last-active-text={String((p.lastActiveText as () => string)())}
            data-long-running={String((p.longRunningRows as () => unknown[])().length)}
            data-flag={JSON.stringify((p.registrationFlag as () => unknown)() ?? null)}
            data-unavailable={JSON.stringify((p.unavailable as () => unknown)())}
            data-status={String((p.displayStatus as () => string)())}
            data-row-now={String((p.rowNow as () => number)())}
            data-meta-id={String(((p.blockMeta as () => { id?: string } | undefined)() ?? {}).id ?? "none")}
            data-model-same={String(p.model === h.modelRef)}
        >
            {p.selectBox as never}
        </div>
    ),
    AgentCardLines: (p: Record<string, unknown>) => (
        <div
            data-testid="lines"
            data-unavailable={JSON.stringify((p.unavailable as () => unknown)())}
            data-flash={String((p.summaryFlash as () => boolean)())}
            data-row-now={String((p.rowNow as () => number)())}
            data-model-same={String(p.model === h.modelRef)}
        />
    ),
}));

import { NO_BACKGROUND_TASKS } from "./swarm-background";
import type { AgentTreeNode, SwarmViewModel } from "./swarm-model";
import { PARKED_ROW_TITLE } from "./swarm-open-parked";
import { AgentRow } from "./swarm-view";

const BLOCK = "b1";

function treeNode(over: Partial<AgentTreeNode> = {}): AgentTreeNode {
    return {
        blockId: BLOCK,
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
        ...over,
    } as AgentTreeNode;
}

function makeActivation(over: Record<string, unknown> = {}) {
    return {
        isParked: () => false,
        openRefused: () => "",
        onClick: vi.fn(),
        onPointerDown: vi.fn(),
        onKeyDown: vi.fn(),
        onMouseDown: vi.fn(),
        ...over,
    };
}

interface Setup {
    node?: AgentTreeNode;
    collapsed?: boolean;
    focused?: string | null;
    rowKey?: string;
    selected?: boolean;
}

function mount(opts: Setup = {}) {
    const [node, setNode] = createSignal(opts.node ?? treeNode());
    const [collapsed, setCollapsed] = createSignal(opts.collapsed ?? false);
    const [focused, setFocused] = createSignal<string | null>(opts.focused ?? null);
    const isAgentCollapsed = vi.fn(() => collapsed());
    const isSelected = vi.fn(() => opts.selected ?? false);
    const toggleSelected = vi.fn();
    const model = {
        isAgentCollapsed,
        isSelected,
        toggleSelected,
        isExpanded: () => false,
        toggleDispatchExpanded: vi.fn(),
        retireRow: vi.fn(),
        pauseCountdown: vi.fn(),
        resumeCountdown: vi.fn(),
        countdownStateAtom: () => new Map(),
    } as unknown as SwarmViewModel;
    h.modelRef = model;
    const r = render(() => (
        <div class="swarm-view">
            <AgentRow
                node={node as Accessor<AgentTreeNode>}
                focusedBlockId={focused}
                model={model}
                rowKey={opts.rowKey}
            />
        </div>
    ));
    const q = (sel: string) => r.container.querySelector(sel) as HTMLElement | null;
    return {
        ...r,
        q,
        node,
        setNode,
        setCollapsed,
        setFocused,
        isAgentCollapsed,
        isSelected,
        toggleSelected,
        card: () => q(".swarm-agent-card")!,
    };
}

beforeEach(() => {
    h.blocks = {};
    h.light = false;
    h.tick = () => 0;
    h.longRows = [];
    h.bgRows = [];
    h.supRows = [];
    h.lastMs = null;
    h.awaiting = false;
    h.unavailable = null;
    h.flagValue = undefined;
    h.flashValue = false;
    h.activation = makeActivation();
    for (const k of Object.keys(h.calls) as (keyof typeof h.calls)[]) {
        const v = h.calls[k];
        if (Array.isArray(v)) v.length = 0;
    }
    h.calls.supervisorMounts = 0;
});
afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
});

describe("the group and the card", () => {
    it("is a group holding the card, keyed by the list's key", () => {
        const m = mount({ rowKey: "k1" });
        const group = m.q(".swarm-agent-group")!;
        expect(group.getAttribute("data-swarm-key")).toBe("k1");
        expect(group.firstElementChild).toBe(m.card());
    });

    it("has no key attribute without one", () => {
        expect(mount().q(".swarm-agent-group")!.hasAttribute("data-swarm-key")).toBe(false);
    });

    it("names its status in a class", () => {
        const m = mount();
        expect(m.card().classList.contains("swarm-agent-card--running")).toBe(true);
        m.setNode(treeNode({ agentStatus: "idle" }));
        expect(m.card().classList.contains("swarm-agent-card--idle")).toBe(true);
        expect(m.card().classList.contains("swarm-agent-card--running")).toBe(false);
    });

    it("is active while it is the focused agent, and follows focus", () => {
        const m = mount({ focused: BLOCK });
        expect(m.card().classList.contains("swarm-agent-card--active")).toBe(true);
        m.setFocused("other");
        expect(m.card().classList.contains("swarm-agent-card--active")).toBe(false);
        m.setFocused(BLOCK);
        expect(m.card().classList.contains("swarm-agent-card--active")).toBe(true);
    });

    it("is not active when another agent is focused", () => {
        expect(mount({ focused: "other" }).card().classList.contains("swarm-agent-card--active")).toBe(false);
    });

    it("carries its block id", () => {
        expect(mount().card().getAttribute("data-block-id")).toBe(BLOCK);
    });

    it("carries no block id when it has none", () => {
        const m = mount({ node: treeNode({ blockId: null as never }) });
        expect(m.card().hasAttribute("data-block-id")).toBe(false);
    });

    it("is a focusable button", () => {
        const c = mount().card();
        expect(c.getAttribute("role")).toBe("button");
        expect(c.getAttribute("tabindex")).toBe("0");
    });

    it("holds the header, then the lines", () => {
        const kids = Array.from(mount().card().children).map((c) => c.getAttribute("data-testid"));
        expect(kids).toEqual(["head", "lines"]);
    });
});

describe("the colours", () => {
    it("takes its border colours from the agent's pane meta", () => {
        h.blocks[`block:${BLOCK}`] = { meta: { id: "m1" } };
        const c = mount().card();
        expect(c.style.getPropertyValue("--swarm-agent-active-border")).toBe("active(m1,false)");
        expect(c.style.getPropertyValue("--swarm-agent-hover-border")).toBe("hover(m1,false)");
    });

    it("asks for the light-theme variant in a light theme", () => {
        h.light = true;
        h.blocks[`block:${BLOCK}`] = { meta: { id: "m1" } };
        const c = mount().card();
        expect(c.style.getPropertyValue("--swarm-agent-active-border")).toBe("active(m1,true)");
        expect(h.calls.colors.at(-1)![1]).toBe(true);
    });

    it("reads the block of its own node", () => {
        mount({ node: treeNode({ blockId: "zz" }) });
        expect(h.calls.orefs).toContain("block:zz");
        expect(h.calls.orefs).not.toContain(`block:${BLOCK}`);
    });

    it("looks up no block, and has no meta, for an agent with no block id", () => {
        const m = mount({ node: treeNode({ blockId: null as never }) });
        expect(h.calls.orefs).toEqual([]);
        expect(h.calls.colors.at(-1)![0]).toBeUndefined();
        expect(m.q('[data-testid="head"]')!.getAttribute("data-meta-id")).toBe("none");
    });

    it("hands the pane meta to the header", () => {
        h.blocks[`block:${BLOCK}`] = { meta: { id: "m1" } };
        expect(mount().q('[data-testid="head"]')!.getAttribute("data-meta-id")).toBe("m1");
    });
});

describe("the card's handlers", () => {
    it("hands a click, a press, a key and a mouse press to the activation handlers", () => {
        const m = mount();
        fireEvent.click(m.card());
        expect(h.activation.onClick).toHaveBeenCalledTimes(1);
        fireEvent.pointerDown(m.card());
        expect(h.activation.onPointerDown).toHaveBeenCalledTimes(1);
        fireEvent.keyDown(m.card(), { key: "Enter" });
        expect(h.activation.onKeyDown).toHaveBeenCalledTimes(1);
        fireEvent.mouseDown(m.card());
        expect(h.activation.onMouseDown).toHaveBeenCalledTimes(1);
        expect(h.activation.onClick).toHaveBeenCalledTimes(1);
    });

    it("hands each handler its own event", () => {
        const m = mount();
        fireEvent.click(m.card());
        expect((h.activation.onClick as ReturnType<typeof vi.fn>).mock.calls[0][0].type).toBe("click");
        fireEvent.keyDown(m.card(), { key: "x" });
        expect((h.activation.onKeyDown as ReturnType<typeof vi.fn>).mock.calls[0][0].type).toBe("keydown");
    });
});

describe("the tooltip", () => {
    it("names the agent", () => {
        expect(mount().card().getAttribute("data-hover-tip")).toBe("AgentX");
    });

    it("says it is parked when the agent has no pane, and follows that", () => {
        const [parked, setParked] = createSignal(true);
        h.activation = makeActivation({ isParked: parked });
        const m = mount();
        expect(m.card().getAttribute("data-hover-tip")).toBe(`AgentX: ${PARKED_ROW_TITLE}`);
        setParked(false);
        expect(m.card().getAttribute("data-hover-tip")).toBe("AgentX");
    });
});

describe("the copy menu", () => {
    it("offers the agent's name and block id, and what stops its registration", () => {
        h.flagValue = { reason: "x" };
        const m = mount();
        fireEvent.contextMenu(m.card());
        expect(h.calls.copyMenu).toHaveLength(1);
        const [items, ev, extra] = h.calls.copyMenu[0];
        expect(items).toEqual([
            { label: "Copy agent name", value: "AgentX" },
            { label: "Copy block ID", value: BLOCK },
        ]);
        expect((ev as Event).type).toBe("contextmenu");
        expect(extra).toEqual([{ label: 'stop({"reason":"x"})' }]);
    });

    it("passes no flag when the agent's registration is fine", () => {
        const m = mount();
        fireEvent.contextMenu(m.card());
        expect(h.calls.copyMenu[0][2]).toEqual([{ label: "stop(undefined)" }]);
    });

    it("reads the node as it is when the menu opens", () => {
        const m = mount();
        m.setNode(treeNode({ agentName: "Renamed", blockId: "b2" }));
        fireEvent.contextMenu(m.card());
        expect(h.calls.copyMenu[0][0]).toEqual([
            { label: "Copy agent name", value: "Renamed" },
            { label: "Copy block ID", value: "b2" },
        ]);
    });
});

describe("what it works out for the header", () => {
    const head = (m: ReturnType<typeof mount>) => m.q('[data-testid="head"]')!;

    it("gives the display status for the block and its status", () => {
        const m = mount();
        expect(head(m).getAttribute("data-status")).toBe(`status(${BLOCK},running)`);
        m.setNode(treeNode({ agentStatus: "idle" }));
        expect(head(m).getAttribute("data-status")).toBe(`status(${BLOCK},idle)`);
    });

    it("hands the header the model, and asks it whether this agent is collapsed", () => {
        const m = mount({ collapsed: true });
        expect(head(m).getAttribute("data-model-same")).toBe("true");
        expect(m.isAgentCollapsed).toHaveBeenCalledWith(BLOCK);
        expect(head(m).getAttribute("data-collapsed")).toBe("true");
        m.setCollapsed(false);
        expect(head(m).getAttribute("data-collapsed")).toBe("false");
    });

    it("hands the lines the model too", () => {
        expect(mount().q('[data-testid="lines"]')!.getAttribute("data-model-same")).toBe("true");
    });

    it("passes the parked state, the registration flag and the registration key", () => {
        h.activation = makeActivation({ isParked: () => true });
        h.flagValue = { why: "dup" };
        const m = mount();
        expect(head(m).getAttribute("data-parked")).toBe("true");
        expect(head(m).getAttribute("data-flag")).toBe('{"why":"dup"}');
        expect(h.calls.flagKey).toEqual([BLOCK]);
    });

    it("passes no flag when there is none", () => {
        expect(head(mount()).getAttribute("data-flag")).toBe("null");
    });

    it("works out when the agent was last active, from its pane meta", () => {
        h.blocks[`block:${BLOCK}`] = { meta: { id: "m1" } };
        h.lastMs = 12345;
        const m = mount();
        expect(h.calls.lastMs.at(-1)![0]).toEqual({ id: "m1" });
        expect(head(m).getAttribute("data-last-active")).toBe("12345");
    });

    it("words it against now, and again on every tick", () => {
        vi.useFakeTimers();
        vi.setSystemTime(1000);
        h.lastMs = 500;
        const [tick, setTick] = createSignal(0);
        h.tick = tick;
        const m = mount();
        expect(head(m).getAttribute("data-last-active-text")).toBe("label(500,1000)");
        vi.setSystemTime(2000);
        setTick(1);
        expect(head(m).getAttribute("data-last-active-text")).toBe("label(500,2000)");
        vi.useRealTimers();
    });

    it("lists the long-running rows for the block, now, and again on every tick", () => {
        vi.useFakeTimers();
        vi.setSystemTime(5000);
        h.longRows = [{ id: "l1", title: "sleep 1", startedAt: 0 }];
        const [tick, setTick] = createSignal(0);
        h.tick = tick;
        const m = mount();
        expect(h.calls.longRunning[0]).toEqual([BLOCK, 5000]);
        expect(head(m).getAttribute("data-long-running")).toBe("1");
        const before = h.calls.longRunning.length;
        vi.setSystemTime(6000);
        setTick(1);
        expect(h.calls.longRunning.length).toBeGreaterThan(before);
        expect(h.calls.longRunning.at(-1)).toEqual([BLOCK, 6000]);
        vi.useRealTimers();
    });

    it("narrows the agent's own background tasks to those still shown, now", () => {
        vi.useFakeTimers();
        vi.setSystemTime(7000);
        const own = [{ id: "o1" }];
        h.bgRows = [{ id: "o1" }];
        const m = mount({ node: treeNode({ backgroundTasks: { own, bySubagent: new Map() } as never }) });
        expect(h.calls.visibleBg[0]).toEqual([own, 7000]);
        expect(m.q('[data-testid="bg-bucket"]')!.getAttribute("data-count")).toBe("1");
        vi.useRealTimers();
    });

    it("narrows the background tasks again on every tick", () => {
        vi.useFakeTimers();
        vi.setSystemTime(7000);
        const [tick, setTick] = createSignal(0);
        h.tick = tick;
        mount();
        const before = h.calls.visibleBg.length;
        vi.setSystemTime(8000);
        setTick(1);
        expect(h.calls.visibleBg.length).toBeGreaterThan(before);
        expect(h.calls.visibleBg.at(-1)![1]).toBe(8000);
        vi.useRealTimers();
    });

    it("gives the activation handlers the node and a reader of the pane meta", () => {
        h.blocks[`block:${BLOCK}`] = { meta: { id: "m1" } };
        const m = mount();
        expect(h.calls.activationArgs).toHaveLength(1);
        const [nodeArg, metaArg] = h.calls.activationArgs[0] as [() => AgentTreeNode, () => unknown];
        expect(nodeArg()).toBe(m.node());
        expect(metaArg()).toEqual({ id: "m1" });
    });

    it("reads the Supervisor's decisions from the pane meta", () => {
        h.blocks[`block:${BLOCK}`] = { meta: { id: "m1" } };
        h.supRows = [{ id: "s1" }, { id: "s2" }];
        const m = mount();
        expect(h.calls.supervisor.at(-1)![0]).toEqual({ id: "m1" });
        expect(m.q('[data-testid="supervisor-bucket"]')!.getAttribute("data-count")).toBe("2");
    });

    it("gives a row now, as a number that moves with the tick", () => {
        vi.useFakeTimers();
        vi.setSystemTime(1000);
        const [tick, setTick] = createSignal(0);
        h.tick = tick;
        const m = mount();
        expect(head(m).getAttribute("data-row-now")).toBe("1000");
        expect(m.q('[data-testid="lines"]')!.getAttribute("data-row-now")).toBe("1000");
        vi.setSystemTime(3000);
        setTick(1);
        expect(head(m).getAttribute("data-row-now")).toBe("3000");
        expect(m.q('[data-testid="lines"]')!.getAttribute("data-row-now")).toBe("3000");
        vi.useRealTimers();
    });
});

describe("why the agent is unavailable", () => {
    const head = (m: ReturnType<typeof mount>) => m.q('[data-testid="head"]')!;

    it("asks the meta, whether the agent is running, and now", () => {
        vi.useFakeTimers();
        vi.setSystemTime(9000);
        h.blocks[`block:${BLOCK}`] = { meta: { id: "m1" } };
        h.unavailable = { kind: "quota", until: 99 };
        const m = mount();
        expect(h.calls.unavailable.at(-1)).toEqual([{ id: "m1" }, true, 9000]);
        expect(head(m).getAttribute("data-unavailable")).toBe('{"kind":"quota","until":99}');
        expect(m.q('[data-testid="lines"]')!.getAttribute("data-unavailable")).toBe('{"kind":"quota","until":99}');
        vi.useRealTimers();
    });

    it("says the agent is not running when it is idle", () => {
        mount({ node: treeNode({ agentStatus: "idle" }) });
        expect(h.calls.unavailable.at(-1)![1]).toBe(false);
    });

    it("asks whether the agent is waiting on the user, from the meta and the status", () => {
        h.blocks[`block:${BLOCK}`] = { meta: { id: "m1" } };
        mount();
        expect(h.calls.awaiting.at(-1)).toEqual([{ id: "m1" }, "running"]);
    });

    it("says nothing is wrong, and does not ask, while the agent is waiting on the user", () => {
        h.awaiting = true;
        h.unavailable = { kind: "quota" };
        const m = mount();
        expect(head(m).getAttribute("data-unavailable")).toBe("null");
        expect(h.calls.unavailable).toEqual([]);
    });
});

describe("the child rows it counts", () => {
    const head = (m: ReturnType<typeof mount>) => m.q('[data-testid="head"]')!;

    it("has none, and no children, for an agent with nothing going on", () => {
        const m = mount();
        expect(head(m).getAttribute("data-total-rows")).toBe("0");
        expect(head(m).getAttribute("data-has-children")).toBe("false");
    });

    it("counts every kind of row, once", () => {
        h.longRows = [{ id: "l1", title: "t", startedAt: 0 }];
        h.bgRows = [{ id: "b1" }, { id: "b2" }];
        h.supRows = [{ id: "s1" }];
        const m = mount({
            collapsed: true,
            node: treeNode({
                todoRows: [{ text: "a", status: "pending" }],
                agentToolRows: [{ id: "a1" }, { id: "a2" }] as never,
                workflowRows: [{ id: "w1" }] as never,
                shellRows: [{ shell_id: "sh" }] as never,
                cronRows: [{ id: "c1" }] as never,
            }),
        });
        expect(head(m).getAttribute("data-total-rows")).toBe(String(1 + 2 + 1 + 1 + 1 + 1 + 2 + 1));
        expect(head(m).getAttribute("data-has-children")).toBe("true");
    });

    it("has children from the Supervisor's decisions alone", () => {
        h.supRows = [{ id: "s1" }];
        const m = mount();
        expect(head(m).getAttribute("data-total-rows")).toBe("1");
        expect(head(m).getAttribute("data-has-children")).toBe("true");
    });

    it("has children from the agent's background tasks alone", () => {
        h.bgRows = [{ id: "b1" }];
        const m = mount();
        expect(head(m).getAttribute("data-total-rows")).toBe("1");
        expect(head(m).getAttribute("data-has-children")).toBe("true");
    });

    it("has children from a long-running call alone", () => {
        h.longRows = [{ id: "l1", title: "t", startedAt: 0 }];
        const m = mount();
        expect(head(m).getAttribute("data-total-rows")).toBe("1");
    });

    it("follows the node as its rows change", () => {
        const m = mount();
        m.setNode(
            treeNode({
                todoRows: [
                    { text: "a", status: "pending" },
                    { text: "b", status: "pending" },
                ],
            })
        );
        expect(head(m).getAttribute("data-total-rows")).toBe("2");
    });
});

describe("the fleet checkbox", () => {
    it("is handed to the header, unchecked for an agent that is not selected", () => {
        const m = mount();
        const box = m.q('[data-testid="head"] input.swarm-agent-select-checkbox') as HTMLInputElement;
        expect(box.type).toBe("checkbox");
        expect(box.checked).toBe(false);
        expect(box.title).toBe("Select for a fleet action (broadcast/stop)");
        expect(m.isSelected).toHaveBeenCalledWith(BLOCK);
    });

    it("is checked for a selected agent", () => {
        const m = mount({ selected: true });
        expect((m.q("input.swarm-agent-select-checkbox") as HTMLInputElement).checked).toBe(true);
    });

    it("toggles the agent's selection when changed", () => {
        const m = mount();
        fireEvent.click(m.q("input.swarm-agent-select-checkbox")!);
        expect(m.toggleSelected).toHaveBeenCalledWith(BLOCK);
    });

    it("keeps its click from reaching the card", () => {
        const m = mount();
        fireEvent.click(m.q("input.swarm-agent-select-checkbox")!);
        expect(h.activation.onClick).not.toHaveBeenCalled();
    });
});

describe("the lines", () => {
    it("get the unavailable reason and the summary flash", () => {
        h.flashValue = true;
        h.unavailable = { kind: "signed_out" };
        const m = mount();
        const lines = m.q('[data-testid="lines"]')!;
        expect(lines.getAttribute("data-flash")).toBe("true");
        expect(lines.getAttribute("data-unavailable")).toBe('{"kind":"signed_out"}');
    });

    it("are not flashing by default", () => {
        expect(mount().q('[data-testid="lines"]')!.getAttribute("data-flash")).toBe("false");
    });
});

describe("a refused open", () => {
    it("says why under the card", () => {
        h.activation = makeActivation({ openRefused: () => "No such agent" });
        const m = mount();
        const alert = m.q('[role="alert"]')!;
        expect(alert.textContent).toBe("No such agent");
        expect(alert.classList.contains("swarm-stopped__error")).toBe(true);
        expect(alert.previousElementSibling).toBe(m.card());
    });

    it("shows nothing when nothing was refused", () => {
        expect(mount().q('[role="alert"]')).toBeNull();
    });

    it("follows the reason as it changes", () => {
        const [why, setWhy] = createSignal("");
        h.activation = makeActivation({ openRefused: why });
        const m = mount();
        expect(m.q('[role="alert"]')).toBeNull();
        setWhy("Nope");
        expect(m.q('[role="alert"]')!.textContent).toBe("Nope");
    });
});

describe("the children", () => {
    it("are not mounted while the row is collapsed", () => {
        const m = mount({ collapsed: true });
        expect(m.q(".swarm-children")).toBeNull();
        expect(m.q('[data-testid="supervisor-bucket"]')).toBeNull();
    });

    it("are mounted under the card when the row is expanded", () => {
        const m = mount();
        const kids = m.q(".swarm-children")!;
        expect(kids.previousElementSibling).toBe(m.card());
        expect(kids.parentElement).toBe(m.q(".swarm-agent-group"));
    });

    it("come and go as the row is collapsed and expanded", () => {
        const m = mount();
        expect(m.q(".swarm-children")).not.toBeNull();
        m.setCollapsed(true);
        expect(m.q(".swarm-children")).toBeNull();
        m.setCollapsed(false);
        expect(m.q(".swarm-children")).not.toBeNull();
    });

    it("come after a refused-open alert", () => {
        h.activation = makeActivation({ openRefused: () => "Nope" });
        const m = mount();
        const order = Array.from(m.q(".swarm-agent-group")!.children).map((c) =>
            c.classList.contains("swarm-agent-card") ? "card" : (c.getAttribute("role") ?? c.className)
        );
        expect(order).toEqual(["card", "alert", "swarm-children"]);
    });

    it("hold each bucket, in order: Supervisor, Todos, Shell, Cron, Running, Background", () => {
        h.supRows = [{ id: "s1" }];
        h.longRows = [{ id: "l1", title: "sleep 9", startedAt: 0 }];
        h.bgRows = [{ id: "b1" }];
        const m = mount({
            node: treeNode({
                todoRows: [{ text: "a", status: "pending" }],
                shellRows: [
                    { shell_id: "sh", block_id: BLOCK, cmd: "ls", title: "ls", started_at: 0, line_count: 0 },
                ] as never,
                cronRows: [
                    {
                        id: "c1",
                        block_id: BLOCK,
                        name: "n",
                        expression: "* * * * *",
                        target: "t",
                        created_by: "x",
                        enabled: true,
                        last_fired: null,
                        fire_count: 0,
                        max_fires: null,
                        next_fire: null,
                    },
                ] as never,
            }),
        });
        const order = Array.from(m.q(".swarm-children")!.children).map(
            (c) => c.getAttribute("data-testid") ?? c.className.replace("swarm-bucket ", "")
        );
        expect(order).toEqual([
            "supervisor-bucket",
            "swarm-bucket--todo",
            "swarm-bucket--shell",
            "swarm-bucket--cron",
            "swarm-bucket--longrunning",
            "bg-bucket",
        ]);
    });

    it("show the todo bucket's progress from the node's own rows, with what was cut off", () => {
        const m = mount({
            node: treeNode({
                todoRows: [
                    { text: "a", status: "completed" },
                    { text: "b", status: "pending" },
                ],
                todosTruncated: 3,
            }),
        });
        expect(m.q(".swarm-bucket--todo .swarm-bucket-count")!.textContent).toBe("1/2");
        expect(m.q(".swarm-todo-truncated")!.textContent).toBe("+3 more");
    });

    it("note when earlier todos may be missing", () => {
        const m = mount({ node: treeNode({ todoRows: [{ text: "a", status: "pending" }], todosPartial: true }) });
        expect(m.q(".swarm-todo-partial")!.textContent!.trim()).toBe("earlier items may be missing");
        cleanup();
        const m2 = mount({ node: treeNode({ todoRows: [{ text: "a", status: "pending" }], todosPartial: false }) });
        expect(m2.q(".swarm-todo-partial")).toBeNull();
    });

    it("hold a subagent row for each Agent Tool row, dimmed by the parent's status (cut off when the parent is idle)", () => {
        const sub = {
            agent_id: "sub-1234567",
            status: "active",
            slug: "scout",
            display_name: "Scout",
            tool_use_id: "tu1",
            dispatch_id: "d1",
            last_event_at: 1,
        };
        const m = mount({ node: treeNode({ agentToolRows: [sub] as never }) });
        expect(m.q(".swarm-bucket--agent-tool .swarm-bucket-label")!.textContent).toBe("Agent Tool");
        expect(m.q(".swarm-bucket--agent-tool .swarm-bucket-count")!.textContent).toBe("1");
        expect(m.q(".swarm-subagent-slug")!.textContent).toBe("Scout");
        expect(m.q(".swarm-subagent-row")!.classList.contains("swarm-subagent-row--active")).toBe(true);
        m.setNode(treeNode({ agentStatus: "idle", agentToolRows: [sub] as never }));
        expect(m.q(".swarm-subagent-row")!.classList.contains("swarm-subagent-row--abandoned")).toBe(true);
    });

    it("give a subagent row the background tasks its tool call launched", () => {
        const mine = [{ id: "bg1" }];
        const sub = {
            agent_id: "sub-1234567",
            status: "active",
            slug: "scout",
            display_name: "Scout",
            tool_use_id: "tu1",
            dispatch_id: "d1",
            last_event_at: 1,
        };
        const other = { ...sub, agent_id: "sub-7654321", tool_use_id: undefined };
        h.bgRows = [{ id: "bg1" }];
        mount({
            node: treeNode({
                agentToolRows: [sub, other] as never,
                backgroundTasks: { own: [], bySubagent: new Map([["tu1", mine]]) } as never,
            }),
        });
        const lists = h.calls.visibleBg.map((c) => c[0]);
        expect(lists).toContain(mine);
        expect(lists.filter((l) => Array.isArray(l) && l.length === 0).length).toBeGreaterThan(0);
        expect(document.querySelectorAll('[data-testid="sub-bg"]')).toHaveLength(2);
    });

    it("hold a row for each Workflow dispatch, with its progress", () => {
        const wf = {
            kind: "workflowDispatch",
            dispatchId: "wf1",
            name: "Build",
            memberCount: 3,
            membersDone: 1,
            status: "active",
            lastEventAt: 1,
        };
        const m = mount({ node: treeNode({ workflowRows: [wf] as never }) });
        expect(m.q(".swarm-bucket--workflow .swarm-bucket-label")!.textContent).toBe("Workflow");
        expect(m.q(".swarm-bucket--workflow .swarm-bucket-count")!.textContent).toBe("1");
        expect(m.q(".swarm-workflow-name")!.textContent).toBe("Build");
        expect(m.q(".swarm-workflow-group")!.textContent).toContain("2/3 active");
    });

    it("keep the Agent Tool and Workflow buckets between the Todos and the Shell", () => {
        const sub = {
            agent_id: "sub-1234567",
            status: "active",
            slug: "scout",
            display_name: "Scout",
            tool_use_id: "tu1",
            dispatch_id: "d1",
            last_event_at: 1,
        };
        const wf = {
            kind: "workflowDispatch",
            dispatchId: "wf1",
            name: "Build",
            memberCount: 3,
            membersDone: 1,
            status: "active",
            lastEventAt: 1,
        };
        const m = mount({
            node: treeNode({
                todoRows: [{ text: "a", status: "pending" }],
                agentToolRows: [sub] as never,
                workflowRows: [wf] as never,
                shellRows: [
                    { shell_id: "sh", block_id: BLOCK, cmd: "ls", title: "ls", started_at: 0, line_count: 0 },
                ] as never,
            }),
        });
        const order = Array.from(m.q(".swarm-children")!.children).map((c) => c.className.replace("swarm-bucket ", ""));
        expect(order.filter(Boolean)).toEqual([
            "swarm-bucket--todo",
            "swarm-bucket--agent-tool",
            "swarm-bucket--workflow",
            "swarm-bucket--shell",
        ]);
    });

    it("get the node as it is now, and are rebuilt when the node is replaced", () => {
        const m = mount({ node: treeNode({ todoRows: [{ text: "a", status: "pending" }] }) });
        const mounts = h.calls.supervisorMounts;
        expect(m.q(".swarm-bucket--todo .swarm-bucket-count")!.textContent).toBe("0/1");
        m.setNode(treeNode({ todoRows: [{ text: "a", status: "completed" }] }));
        expect(h.calls.supervisorMounts).toBeGreaterThan(mounts);
        expect(m.q(".swarm-bucket--todo .swarm-bucket-count")!.textContent).toBe("1/1");
    });

    it("are not rebuilt when the same node is set again", () => {
        const n = treeNode();
        const m = mount({ node: n });
        const mounts = h.calls.supervisorMounts;
        m.setNode(n);
        expect(h.calls.supervisorMounts).toBe(mounts);
    });
});
