// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Characterisation tests for TabContent: what a tab renders (not found, the empty-tab identity panel,
 * the tile layout inside the tab's modal layer), the empty-tab widget menu, and what the layout's
 * close hooks do (`onNodeDelete`, and `beforeNodeDelete` with its busy-agent and "Leave site?"
 * prompts, parked agents, the shutdown log and the person-close note). They pin today's behaviour
 * (written against the unsplit component) so it can be split into named pieces without changing it.
 * The layout, the chrome, the stores, srv and the host API are mocked; the pane-close and leave
 * logic, the prompt queues and the DOM are real.
 */

import { cleanup, render } from "@solidjs/testing-library";
import type { JSX } from "solid-js";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type Contents = {
    renderContent: (nodeModel: unknown) => JSX.Element;
    renderPreview: (nodeModel: unknown) => JSX.Element;
    tabId: string;
    onNodeDelete: (data: TabLayoutData) => Promise<unknown>;
    beforeNodeDelete: (
        data: TabLayoutData,
        hooks?: { asking: (answer: Promise<boolean>) => Promise<boolean> }
    ) => Promise<boolean>;
    gapSizePx: number | undefined;
};

type Parked = { agentId: string; name: string };

const h = vi.hoisted(() => ({
    tile: null as null | { contents: unknown; tabAtom: unknown; getCursorPoint: unknown },
    // The questions asked in a pane (`askInPane`), oldest first, as the tests read and answer them.
    asked: [] as { kind: "busy" | "leave"; blockId: string; props: Record<string, unknown>; resolve: (close: boolean) => void }[],
    confirms: null as unknown as {
        busy: () => { busy: { blockId: string; name: string; turnActive: boolean; processCount: number }[] } | null;
        leave: () => { leaving: { blockId: string; site: string }[] } | null;
        answerBusy: (close: boolean) => void;
        answerLeave: (close: boolean) => void;
    },
    tabs: () => ({}) as Record<string, unknown>,
    settings: () => ({}) as Record<string, unknown>,
    fullConfig: undefined as unknown,
    objects: {} as Record<string, unknown>,
    aboutDetails: undefined as undefined | { version?: string; gitHash?: string },
    dev: false,
    hibernationCheck: undefined as undefined | ((id: string) => Promise<{ leave_prompt?: boolean } | undefined>),
    keepRunning: false,
    parked: {} as Record<string, Parked>,
    closing: false,
    sendLog: vi.fn<(line: string) => void>(),
    getCursorPoint: vi.fn(),
    showLive: vi.fn<(builder: () => unknown, e: unknown) => void>(),
    buildMenu: vi.fn<(widgets: unknown, settings: unknown, add: (def: unknown) => void) => unknown>(() => ["menu"]),
    createBlock: vi.fn<(def: unknown) => unknown>(),
    pushNotification: vi.fn<(n: Record<string, unknown>) => void>(),
    pushFlashError: vi.fn<(n: Record<string, unknown>) => void>(),
    closePane: vi.fn<(ids: string[]) => Promise<unknown>>(async () => ({ ok: true })),
    controllerStatus: vi.fn<(id: string) => Promise<{ turn_active?: boolean } | null>>(async () => null),
    processList: vi.fn<(client: unknown, req: { block_id: string }) => Promise<{ processes: unknown[] }>>(async () => ({
        processes: [],
    })),
    closeWithShutdownLog: vi.fn<(tabId: string, ids: string[], isAgent: (id: string) => boolean) => Promise<void>>(
        async () => {}
    ),
    noteKeepsRunning: vi.fn<(agents: Parked[]) => void>(),
    parkedAgentOf: vi.fn<(id: string) => Parked | undefined>(),
}));

// What the old tab-level prompts exposed: the oldest open question of each kind, and a way to answer it.
const askedOf = (kind: "busy" | "leave") => h.asked.find((q) => q.kind === kind);
const answerOldest = (kind: "busy" | "leave", close: boolean) => {
    const q = askedOf(kind);
    if (!q) return;
    h.asked.splice(h.asked.indexOf(q), 1);
    q.resolve(close);
};
h.confirms = {
    busy: () => (askedOf("busy")?.props as ReturnType<typeof h.confirms.busy>) ?? null,
    leave: () => (askedOf("leave")?.props as ReturnType<typeof h.confirms.leave>) ?? null,
    answerBusy: (close) => answerOldest("busy", close),
    answerLeave: (close) => answerOldest("leave", close),
};

