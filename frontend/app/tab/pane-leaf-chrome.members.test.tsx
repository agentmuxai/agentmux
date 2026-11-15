// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a pane leaf does around its tabs, pinned as ordered logs: which members are mounted and in
 * which state, the wrappers each Block is handed, the calls to the mount watch, the minimized flag
 * and the dormant flag (and their cleanups), the latched chrome and keep-alive decisions, the
 * ViewModel the chrome sees, and the state a closed tab leaves behind. The Block, the shared
 * shell and the three registries are stand-ins that log; the leaf itself is real. A refactor of
 * PaneLeafChrome must leave the committed snapshots as they are; regenerate with `vitest -u` only
 * for a change meant to behave differently.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createEffect, createSignal, onCleanup, untrack, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { NodeModel } from "@/layout/index";
import { registerPaneTab } from "@/app/store/pane-tab-registry";
import { stubPaneTab } from "@/app/block/pane-tab-test-utils";
import { PaneStackMemberActiveProvider, usePaneStackMemberActive } from "@/app/tab/pane-stack-member";
import { resetTabLandingForTests, showTabLanding } from "@/layout/lib/tabLanding";
import { beginTabTearOff, endTabTearOff, tabTearOffShown } from "@/layout/lib/tabTearOff";

const h = vi.hoisted(() => ({
    log: [] as string[],
    ids: new WeakMap<object, number>(),
    next: 0,
}));
const log = (line: string) => h.log.push(line);
const numberOf = (o: object): number => {
    if (!h.ids.has(o)) h.ids.set(o, ++h.next);
    return h.ids.get(o)!;
};

for (const view of ["agent", "term", "help", "swarm"]) {
    registerPaneTab(stubPaneTab(view, view === "agent" || view === "term" ? { lifecycle: "keepAlive" } : {}));
}

type TestMeta = { view?: string; agentId?: string };
const metaSignals = new Map<string, ReturnType<typeof createSignal<{ meta?: TestMeta }>>>();
const metaOf = (id: string) => {
    if (!metaSignals.has(id)) metaSignals.set(id, createSignal<{ meta?: TestMeta }>({ meta: {} }));
    return metaSignals.get(id)!;
};
// An agent pane here has an agent chosen: only such a pane has a session to wait for (the picker has none).
const setView = (id: string, view: string | undefined) =>
    metaOf(id)[1]({ meta: view === "agent" ? { view, agentId: "agent-a" } : { view } });

const [treeVersion, bumpTree] = createSignal(0);
const stacks = new Map<string, string[] | undefined>();
const setStack = (stack: string[] | undefined) => {
    stacks.set("node-1", stack);
    bumpTree((v) => v + 1);
};
const fakeLayoutModel = { localTreeStateAtom: () => treeVersion(), treeState: { rootNode: {} } } as any;
vi.mock("@/layout/lib/layoutNode", () => ({
    findNode: (_root: unknown, nodeId: string) => ({ data: { blockStack: stacks.get(nodeId) } }),
}));

vi.mock("@/app/store/global", () => ({
    MOS: {
        makeORef: (_otype: string, oid: string) => oid,
        getMuxObjectAtom: (oid: string) => metaOf(oid)[0],
    },
}));
vi.mock("@/app/tab/agent-mount-watch", () => ({
    expectAgentMount: (id: string, describe: () => string) => log(`expectAgentMount ${id} :: ${untrack(describe)}`),
    agentMountSettled: (id: string) => log(`agentMountSettled ${id}`),
    agentMountForgotten: (id: string) => log(`agentMountForgotten ${id}`),
    agentMountStalled: () => false,
}));
vi.mock("@/app/block/pane-minimized", () => ({
    setBlockMinimized: (id: string, minimized: boolean) => log(`setBlockMinimized ${id} ${minimized}`),
}));
vi.mock("@/app/store/block-component-registry", () => ({
    setKeepAliveBlockDormant: (id: string, dormant: boolean) => log(`setKeepAliveBlockDormant ${id} ${dormant}`),
}));
vi.mock("@/app/element/PaneChrome", () => ({
    renderPaneChromeShell: (_nm: NodeModel, content: JSX.Element) => {
        log("renderPaneChromeShell");
        return <div data-testid="shell">{content}</div>;
    },
}));