vi.mock("@/layout/index", () => ({}));
vi.mock("@/layout/lib/layoutStack", () => ({
    effectiveStack: (data: { blockId: string; blockStack?: string[] }) =>
        data.blockStack?.length ? data.blockStack : [data.blockId],
}));
vi.mock("@/layout/lib/TileLayout.platform", () => ({
    TileLayout: (props: { contents: unknown; tabAtom: unknown; getCursorPoint: unknown }) => {
        h.tile = props;
        return <div data-testid="tile-layout" />;
    },
}));
vi.mock("@/app/block/block", () => ({
    Block: (props: { nodeModel: { blockId: string }; preview?: boolean }) => (
        <div data-testid="preview-block" data-block={props.nodeModel.blockId} data-preview={String(props.preview)} />
    ),
    resolveEffectiveViewType: (view: string) => ({ forge: "agent", webpane: "browser" })[view as "forge"] ?? view,
}));
vi.mock("@/app/tab/pane-leaf-chrome", () => ({
    PaneLeafChrome: (props: { nodeModel: { blockId: string } }) => (
        <div data-testid="leaf-chrome" data-block={props.nodeModel.blockId} />
    ),
}));
vi.mock("@/app/tab/pane-close-confirms", () => ({
    CloseBusyConfirm: () => null,
    LeaveSiteConfirm: () => null,
    askInPane: (_tabId: string, blockId: string, source: string, _subject: string[], _Component: unknown, props: Record<string, unknown>) =>
        new Promise<boolean>((resolve) =>
            h.asked.push({ kind: source === "close-busy" ? "busy" : "leave", blockId, props, resolve })
        ),
}));
vi.mock("@/element/ModalLayer", () => ({
    ModalLayer: (props: { scope: string; children?: JSX.Element }) => (
        <div data-testid="modal-layer" data-scope={props.scope}>
            {props.children}
        </div>
    ),
}));
vi.mock("@/app/store/contextmenu", () => ({ ContextMenuModel: { showLiveContextMenu: h.showLive } }));
vi.mock("@/app/window/action-widgets-config", () => ({ buildPaneWidgetMenuItems: h.buildMenu }));
vi.mock("@/store/block-layout-actions", () => ({ createBlock: h.createBlock }));
vi.mock("@/store/global", () => ({
    atoms: { fullConfigAtom: () => h.fullConfig, settingsAtom: () => h.settings() },
    getApi: () => ({
        sendLog: h.sendLog,
        getCursorPoint: h.getCursorPoint,
        browserPanes: { hibernationCheck: h.hibernationCheck },
        getAboutModalDetails: () => h.aboutDetails,
    }),
    getHostName: () => "box",
    getUserName: () => "me",
    isDev: () => h.dev,
    pushNotification: h.pushNotification,
}));
vi.mock("@/store/services", () => ({
    BlockService: { GetControllerStatus: h.controllerStatus },
    ObjectService: { ClosePane: h.closePane },
}));
vi.mock("@/store/mos", () => ({
    makeORef: (otype: string, id: string) => `${otype}:${id}`,
    getObjectValue: (oref: string) => h.objects[oref],
    getMuxObjectAtom: (oref: string) => () => h.tabs()[oref],
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { AgentProcessListCommand: h.processList } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: { name: "tab-rpc-client" } }));
vi.mock("@/app/store/flash-notifications", () => ({ pushFlashError: h.pushFlashError }));
vi.mock("@/app/view/agent/shutdown/close-with-log", () => ({
    closeWithShutdownLog: h.closeWithShutdownLog,
    isClosingWithLog: () => h.closing,
}));
vi.mock("@/app/view/agent/shutdown/keeps-running", () => ({
    agentsKeepRunning: () => h.keepRunning,
    noteKeepsRunning: h.noteKeepsRunning,
    parkedAgentOf: h.parkedAgentOf,
}));

import { takePersonClose } from "@/app/view/browser/leave-prompt";
import { TabContent } from "./tabcontent";

const [tabs, setTabs] = createSignal<Record<string, unknown>>({});
const [settings, setSettings] = createSignal<Record<string, unknown>>({});
h.tabs = tabs;
h.settings = settings;

function tabWith(blockids: string[] | undefined): Tab {
    return { blockids } as unknown as Tab;
}

function setBlock(blockId: string, meta: Record<string, unknown>): void {
    h.objects[`block:${blockId}`] = { meta };
}

function layoutData(...blockIds: string[]): TabLayoutData {
    return { blockId: blockIds[0], blockStack: blockIds } as unknown as TabLayoutData;
}

function contents(): Contents {
    return (h.tile as { contents: unknown }).contents as Contents;
}

function mount(tabId = "t1") {
    return render(() => <TabContent tabId={tabId} />);
}

function mountWithPanes(tabId = "t1") {
    setTabs({ [`tab:${tabId}`]: tabWith(["b1"]) });
    return mount(tabId);
}

async function flush(): Promise<void> {
    for (let i = 0; i < 10; i++) await Promise.resolve();
}

beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-01-01T00:00:00.000Z"));
    h.tile = null;
    h.asked.length = 0;
    h.fullConfig = undefined;
    h.objects = {};
    h.aboutDetails = undefined;
    h.dev = false;
    h.hibernationCheck = undefined;
    h.keepRunning = false;
    h.parked = {};
    h.closing = false;
    for (const id of ["a1", "a2", "w1", "w2", "w3", "t1b", "f1"]) takePersonClose(id);
    h.controllerStatus.mockReset().mockImplementation(async () => null);
    h.processList.mockReset().mockImplementation(async () => ({ processes: [] }));
    h.closePane.mockReset().mockImplementation(async () => ({ ok: true }));
    h.parkedAgentOf.mockReset().mockImplementation((id) => h.parked[id]);
    setTabs({});
    setSettings({});
});

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
    vi.useRealTimers();
});

describe("TabContent render", () => {
    it("says Tab Not Found when the tab has no data", () => {
        const { container, queryByTestId } = mount();
        expect(container.textContent).toBe("Tab Not Found");
        expect(container.querySelector(".centered-div")).not.toBeNull();
        expect(queryByTestId("tile-layout")).toBeNull();
        expect(container.querySelector(".empty-tab-logo")).toBeNull();
    });

    it("draws the root as a clipped, relative row on the workspace surface", () => {
        const { container } = mount();
        const root = container.firstElementChild as HTMLElement;
        expect(root.className).toBe(
            "flex flex-row flex-grow min-h-0 w-full items-center justify-center overflow-hidden relative"
        );
        expect(root.style.background).toBe("var(--workspace-surface)");
    });

    it("shows the identity panel and no tile layout for a tab with no panes", () => {
        setTabs({ "tab:t1": tabWith([]) });
        const { container, queryByTestId } = mount();
        expect(container.querySelector(".empty-tab-logo")).not.toBeNull();
        expect(queryByTestId("tile-layout")).toBeNull();
        expect(queryByTestId("modal-layer")).toBeNull();
    });

    it("treats a tab with no block list as empty", () => {
        setTabs({ "tab:t1": tabWith(undefined) });
        const { container } = mount();
        expect(container.querySelector(".empty-tab-logo")).not.toBeNull();
    });

    it("shows the tile layout in the tab's modal layer for a tab with panes", () => {
        const { container, getByTestId, queryByTestId } = mountWithPanes();
        const layer = getByTestId("modal-layer");
        expect(layer.getAttribute("data-scope")).toBe("tab");
        // The close questions are no longer drawn here: each is a modal of the pane being closed.
        expect(Array.from(layer.children).map((c) => c.getAttribute("data-testid"))).toEqual(["tile-layout"]);
        expect(container.querySelector(".empty-tab-logo")).toBeNull();
        expect(queryByTestId("tile-layout")).not.toBeNull();
    });

    it("hands the tile layout the tab atom, the cursor point reader and the contents", () => {
        mountWithPanes();
        const tile = h.tile as NonNullable<typeof h.tile>;
        expect(tile.getCursorPoint).toBe(h.getCursorPoint);
        expect((tile.tabAtom as () => unknown)()).toEqual(tabWith(["b1"]));
        expect(contents().tabId).toBe("t1");
    });

    it("hands the layout the same tab atom each time it reads it", () => {
        mountWithPanes();
        const tile = h.tile as NonNullable<typeof h.tile>;
        expect(tile.tabAtom).toBe(tile.tabAtom);
    });

    it("passes the tile gap setting to the layout and follows it", () => {
        setSettings({ "window:tilegapsize": 6 });
        mountWithPanes();
        expect(contents().gapSizePx).toBe(6);
        setSettings({ "window:tilegapsize": 12 });
        expect(contents().gapSizePx).toBe(12);
    });

    it("leaves the gap undefined when the setting is absent", () => {
        mountWithPanes();
        expect(contents().gapSizePx).toBeUndefined();
    });

    it("switches from the empty panel to the layout when the first pane arrives", () => {
        setTabs({ "tab:t1": tabWith([]) });
        const { container, queryByTestId } = mount();
        expect(queryByTestId("tile-layout")).toBeNull();
        setTabs({ "tab:t1": tabWith(["b1"]) });
        expect(queryByTestId("tile-layout")).not.toBeNull();
        expect(container.querySelector(".empty-tab-logo")).toBeNull();
    });

    it("renders a leaf through the pane chrome and a drag preview as a plain Block", () => {
        const { container } = mountWithPanes();
        const leaf = render(() => <>{contents().renderContent({ blockId: "b9" })}</>);
        expect(leaf.getByTestId("leaf-chrome").getAttribute("data-block")).toBe("b9");
        const preview = render(() => <>{contents().renderPreview({ blockId: "b8" })}</>);
        const block = preview.getByTestId("preview-block");
        expect(block.getAttribute("data-block")).toBe("b8");
        expect(block.getAttribute("data-preview")).toBe("true");
        expect(container.querySelector("[data-testid=preview-block]")).toBeNull();
    });

    it("looks the tab up by the tab id it is given", () => {
        setTabs({ "tab:t7": tabWith(["b1"]), "tab:t1": tabWith([]) });
        const { container } = mount("t7");
        expect(container.querySelector(".empty-tab-logo")).toBeNull();
        expect(contents().tabId).toBe("t7");
    });
});

describe("TabContent empty-tab identity", () => {
    beforeEach(() => {
        setTabs({ "tab:t1": tabWith([]) });
    });

    it("shows the logo, user@host, the version and the build", () => {
        h.aboutDetails = { version: "1.2.3", gitHash: "abc123" };
        const { container } = mount();
        const logo = container.querySelector("img.empty-tab-logo") as HTMLImageElement;
        expect(logo.alt).toBe("AgentMux");
        expect(logo.getAttribute("src")).toMatch(/^data:image\/svg\+xml,/);
        expect(logo.style.maxWidth).toBe("160px");
        expect(logo.style.maxHeight).toBe("160px");
        expect(container.textContent).toContain("me@box");
        expect(container.textContent).toContain("v1.2.3 (abc123)");
    });

    it("marks a dev build's hash", () => {
        h.dev = true;
        h.aboutDetails = { version: "1.2.3", gitHash: "abc123" };
        const { container } = mount();
        expect(container.textContent).toContain("v1.2.3 (dev-abc123)");
    });

    it("shows the version alone when there is no git hash", () => {
        h.aboutDetails = { version: "1.2.3" };
        const { container } = mount();
        expect(container.textContent).toContain("v1.2.3");
        expect(container.textContent).not.toContain("(");
    });

    it("shows no build for a dev build that has no git hash", () => {
        h.dev = true;
        h.aboutDetails = { version: "1.2.3" };
        const { container } = mount();
        expect(container.textContent).toContain("v1.2.3");
        expect(container.textContent).not.toContain("(");
        expect(container.textContent).not.toContain("dev-");
    });

    it("shows no version line, and no build, when there is no version", () => {
        h.aboutDetails = { gitHash: "abc123" };
        const { container } = mount();
        expect(container.textContent).toContain("me@box");
        expect(container.textContent).not.toContain("abc123");
        expect(container.textContent).not.toContain("v");
    });

    it("copes with the host having no about details", () => {
        h.aboutDetails = undefined;
        const { container } = mount();
        expect(container.textContent).toBe("me@box");
    });

    it("draws the panel faint, click-through and unselectable", () => {
        const { container } = mount();
        const panel = container.querySelector("img.empty-tab-logo")!.parentElement as HTMLElement;
        expect(panel.className).toBe("flex flex-col items-center gap-3 select-none pointer-events-none");
        expect(panel.style.opacity).toBe("0.4");
    });
});