let chromeNodeModel: NodeModel | null = null;
/** The node model each mounted Block was given, by block id. */
const blockNodeModels = new Map<string, NodeModel>();
/** Members whose ViewModel supplies no chrome of its own, so the leaf uses the shared shell. */
const noChrome = new Set<string>();
vi.mock("@/app/block/block", () => ({
    resolveEffectiveViewType: (v: string) => v,
    Block: (props: { nodeModel: NodeModel; preview: boolean }) => {
        const nm = props.nodeModel;
        const id = nm.blockId;
        blockNodeModels.set(id, nm);
        log(`Block mount ${id} wrapper#${numberOf(nm)} hoisted=${(nm as { paneChromeHoisted?: () => boolean }).paneChromeHoisted?.()} preview=${props.preview}`);
        const owner = {};
        const memberActive = usePaneStackMemberActive();
        createEffect(() => log(`Block ${id} memberActive=${memberActive()}`));
        createEffect(() => {
            nm.setActiveViewModel?.(
                {
                    viewType: "agent",
                    blockId: id,
                    renderPaneChrome: noChrome.has(id) ? undefined : (chromeNm: NodeModel, content: JSX.Element) => {
                        log(`renderPaneChrome by vm of ${id}`);
                        chromeNodeModel = chromeNm;
                        return <div data-testid="chrome">{content}</div>;
                    },
                } as unknown as ViewModel,
                owner,
            );
            log(`Block ${id} set its vm`);
        });
        onCleanup(() => {
            log(`Block unmount ${id}`);
            nm.setActiveViewModel?.(null, owner);
        });
        const el = document.createElement("div");
        el.dataset.testid = `block-${id}`;
        el.textContent = id;
        return el;
    },
}));

async function loadLeaf() {
    return (await import("./pane-leaf-chrome")).PaneLeafChrome;
}

interface Rig {
    setActive: (id: string) => void;
    setMinimized: (v: boolean) => void;
    setMagnified: (v: boolean) => void;
    setOuterActive: (v: boolean) => void;
}

async function mount(initialActive = "b1"): Promise<Rig> {
    const PaneLeafChrome = await loadLeaf();
    const [active, setActive] = createSignal(initialActive);
    const [minimized, setMinimized] = createSignal(false);
    const [magnified, setMagnified] = createSignal(false);
    const [outerActive, setOuterActive] = createSignal(true);
    const leafVm = createSignal<ViewModel | null>(null);
    const nodeModel = {
        blockId: "b1",
        nodeId: "node-1",
        layoutModel: fakeLayoutModel,
        activeBlockId: active,
        isMinimized: minimized,
        isMagnified: magnified,
        innerRect: () => ({ width: "400px - 3px", height: "300px - 3px" }),
        activeViewModel: leafVm[0],
        setActiveViewModel: (vm: ViewModel | null, _owner: object) => {
            log(`leaf setActiveViewModel ${(vm as any)?.blockId ?? null}`);
            leafVm[1](() => vm);
        },
    } as unknown as NodeModel;
    render(() => (
        <PaneStackMemberActiveProvider value={outerActive}>
            <PaneLeafChrome nodeModel={nodeModel} />
        </PaneStackMemberActiveProvider>
    ));
    return { setActive, setMinimized, setMagnified, setOuterActive };
}

const out = () => h.log.join("\n").replace(/\n+$/, "") + "\n";
const snap = (name: string) => `./__snapshots__/pane-leaf-chrome.members.${name}.txt`;
const dom = () => {
    const slots = [...document.querySelectorAll<HTMLElement>(".pane-leaf-keepalive-slot")].map(
        (s) =>
            `${s.textContent}[dormant=${s.getAttribute("data-pane-dormant")} vis=${s.style.visibility} op=${s.style.opacity} pe=${s.style.pointerEvents}]`,
    );
    const chrome = document.querySelectorAll('[data-testid="chrome"]').length;
    const shell = document.querySelectorAll('[data-testid="shell"]').length;
    const blocks = [...document.querySelectorAll('[data-testid^="block-"]')].map((b) => b.textContent);
    log(`dom chrome=${chrome} shell=${shell} blocks=${JSON.stringify(blocks)} slots=${JSON.stringify(slots)}`);
};
const chromeSees = () => log(`chrome sees vm of ${(chromeNodeModel?.activeViewModel?.() as any)?.blockId ?? null}`);

beforeEach(() => {
    h.log.length = 0;
    h.next = 0;
    h.ids = new WeakMap();
    chromeNodeModel = null;
    setStack(undefined);
});
afterEach(() => {
    cleanup();
    noChrome.clear();
    metaSignals.clear();
    stacks.clear();
});

describe("a single-member pane", () => {
    it("an agent pane gets the chrome and a Block, is watched for its mount, and lets go on unmount", async () => {
        setView("b1", "agent");
        await mount();
        dom();
        chromeSees();
        log("-- unmount");
        cleanup();
        await expect(out()).toMatchFileSnapshot(snap("single-agent"));
    });

    it("a help pane gets the shared shell and no keep-alive slot; the agent watch settles it", async () => {
        setView("b1", "help");
        await mount();
        dom();
        chromeSees();
        log("-- unmount");
        cleanup();
        await expect(out()).toMatchFileSnapshot(snap("single-help"));
    });

    it("a view that supplies no chrome of its own is wrapped in the shared shell", async () => {
        setView("b1", "help");
        noChrome.add("b1");
        await mount();
        dom();
        await expect(out()).toMatchFileSnapshot(snap("single-shell"));
    });

    it("an unregistered view type is a bare Block, watched as 'not loaded' until it resolves", async () => {
        setView("b1", "mystery");
        await mount();
        dom();
        await expect(out()).toMatchFileSnapshot(snap("single-unregistered"));
    });
});

describe("a view that arrives late", () => {
    it("starts as a bare Block, hoists when the view resolves, and keeps the chrome after the view changes to a plain one", async () => {
        await mount();
        log("-- unresolved");
        dom();
        setView("b1", "agent");
        log("-- agent");
        dom();
        chromeSees();
        setView("b1", "mystery");
        log("-- mystery (latched)");
        dom();
        setView("b1", "help");
        log("-- help (keep-alive latched)");
        dom();
        await expect(out()).toMatchFileSnapshot(snap("late-view"));
    });

    it("a meta update that leaves the view alone does not run the mount watch again", async () => {
        setView("b1", "agent");
        await mount();
        log("-- same view, new object");
        setView("b1", "agent");
        log("-- done");
        await expect(out()).toMatchFileSnapshot(snap("same-view"));
    });
});

describe("a stack of tabs", () => {
    it("mounts the active member and the keep-alive ones, hides the rest, and follows the active one", async () => {
        setView("a1", "agent");
        setView("h1", "help");
        setView("t1", "term");
        setStack(["a1", "h1", "t1"]);
        const rig = await mount("a1");
        dom();
        chromeSees();
        for (const id of ["h1", "t1", "a1"]) {
            log(`-- switch to ${id}`);
            rig.setActive(id);
            dom();
            chromeSees();
        }
        await expect(out()).toMatchFileSnapshot(snap("stack-switch"));
    });

    it("a tab pushed onto the stack mounts at once; a closed one is cleaned up and starts fresh when it comes back", async () => {
        setView("a1", "agent");
        setView("a2", "agent");
        setStack(["a1"]);
        await mount("a1");
        log("-- push a2");
        setStack(["a1", "a2"]);
        dom();
        log("-- close a2");
        setStack(["a1"]);
        dom();
        log("-- a2 comes back");
        setStack(["a1", "a2"]);
        dom();
        chromeSees();
        log("-- unmount");
        cleanup();
        await expect(out()).toMatchFileSnapshot(snap("stack-push-close"));
    });

    it("tells each Block whether its member is the active one inside an active pane, and not in a hidden one", async () => {
        setView("a1", "agent");
        setView("a2", "agent");
        setStack(["a1", "a2"]);
        const rig = await mount("a1");
        log("-- hide the pane");
        rig.setOuterActive(false);
        log("-- show the pane, switch to a2");
        rig.setOuterActive(true);
        rig.setActive("a2");
        await expect(out()).toMatchFileSnapshot(snap("stack-member-active"));
    });

    it("a stack of plain tabs mounts one Block at a time, each with a fresh wrapper", async () => {
        setView("h1", "help");
        setView("s1", "swarm");
        setStack(["h1", "s1"]);
        const rig = await mount("h1");
        dom();
        rig.setActive("s1");
        dom();
        chromeSees();
        await expect(out()).toMatchFileSnapshot(snap("stack-plain"));
    });
});

describe("a minimized pane", () => {
    it("marks every member minimized while it is a chip, not while it is maximized, and clears it on unmount", async () => {
        setView("a1", "agent");
        setView("h1", "help");
        setStack(["a1", "h1"]);
        const rig = await mount("a1");
        for (const [what, fn] of [
            ["minimize", () => rig.setMinimized(true)],
            ["maximize", () => rig.setMagnified(true)],
            ["restore the size", () => rig.setMagnified(false)],
            ["restore", () => rig.setMinimized(false)],
            ["minimize again", () => rig.setMinimized(true)],
        ] as const) {
            log(`-- ${what}`);
            fn();
        }
        log("-- unmount");
        cleanup();
        await expect(out()).toMatchFileSnapshot(snap("minimized"));
    });
});