describe("TabContent context menu", () => {
    function rightClick(target: Element) {
        const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
        target.dispatchEvent(event);
        return event;
    }

    it("opens the live widget menu on an empty tab and swallows the event", () => {
        setTabs({ "tab:t1": tabWith([]) });
        const parent = vi.fn();
        const { container } = render(() => (
            <div onContextMenu={parent}>
                <TabContent tabId="t1" />
            </div>
        ));
        const event = rightClick(container.querySelector(".empty-tab-logo")!);
        expect(event.defaultPrevented).toBe(true);
        expect(parent).not.toHaveBeenCalled();
        expect(h.showLive).toHaveBeenCalledTimes(1);
        expect(h.showLive.mock.calls[0][1]).toBe(event);
    });

    it("builds the menu from the widgets and settings, and adds a block for a chosen widget", () => {
        setTabs({ "tab:t1": tabWith([]) });
        const widgets = { w1: { label: "One" } };
        const config = { widgets, settings: { "widget:x": true } };
        h.fullConfig = config;
        const { container } = mount();
        rightClick(container.firstElementChild!);
        const builder = h.showLive.mock.calls[0][0];
        expect(builder()).toEqual(["menu"]);
        expect(h.buildMenu).toHaveBeenCalledTimes(1);
        expect(h.buildMenu.mock.calls[0][0]).toBe(widgets);
        expect(h.buildMenu.mock.calls[0][1]).toBe(config.settings);
        const blockdef = { meta: { view: "term" } };
        h.buildMenu.mock.calls[0][2](blockdef);
        expect(h.createBlock).toHaveBeenCalledWith(blockdef);
    });

    it("builds the menu from empty maps when there is no config yet", () => {
        setTabs({ "tab:t1": tabWith([]) });
        const { container } = mount();
        rightClick(container.firstElementChild!);
        h.showLive.mock.calls[0][0]();
        expect(h.buildMenu.mock.calls[0][0]).toEqual({});
        expect(h.buildMenu.mock.calls[0][1]).toEqual({});
    });

    it("re-reads the config each time the live menu is rebuilt", () => {
        setTabs({ "tab:t1": tabWith([]) });
        const { container } = mount();
        rightClick(container.firstElementChild!);
        const builder = h.showLive.mock.calls[0][0];
        h.fullConfig = { widgets: { a: {} }, settings: {} };
        builder();
        h.fullConfig = { widgets: { b: {} }, settings: {} };
        builder();
        expect(h.buildMenu.mock.calls[0][0]).toEqual({ a: {} });
        expect(h.buildMenu.mock.calls[1][0]).toEqual({ b: {} });
    });

    it("leaves the event alone when the tab has panes", () => {
        setTabs({ "tab:t1": tabWith(["b1"]) });
        const parent = vi.fn();
        const { container } = render(() => (
            <div onContextMenu={parent}>
                <TabContent tabId="t1" />
            </div>
        ));
        const event = rightClick(container.firstElementChild!.firstElementChild!);
        expect(event.defaultPrevented).toBe(false);
        expect(parent).toHaveBeenCalledTimes(1);
        expect(h.showLive).not.toHaveBeenCalled();
    });

    it("leaves the event alone when the tab is not found", () => {
        const parent = vi.fn();
        const { container } = render(() => (
            <div onContextMenu={parent}>
                <TabContent tabId="t1" />
            </div>
        ));
        const event = rightClick(container.firstElementChild!.firstElementChild!);
        expect(event.defaultPrevented).toBe(false);
        expect(parent).toHaveBeenCalledTimes(1);
        expect(h.showLive).not.toHaveBeenCalled();
    });

    it("follows the tab gaining panes", () => {
        setTabs({ "tab:t1": tabWith([]) });
        const { container } = mount();
        setTabs({ "tab:t1": tabWith(["b1"]) });
        rightClick(container.firstElementChild!);
        expect(h.showLive).not.toHaveBeenCalled();
    });
});