describe("a tab dragged out of the pane", () => {
    afterEach(() => {
        endTabTearOff();
        resetTabLandingForTests();
    });

    /** The mounted members, the one in front marked. */
    const members = () =>
        [...document.querySelectorAll<HTMLElement>(".pane-leaf-keepalive-slot")]
            .map((s) => (s.style.visibility === "hidden" ? s.textContent : `[${s.textContent}]`))
            .join(" ");

    it("shows the tab that takes its place once it has left, keeps the dragged one mounted, and puts it back in front if it stays", async () => {
        setView("a1", "agent");
        setView("h1", "help");
        setView("h2", "help");
        setStack(["a1", "h1", "h2"]);
        const rig = await mount("a1");
        rig.setActive("h1");
        expect(members()).toBe("a1 [h1]");

        // Its own place still offers the tear-off: the tab is in front, under that place.
        beginTabTearOff(fakeLayoutModel, "h1", "node-1");
        expect(members()).toBe("a1 [h1]");

        // It has left: its right-hand neighbour is in front, for the chrome too, and it stays mounted.
        tabTearOffShown();
        expect(members()).toBe("a1 h1 [h2]");
        expect(chromeNodeModel?.shownBlockId?.()).toBe("h2");
        // The chrome itself stays on the layout's active tab: its header holds the pill being dragged.
        expect(chromeNodeModel?.activeBlockId?.()).toBe("h1");

        // The drag ends with the tab still in the pane.
        endTabTearOff();
        expect(members()).toBe("a1 [h1]");
        expect(chromeNodeModel?.shownBlockId?.()).toBe("h1");
    });

    it("shows the last tab when the dragged one was rightmost, and leaves a pane alone whose dragged tab was not in front", async () => {
        setView("a1", "agent");
        setView("a2", "agent");
        setStack(["a1", "a2"]);
        const rig = await mount("a2");
        beginTabTearOff(fakeLayoutModel, "a2", "node-1");
        tabTearOffShown();
        expect(members()).toBe("[a1] a2");
        endTabTearOff();

        rig.setActive("a1");
        beginTabTearOff(fakeLayoutModel, "a2", "node-1");
        tabTearOffShown();
        expect(members()).toBe("[a1] a2");
    });

    it("mounts nothing again when a tab's drag starts in a pane none of whose tabs is kept alive", async () => {
        setView("h1", "help");
        setView("h2", "help");
        setStack(["h1", "h2"]);
        await mount("h1");
        // Only the tab in front is mounted, as before, but in a slot of its own from the start: a pane with
        // several tabs is laid out for a tab's drag before one begins.
        expect(members()).toBe("[h1]");
        const block = document.querySelector('[data-testid^="block-"]');

        // The drag's start changes nothing. Mounted again here, the tab gave the chrome a new view model and
        // so a new header, and the pill being dragged was in the old one.
        beginTabTearOff(fakeLayoutModel, "h1", "node-1");
        expect(members()).toBe("[h1]");
        expect(document.querySelector('[data-testid^="block-"]')).toBe(block);
        tabTearOffShown();
        expect(members()).toBe("h1 [h2]");
        expect(document.querySelector('[data-testid^="block-"]')).toBe(block);
        endTabTearOff();
        expect(members()).toBe("[h1]");
        expect(document.querySelector('[data-testid^="block-"]')).toBe(block);
    });

    it("leaves a pane with one tab that is not kept alive without slots", async () => {
        setView("h1", "help");
        setStack(["h1"]);
        await mount("h1");
        expect(members()).toBe("");
        expect(document.querySelectorAll('[data-testid^="block-"]').length).toBe(1);
    });

    it("sizes the dragged tab's content for its landing place while it stands there, and for its pane otherwise", async () => {
        setView("a1", "agent");
        setView("a2", "agent");
        setStack(["a1", "a2"]);
        await mount("a1");
        const inner = () => blockNodeModels.get("a1")!.innerRect();
        expect(inner()).toEqual({ width: "400px - 3px", height: "300px - 3px" });
        const hide = showTabLanding("a1", document.createElement("div"));
        // No size of its own: the content fills the place.
        expect(inner()).toBeNull();
        expect(blockNodeModels.get("a2")!.innerRect()).not.toBeNull();
        hide();
        expect(inner()).toEqual({ width: "400px - 3px", height: "300px - 3px" });
    });

    it("is another pane's drag to every other pane", async () => {
        setView("a1", "agent");
        setView("a2", "agent");
        setStack(["a1", "a2"]);
        await mount("a1");
        beginTabTearOff(fakeLayoutModel, "a1", "node-2");
        tabTearOffShown();
        expect(members()).toBe("[a1] a2");
    });
});