describe("TabContent onNodeDelete", () => {
    it("closes every block of the pane and returns srv's answer", async () => {
        mountWithPanes();
        h.closePane.mockResolvedValueOnce({ closed: 3 });
        const result = await contents().onNodeDelete(layoutData("b1", "b2", "b3"));
        expect(h.closePane).toHaveBeenCalledWith(["b1", "b2", "b3"]);
        expect(result).toEqual({ closed: 3 });
    });

    it("closes the one block of a pane with no stack", async () => {
        mountWithPanes();
        await contents().onNodeDelete({ blockId: "solo" } as unknown as TabLayoutData);
        expect(h.closePane).toHaveBeenCalledWith(["solo"]);
    });

    it("drops empty ids from the stack", async () => {
        mountWithPanes();
        await contents().onNodeDelete(layoutData("b1", "", "b2"));
        expect(h.closePane).toHaveBeenCalledWith(["b1", "b2"]);
    });

    it("traces the close in the host log", async () => {
        mountWithPanes();
        h.closePane.mockResolvedValueOnce({ closed: 1 });
        await contents().onNodeDelete(layoutData("b1", "b2"));
        expect(h.sendLog.mock.calls.map((c) => c[0])).toEqual([
            "[BUG-TRACE] onNodeDelete ENTER for blockIds: b1,b2",
            '[BUG-TRACE] onNodeDelete ClosePane returned: {"closed":1}',
        ]);
    });

    it("tells the person, logs and rethrows when srv fails the close", async () => {
        mountWithPanes();
        const err = new Error("srv down");
        h.closePane.mockRejectedValueOnce(err);
        await expect(contents().onNodeDelete(layoutData("b1"))).rejects.toBe(err);
        expect(h.sendLog).toHaveBeenLastCalledWith("[BUG-TRACE] onNodeDelete ERROR: Error: srv down");
        expect(h.pushFlashError).toHaveBeenCalledWith({
            id: "",
            icon: "triangle-exclamation",
            title: "Couldn't fully close the pane",
            message: "Error: srv down",
            expiration: Date.now() + 10_000,
        });
    });
});

describe("TabContent beforeNodeDelete", () => {
    it("lets a pane of non-agent, non-browser blocks close at once with no srv call", async () => {
        mountWithPanes();
        setBlock("t1b", { view: "term" });
        setBlock("e1", { view: "editor" });
        const ok = await contents().beforeNodeDelete(layoutData("t1b", "e1"));
        expect(ok).toBe(true);
        expect(h.controllerStatus).not.toHaveBeenCalled();
        expect(h.processList).not.toHaveBeenCalled();
        expect(h.closeWithShutdownLog).not.toHaveBeenCalled();
        expect(h.confirms!.busy()).toBeNull();
        expect(h.confirms!.leave()).toBeNull();
    });

    it("refuses without asking anything when the pane is already closing under its log", async () => {
        mountWithPanes();
        h.closing = true;
        setBlock("a1", { view: "agent" });
        const ok = await contents().beforeNodeDelete(layoutData("a1"));
        expect(ok).toBe(false);
        expect(h.controllerStatus).not.toHaveBeenCalled();
        expect(h.closeWithShutdownLog).not.toHaveBeenCalled();
        expect(h.parkedAgentOf).not.toHaveBeenCalled();
    });

    it("closes an idle agent pane in place under its shutdown log", async () => {
        mountWithPanes("tab9");
        setBlock("a1", { view: "agent" });
        const ok = await contents().beforeNodeDelete(layoutData("a1", "t1b"));
        expect(ok).toBe(false);
        expect(h.closeWithShutdownLog).toHaveBeenCalledTimes(1);
        const [tabId, ids, winds] = h.closeWithShutdownLog.mock.calls[0];
        expect(tabId).toBe("tab9");
        expect(ids).toEqual(["a1", "t1b"]);
        expect(winds("a1")).toBe(true);
        expect(winds("t1b")).toBe(false);
        expect(h.noteKeepsRunning).not.toHaveBeenCalled();
        expect(takePersonClose("a1")).toBe(false);
    });

    it("counts the legacy forge view as an agent pane", async () => {
        mountWithPanes();
        setBlock("f1", { view: "forge" });
        const ok = await contents().beforeNodeDelete(layoutData("f1"));
        expect(ok).toBe(false);
        expect(h.controllerStatus).toHaveBeenCalledWith("f1");
        expect(h.closeWithShutdownLog).toHaveBeenCalledTimes(1);
    });

    it("does not count a block whose view is missing or not a string as an agent", async () => {
        mountWithPanes();
        setBlock("n1", {});
        setBlock("n2", { view: 7 });
        const ok = await contents().beforeNodeDelete(layoutData("n1", "n2", "unknown"));
        expect(ok).toBe(true);
        expect(h.controllerStatus).not.toHaveBeenCalled();
    });

    it("asks srv for the turn and the processes of each agent, and not for the others", async () => {
        mountWithPanes();
        setBlock("a1", { view: "agent" });
        setBlock("a2", { view: "agent" });
        setBlock("t1b", { view: "term" });
        const pending = contents().beforeNodeDelete(layoutData("a1", "t1b", "a2"));
        await flush();
        expect(h.controllerStatus.mock.calls.map((c) => c[0])).toEqual(["a1", "a2"]);
        expect(h.processList.mock.calls.map((c) => c[1])).toEqual([{ block_id: "a1" }, { block_id: "a2" }]);
        expect(h.processList.mock.calls[0][0]).toEqual({ name: "tab-rpc-client" });
        expect(await pending).toBe(false);
    });

    describe("busy agents", () => {
        beforeEach(() => {
            mountWithPanes();
            setBlock("a1", { view: "agent", agentName: "Posa", agentId: "posa" });
        });

        it("asks once for the whole pane, listing the busy agents, and goes on when answered yes", async () => {
            h.controllerStatus.mockResolvedValue({ turn_active: true });
            h.processList.mockResolvedValue({ processes: [{}, {}] });
            const pending = contents().beforeNodeDelete(layoutData("a1"));
            await flush();
            expect(h.confirms!.busy()).toMatchObject({
                busy: [{ blockId: "a1", name: "Posa", turnActive: true, processCount: 2 }],
            });
            expect(h.closeWithShutdownLog).not.toHaveBeenCalled();
            h.confirms!.answerBusy(true);
            expect(await pending).toBe(false);
            expect(h.confirms!.busy()).toBeNull();
            expect(h.closeWithShutdownLog).toHaveBeenCalledTimes(1);
            expect(h.pushNotification).not.toHaveBeenCalled();
        });

        it("keeps the pane open with a quiet notice when answered no", async () => {
            h.controllerStatus.mockResolvedValue({ turn_active: true });
            h.processList.mockResolvedValue({ processes: [{}] });
            const pending = contents().beforeNodeDelete(layoutData("a1"));
            await flush();
            h.confirms!.answerBusy(false);
            expect(await pending).toBe(false);
            expect(h.closeWithShutdownLog).not.toHaveBeenCalled();
            expect(h.pushNotification).toHaveBeenCalledWith({
                icon: "fa-triangle-exclamation",
                title: "Not closed",
                message: "Posa — mid-turn, 1 process running",
                timestamp: "2026-01-01T00:00:00.000Z",
                type: "info",
                expiration: Date.now() + 5000,
            });
        });

        it("joins the notice lines of several busy agents with a semicolon", async () => {
            setBlock("a2", { view: "agent", agentName: "Maop" });
            h.controllerStatus.mockImplementation(async (id) => ({ turn_active: id === "a1" }));
            h.processList.mockImplementation(async (_client, req) => ({
                processes: req.block_id === "a2" ? [{}, {}] : [],
            }));
            const pending = contents().beforeNodeDelete(layoutData("a1", "a2"));
            await flush();
            h.confirms!.answerBusy(false);
            await pending;
            expect(h.pushNotification.mock.calls[0][0].message).toBe("Posa — mid-turn; Maop — 2 processes running");
        });

        it("does not list an agent that is neither mid-turn nor running processes", async () => {
            setBlock("a2", { view: "agent", agentName: "Maop" });
            h.controllerStatus.mockImplementation(async (id) => ({ turn_active: id === "a2" }));
            const pending = contents().beforeNodeDelete(layoutData("a1", "a2"));
            await flush();
            expect(h.confirms!.busy()!.busy.map((m) => m.blockId)).toEqual(["a2"]);
            h.confirms!.answerBusy(true);
            await pending;
        });

        it("names an agent by agentId when it has no agentName, else by a generic name", async () => {
            setBlock("a2", { view: "agent", agentId: "posa-2" });
            setBlock("a3", { view: "agent" });
            h.controllerStatus.mockResolvedValue({ turn_active: true });
            const pending = contents().beforeNodeDelete(layoutData("a2", "a3"));
            await flush();
            expect(h.confirms!.busy()!.busy.map((m) => m.name)).toEqual(["posa-2", "An agent"]);
            h.confirms!.answerBusy(true);
            await pending;
        });

        it("treats a failing status or process lookup as not busy", async () => {
            h.controllerStatus.mockRejectedValue(new Error("no status"));
            h.processList.mockRejectedValue(new Error("no list"));
            const ok = await contents().beforeNodeDelete(layoutData("a1"));
            expect(ok).toBe(false);
            expect(h.confirms!.busy()).toBeNull();
            expect(h.closeWithShutdownLog).toHaveBeenCalledTimes(1);
        });

        it("hands the person's pending answer to the layout's asking hook when it gives one", async () => {
            h.controllerStatus.mockResolvedValue({ turn_active: true });
            const asking = vi.fn<(answer: Promise<boolean>) => Promise<boolean>>(async () => true);
            const ok = await contents().beforeNodeDelete(layoutData("a1"), { asking });
            expect(asking).toHaveBeenCalledTimes(1);
            expect(ok).toBe(false);
            expect(h.closeWithShutdownLog).toHaveBeenCalledTimes(1);
            expect(h.confirms!.busy()).not.toBeNull();
        });

        it("uses the hook's answer, not the prompt's, to decide", async () => {
            h.controllerStatus.mockResolvedValue({ turn_active: true });
            const asking = vi.fn<(answer: Promise<boolean>) => Promise<boolean>>(async () => false);
            const ok = await contents().beforeNodeDelete(layoutData("a1"), { asking });
            expect(ok).toBe(false);
            expect(h.closeWithShutdownLog).not.toHaveBeenCalled();
            expect(h.pushNotification).toHaveBeenCalledTimes(1);
        });

        it("never calls the hook when nothing is busy", async () => {
            const asking = vi.fn<(answer: Promise<boolean>) => Promise<boolean>>(async () => true);
            await contents().beforeNodeDelete(layoutData("a1"), { asking });
            expect(asking).not.toHaveBeenCalled();
        });

        it("queues two prompts and answers them in order", async () => {
            h.controllerStatus.mockResolvedValue({ turn_active: true });
            setBlock("a2", { view: "agent", agentName: "Maop" });
            const first = contents().beforeNodeDelete(layoutData("a1"));
            await flush();
            const second = contents().beforeNodeDelete(layoutData("a2"));
            await flush();
            expect(h.confirms!.busy()!.busy[0].name).toBe("Posa");
            h.confirms!.answerBusy(false);
            expect(await first).toBe(false);
            expect(h.closeWithShutdownLog).not.toHaveBeenCalled();
            expect(h.confirms!.busy()!.busy[0].name).toBe("Maop");
            h.confirms!.answerBusy(true);
            expect(await second).toBe(false);
            expect(h.closeWithShutdownLog).toHaveBeenCalledTimes(1);
            expect(h.closeWithShutdownLog.mock.calls[0][1]).toEqual(["a2"]);
            expect(h.confirms!.busy()).toBeNull();
        });

        it("ignores an answer when no prompt is open", () => {
            expect(() => h.confirms!.answerBusy(true)).not.toThrow();
            expect(() => h.confirms!.answerLeave(true)).not.toThrow();
            expect(h.confirms!.busy()).toBeNull();
            expect(h.confirms!.leave()).toBeNull();
        });
    });

    describe("browser panes", () => {
        beforeEach(() => {
            mountWithPanes();
            setBlock("w1", { view: "browser", url: "https://portal.azure.com/x" });
            h.hibernationCheck = vi.fn(async () => ({ leave_prompt: true }));
        });

        it("asks 'Leave site?' in its own dialog naming the site", async () => {
            const pending = contents().beforeNodeDelete(layoutData("w1"));
            await flush();
            expect(h.confirms!.leave()).toMatchObject({ leaving: [{ blockId: "w1", site: "portal.azure.com" }] });
            expect(h.hibernationCheck).toHaveBeenCalledWith("w1");
            h.confirms!.answerLeave(true);
            expect(await pending).toBe(true);
            expect(h.confirms!.leave()).toBeNull();
        });

        it("notes the person's close of a browser pane once it is going ahead", async () => {
            const pending = contents().beforeNodeDelete(layoutData("w1", "t1b"));
            await flush();
            h.confirms!.answerLeave(true);
            await pending;
            expect(takePersonClose("w1")).toBe(true);
            expect(takePersonClose("t1b")).toBe(false);
        });

        it("keeps the pane open with a notice naming the site when answered no", async () => {
            const pending = contents().beforeNodeDelete(layoutData("w1"));
            await flush();
            h.confirms!.answerLeave(false);
            expect(await pending).toBe(false);
            expect(h.pushNotification.mock.calls[0][0]).toMatchObject({
                title: "Not closed",
                message: "portal.azure.com may have unsaved changes.",
                type: "info",
            });
            expect(takePersonClose("w1")).toBe(false);
        });

        it("does not ask when the page would not prompt", async () => {
            h.hibernationCheck = vi.fn(async () => ({ leave_prompt: false }));
            const ok = await contents().beforeNodeDelete(layoutData("w1"));
            expect(ok).toBe(true);
            expect(h.confirms!.leave()).toBeNull();
        });

        it("does not ask when the host cannot check", async () => {
            h.hibernationCheck = undefined;
            const ok = await contents().beforeNodeDelete(layoutData("w1"));
            expect(ok).toBe(true);
            expect(h.confirms!.leave()).toBeNull();
        });

        it("only takes a literal true for 'would prompt'", async () => {
            h.hibernationCheck = vi.fn(async () => ({}));
            const ok = await contents().beforeNodeDelete(layoutData("w1"));
            expect(ok).toBe(true);
            expect(h.confirms!.leave()).toBeNull();
        });

        it("names the page 'this page' when its address is not a URL", async () => {
            setBlock("w1", { view: "browser" });
            const pending = contents().beforeNodeDelete(layoutData("w1"));
            await flush();
            expect(h.confirms!.leave()!.leaving[0].site).toBe("this page");
            h.confirms!.answerLeave(true);
            await pending;
        });

        it("counts a browser pane through the view aliases", async () => {
            setBlock("w3", { view: "webpane", url: "https://example.net/" });
            const pending = contents().beforeNodeDelete(layoutData("w3"));
            await flush();
            expect(h.hibernationCheck).toHaveBeenCalledWith("w3");
            expect(h.confirms!.leave()!.leaving[0].site).toBe("example.net");
            h.confirms!.answerLeave(true);
            expect(await pending).toBe(true);
            expect(takePersonClose("w3")).toBe(true);
        });

        it("does not check blocks that are not browsers", async () => {
            setBlock("t1b", { view: "term" });
            await contents().beforeNodeDelete(layoutData("t1b"));
            expect(h.hibernationCheck).not.toHaveBeenCalled();
        });

        it("hands the layout's asking hook the leave answer too", async () => {
            const asking = vi.fn<(answer: Promise<boolean>) => Promise<boolean>>(async () => true);
            const ok = await contents().beforeNodeDelete(layoutData("w1"), { asking });
            expect(asking).toHaveBeenCalledTimes(1);
            expect(ok).toBe(true);
        });

        it("queues two leave prompts and answers them in order", async () => {
            setBlock("w2", { view: "browser", url: "https://example.org/" });
            const first = contents().beforeNodeDelete(layoutData("w1"));
            await flush();
            const second = contents().beforeNodeDelete(layoutData("w2"));
            await flush();
            expect(h.confirms!.leave()!.leaving[0].site).toBe("portal.azure.com");
            h.confirms!.answerLeave(true);
            expect(await first).toBe(true);
            expect(h.confirms!.leave()!.leaving[0].site).toBe("example.org");
            h.confirms!.answerLeave(false);
            expect(await second).toBe(false);
        });

        it("asks about busy agents first and the page only after that is answered yes", async () => {
            setBlock("a1", { view: "agent", agentName: "Posa" });
            h.controllerStatus.mockResolvedValue({ turn_active: true });
            const pending = contents().beforeNodeDelete(layoutData("a1", "w1"));
            await flush();
            expect(h.confirms!.busy()).not.toBeNull();
            expect(h.confirms!.leave()).toBeNull();
            expect(h.hibernationCheck).not.toHaveBeenCalled();
            h.confirms!.answerBusy(true);
            await flush();
            expect(h.confirms!.leave()).not.toBeNull();
            h.confirms!.answerLeave(true);
            expect(await pending).toBe(false);
            expect(h.closeWithShutdownLog).toHaveBeenCalledTimes(1);
            expect(takePersonClose("w1")).toBe(false);
        });

        it("does not ask about the page when the busy answer was no", async () => {
            setBlock("a1", { view: "agent", agentName: "Posa" });
            h.controllerStatus.mockResolvedValue({ turn_active: true });
            const pending = contents().beforeNodeDelete(layoutData("a1", "w1"));
            await flush();
            h.confirms!.answerBusy(false);
            expect(await pending).toBe(false);
            expect(h.hibernationCheck).not.toHaveBeenCalled();
            expect(h.confirms!.leave()).toBeNull();
        });
    });

    describe("agents that keep running", () => {
        beforeEach(() => {
            mountWithPanes();
            setBlock("a1", { view: "agent" });
            setBlock("a2", { view: "agent" });
            h.parked = { a1: { agentId: "posa", name: "Posa" } };
        });

        it("does not look for parked agents while the setting is off", async () => {
            h.keepRunning = false;
            await contents().beforeNodeDelete(layoutData("a1"));
            expect(h.parkedAgentOf).not.toHaveBeenCalled();
            expect(h.closeWithShutdownLog).toHaveBeenCalledTimes(1);
            expect(h.noteKeepsRunning).not.toHaveBeenCalled();
        });

        it("closes a parked agent's pane at once with a note that it runs on, no probe, no log", async () => {
            h.keepRunning = true;
            h.controllerStatus.mockResolvedValue({ turn_active: true });
            const ok = await contents().beforeNodeDelete(layoutData("a1"));
            expect(ok).toBe(true);
            expect(h.controllerStatus).not.toHaveBeenCalled();
            expect(h.closeWithShutdownLog).not.toHaveBeenCalled();
            expect(h.noteKeepsRunning).toHaveBeenCalledWith([{ agentId: "posa", name: "Posa" }]);
        });

        it("still probes and logs an agent that is not parked", async () => {
            h.keepRunning = true;
            const ok = await contents().beforeNodeDelete(layoutData("a1", "a2"));
            expect(ok).toBe(false);
            expect(h.controllerStatus.mock.calls.map((c) => c[0])).toEqual(["a2"]);
            const [, ids, winds] = h.closeWithShutdownLog.mock.calls[0];
            expect(ids).toEqual(["a1", "a2"]);
            expect(winds("a1")).toBe(false);
            expect(winds("a2")).toBe(true);
            expect(h.noteKeepsRunning).not.toHaveBeenCalled();
        });

        it("notes every parked agent, in pane order", async () => {
            h.keepRunning = true;
            h.parked = { a2: { agentId: "maop", name: "Maop" }, a1: { agentId: "posa", name: "Posa" } };
            await contents().beforeNodeDelete(layoutData("a1", "a2"));
            expect(h.noteKeepsRunning).toHaveBeenCalledWith([
                { agentId: "posa", name: "Posa" },
                { agentId: "maop", name: "Maop" },
            ]);
        });

        it("notes the parked agents and the person's browser close together", async () => {
            h.keepRunning = true;
            setBlock("w1", { view: "browser" });
            const ok = await contents().beforeNodeDelete(layoutData("a1", "w1"));
            expect(ok).toBe(true);
            expect(h.noteKeepsRunning).toHaveBeenCalledTimes(1);
            expect(takePersonClose("w1")).toBe(true);
            expect(takePersonClose("a1")).toBe(false);
        });

        it("notes nothing for a pane of other blocks, and calls the note with an empty list", async () => {
            h.keepRunning = true;
            setBlock("t1b", { view: "term" });
            await contents().beforeNodeDelete(layoutData("t1b"));
            expect(h.noteKeepsRunning).toHaveBeenCalledWith([]);
        });
    });
});

describe("where a pane's close questions are asked", () => {
    it("asks a busy pane's question in that pane, by the block it closes", async () => {
        mountWithPanes();
        setBlock("a1", { view: "agent" });
        h.controllerStatus.mockResolvedValue({ turn_active: true });
        void contents().beforeNodeDelete(layoutData("a1"));
        await flush();
        expect(h.asked.map((q) => [q.kind, q.blockId])).toEqual([["busy", "a1"]]);
    });

    it("asks a browser pane's leave question in that pane", async () => {
        mountWithPanes();
        setBlock("w1", { view: "browser" });
        h.hibernationCheck = vi.fn(async () => ({ leave_prompt: true }));
        void contents().beforeNodeDelete(layoutData("w1"));
        await flush();
        expect(h.asked.map((q) => [q.kind, q.blockId])).toEqual([["leave", "w1"]]);
    });

    it("asks two panes closed together at once, each in its own pane, not one behind the other", async () => {
        mountWithPanes();
        setBlock("a1", { view: "agent" });
        setBlock("a2", { view: "agent" });
        h.controllerStatus.mockResolvedValue({ turn_active: true });
        void contents().beforeNodeDelete(layoutData("a1"));
        void contents().beforeNodeDelete(layoutData("a2"));
        await flush();
        expect(h.asked.map((q) => q.blockId)).toEqual(["a1", "a2"]);
    });
});
