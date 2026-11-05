// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the window tab strip does, pinned as ordered logs: the tabs it lists (and the raw list it
 * gives the drag hooks), the optimistic select and its loop, the optimistic close with and
 * without the confirm modal, the close of the active tab, a failed close, the close request from
 * the keyboard, the legacy pinned-tab migration, and the scroll that keeps the active tab in
 * view. The tab itself, the confirm modal, the drag and tear-off hooks, the AgentMux logo and the
 * services are stand-ins that log; the strip, its selection rules and its close rules are real. A
 * refactor of TabBar must leave the committed snapshots as they are; regenerate with `vitest -u`
 * only for a change meant to behave differently.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    log: [] as string[],
    mac: false,
    gate: null as null | Promise<void>,
    closeFails: false,
    closeGate: null as null | Promise<void>,
    closeRequest: null as null | (() => void),
    dnd: null as null | { allTabIds: () => string[]; getWorkspace: () => any; refs: { tabBarScrollRef: () => HTMLElement | undefined } },
    tearEvents: null as null | { allTabIds: () => string[]; getScrollRef: () => HTMLElement | undefined },
    tearOffCreate: null as null | { getScrollRef: () => HTMLElement | undefined },
    wrappers: new Map<string, HTMLElement>(),
}));
const log = (line: string) => h.log.push(line);

const [workspace, setWorkspace] = createSignal<any>(null);
const [activeTab, setActiveTabSig] = createSignal("t1");
const [settings, setSettings] = createSignal<Record<string, unknown>>({});
const [creating, setCreating] = createSignal<string | null>(null);
const [moreLeft, setMoreLeft] = createSignal(false);
const [moreRight, setMoreRight] = createSignal(false);

vi.mock("@/app/hook/useWindowDrag.platform", () => ({ useWindowDrag: () => ({ dragProps: { "data-dragprops": "yes" } }) }));
vi.mock("@/app/window/logo-menu", () => ({ LogoMenu: () => <div class="AgentMux logo" /> }));
vi.mock("@/layout/index", () => ({ deleteLayoutModelForTab: (id: string) => log(`deleteLayoutModelForTab ${id}`) }));
vi.mock("@/store/config-signals", () => ({ settingsAtom: () => settings() }));
vi.mock("@/store/global", () => ({ atoms: { activeTabId: () => activeTab() } }));
vi.mock("@/store/rpc-api", () => ({
    RpcApi: { SetConfigCommand: async (_c: unknown, data: unknown) => log(`SetConfigCommand ${JSON.stringify(data)}`) },
}));
vi.mock("@/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/store/tab-actions", () => ({
    beginClosePromotion: (id: string) => {
        log(`beginClosePromotion ${id}`);
        return (closed: boolean) => log(`settlePromotion ${closed}`);
    },
    cancelTabCreation: () => {
        log("cancelTabCreation");
        setCreating(null);
    },
    creatingTabId: () => creating(),
    setActiveTab: async (id: string) => {
        log(`setActiveTab ${id}`);
        if (h.gate) await h.gate;
        setActiveTabSig(id);
        log(`setActiveTab ${id} applied`);
    },
}));
vi.mock("@/util/platformutil", () => ({ isMacOS: () => h.mac }));
vi.mock("@/app/store/services", () => ({
    WorkspaceService: {
        CloseTab: async (oid: string, tabId: string) => {
            log(`CloseTab ${oid} ${tabId}`);
            if (h.closeGate) await h.closeGate;
            if (h.closeFails) throw new Error("close failed");
            setWorkspace((w: any) => ({ ...w, tabids: w.tabids.filter((t: string) => t !== tabId) }));
        },
        UpdateTabIds: async (oid: string, tabIds: string[], pinned: string[]) =>
            log(`UpdateTabIds ${oid} ${JSON.stringify(tabIds)} ${JSON.stringify(pinned)}`),
    },
}));
vi.mock("./droppable-tab", () => ({
    DroppableTab: (p: any) => (
        <div
            class="tab-stub"
            data-tab-id={p.tabId}
            data-ws={p.workspaceId}
            data-active-id={p.activeTabId}
            data-active={String(p.isActive)}
            data-first={String(p.isFirst)}
            data-before-active={String(p.isBeforeActive)}
            data-count={String(p.allTabCount)}
            data-index={String(p.tabIndex)}
            data-ids={p.tabIds.join(",")}
        >
            <button class="select" onClick={p.onSelect} />
            <button class="close" onClick={p.onClose} />
        </div>
    ),
}));
vi.mock("./tab-close-confirm-modal", () => ({
    TabCloseConfirmModal: (p: any) => (
        <div class="modal" data-tab={p.tabId}>
            <button class="confirm" onClick={() => p.onConfirm(false)} />
            <button class="confirm-skip" onClick={() => p.onConfirm(true)} />
            <button class="cancel" onClick={() => p.onCancel()} />
        </div>
    ),
}));
vi.mock("./tab-close-request", () => ({
    registerTabCloseRequestHandler: (fn: () => void) => {
        h.closeRequest = fn;
        log("registerTabCloseRequestHandler");
        return () => log("unregisterTabCloseRequestHandler");
    },
}));
vi.mock("./tab-reorder", () => ({
    useTabDragAndDrop: (refs: any, getWorkspace: () => any, allTabIds: () => string[], tearOff: unknown) => {
        h.dnd = { refs, getWorkspace, allTabIds };
        log(`useTabDragAndDrop tearOff=${typeof tearOff}`);
    },
}));
vi.mock("./tab-tearoff-events", () => ({
    useTabTearOffEvents: (_getWorkspace: () => any, getScrollRef: () => HTMLElement | undefined, allTabIds: () => string[]) => {
        h.tearEvents = { allTabIds, getScrollRef };
        log("useTabTearOffEvents");
    },
}));
vi.mock("./tab-tearoff-rpc", () => ({
    createTearOffTabAtRelease: (_getWorkspace: () => any, getScrollRef: () => HTMLElement | undefined) => {
        h.tearOffCreate = { getScrollRef };
        log("createTearOffTabAtRelease");
        return () => {};
    },
}));
vi.mock("./tabbar-dnd", () => ({ tabWrapperRefs: h.wrappers }));
vi.mock("@/app/element/strip-reveal", () => ({
    revealInStrip: (_strip: HTMLElement, el: HTMLElement, last: boolean) => log(`revealInStrip ${el.id} last=${last}`),
}));
vi.mock("@/app/element/scroll-edges", () => ({
    createScrollEdges: () => {
        log("createScrollEdges");
        return { moreLeft, moreRight };
    },
}));

const wsOf = (tabids: string[], pinnedtabids: string[] = []) => ({ oid: "ws1", tabids, pinnedtabids });
const flush = async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
};

async function mount(ws: any, active = "t1") {
    setWorkspace(ws);
    setActiveTabSig(active);
    const { TabBar } = await import("./tabbar");
    return render(() => <TabBar workspace={workspace()} />);
}

const out = () => h.log.join("\n").replace(/\n+$/, "") + "\n";
const snap = (name: string) => `./__snapshots__/tabbar.behavior.${name}.txt`;
const dom = () => {
    const stubs = [...document.querySelectorAll<HTMLElement>(".tab-stub")].map((t) => {
        const d = t.dataset;
        return `${d.tabId}[active=${d.active} activeId=${d.activeId} first=${d.first} beforeActive=${d.beforeActive} count=${d.count} index=${d.index} ids=${d.ids}]`;
    });
    const scroll = document.querySelector(".tab-bar-scroll");
    log(
        `dom root=${document.querySelector(".tab-bar")?.getAttribute("data-dragprops")} AgentMux logo=${document.querySelectorAll(".AgentMux logo").length} ` +
            `separators=${document.querySelectorAll(".tab-separator").length} fill=${document.querySelectorAll(".tab-bar-fill").length} ` +
            `scroll="${scroll?.className}" drag=${scroll?.getAttribute("data-drag-region")} modal=${document.querySelector(".modal")?.getAttribute("data-tab") ?? "none"}`,
    );
    log(`tabs ${JSON.stringify(stubs)}`);
};
const click = (tabId: string, what: "select" | "close") =>
    fireEvent.click(document.querySelector(`.tab-stub[data-tab-id="${tabId}"] .${what}`)!);
const modalClick = (what: "confirm" | "confirm-skip" | "cancel") => fireEvent.click(document.querySelector(`.modal .${what}`)!);
const deferred = () => {
    let release!: () => void;
    const promise = new Promise<void>((r) => (release = r));
    return { promise, release };
};

beforeEach(() => {
    h.log.length = 0;
    h.mac = false;
    h.gate = null;
    h.closeFails = false;
    h.closeGate = null;
    h.closeRequest = null;
    h.dnd = null;
    h.tearEvents = null;
    h.tearOffCreate = null;
    h.wrappers.clear();
    setSettings({});
    setCreating(null);
    setMoreLeft(false);
    setMoreRight(false);
    vi.stubGlobal("requestAnimationFrame", (cb: () => void) => {
        cb();
        return 7;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => log(`cancelAnimationFrame ${id}`));
});
afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
});

describe("the strip", () => {
    it("lists the tabs with a separator between and around them, the AgentMux logo on the left, and gives the hooks the raw list", async () => {
        await mount(wsOf(["t1", "t2", "t3"]), "t2");
        dom();
        log(`dnd raw=${JSON.stringify(h.dnd!.allTabIds())} ws=${h.dnd!.getWorkspace().oid} scrollRef=${h.dnd!.refs.tabBarScrollRef()?.className}`);
        log(`tearEvents raw=${JSON.stringify(h.tearEvents!.allTabIds())} scrollRef=${h.tearEvents!.getScrollRef()?.className}`);
        log(`tearOff scrollRef=${h.tearOffCreate!.getScrollRef()?.className}`);
        log("-- unmount");
        cleanup();
        await expect(out()).toMatchFileSnapshot(snap("render"));
    });

    it("on macOS has no AgentMux logo and no leading separator", async () => {
        h.mac = true;
        await mount(wsOf(["t1", "t2"]), "t1");
        dom();
        await expect(out()).toMatchFileSnapshot(snap("render-mac"));
    });

    it("folds legacy pinned tabs in front of the others and migrates them on the server once", async () => {
        await mount(wsOf(["t1"], ["p1", "p2"]), "p1");
        dom();
        await flush();
        await expect(out()).toMatchFileSnapshot(snap("pinned"));
    });

    it("renders nothing without a workspace, though the drag hooks are already set up", async () => {
        await mount(null);
        dom();
        await expect(out()).toMatchFileSnapshot(snap("no-workspace"));
    });

    it("marks the strip when more tabs are a scroll away on either side", async () => {
        await mount(wsOf(["t1"]));
        setMoreLeft(true);
        log(`scroll="${document.querySelector(".tab-bar-scroll")?.className}"`);
        setMoreRight(true);
        log(`scroll="${document.querySelector(".tab-bar-scroll")?.className}"`);
        setMoreLeft(false);
        log(`scroll="${document.querySelector(".tab-bar-scroll")?.className}"`);
        await expect(out()).toMatchFileSnapshot(snap("edges"));
    });
});

describe("selecting a tab", () => {
    it("highlights the clicked tab at once, drives the server to the newest click, and hands the highlight back", async () => {
        await mount(wsOf(["t1", "t2", "t3"]), "t1");
        const gate = deferred();
        h.gate = gate.promise;
        click("t2", "select");
        await flush();
        log("-- t2 pending");
        dom();
        click("t3", "select");
        await flush();
        log("-- t3 clicked while t2 pending");
        dom();
        click("t3", "select");
        log("-- t3 clicked again (already displayed): no-op");
        gate.release();
        await flush();
        log("-- settled");
        dom();
        await expect(out()).toMatchFileSnapshot(snap("select-loop"));
    });

    it("a click on the displayed tab does nothing; a tab being created is outranked by a click and cancelled", async () => {
        await mount(wsOf(["t1", "t2", "t3"]), "t1");
        click("t1", "select");
        await flush();
        log("-- click on the displayed tab done");
        setCreating("t3");
        log("-- creating t3");
        dom();
        const gate = deferred();
        h.gate = gate.promise;
        click("t2", "select");
        await flush();
        log("-- click on t2");
        dom();
        gate.release();
        await flush();
        dom();
        await expect(out()).toMatchFileSnapshot(snap("select-creating"));
    });
});

describe("closing a tab", () => {
    it("hides the tab at once and closes it on the server, keeping the raw list for the hooks", async () => {
        await mount(wsOf(["t1", "t2", "t3"]), "t1");
        const gate = deferred();
        h.closeGate = gate.promise;
        click("t2", "close");
        await flush();
        log("-- t2 pending");
        dom();
        log(`dnd raw=${JSON.stringify(h.dnd!.allTabIds())}`);
        gate.release();
        await flush();
        log("-- t2 closed");
        dom();
        await expect(out()).toMatchFileSnapshot(snap("close-plain"));
    });

    it("closing the active tab promotes the neighbor and settles the promotion", async () => {
        await mount(wsOf(["t1", "t2", "t3"]), "t2");
        const gate = deferred();
        h.closeGate = gate.promise;
        click("t2", "close");
        await flush();
        log("-- t2 pending");
        dom();
        gate.release();
        await flush();
        log("-- closed");
        dom();
        await expect(out()).toMatchFileSnapshot(snap("close-active"));
    });

    it("a close the server refuses puts the tab back and settles the promotion as not closed", async () => {
        await mount(wsOf(["t1", "t2"]), "t1");
        h.closeFails = true;
        click("t1", "close");
        await flush();
        dom();
        await expect(out()).toMatchFileSnapshot(snap("close-fails"));
    });

    it("never starts a close of the last visible tab while the others are closing", async () => {
        await mount(wsOf(["t1", "t2", "t3"]), "t1");
        const gate = deferred();
        h.closeGate = gate.promise;
        click("t2", "close");
        click("t3", "close");
        click("t1", "close");
        await flush();
        log("-- t2 and t3 closing, t1 refused");
        dom();
        gate.release();
        await flush();
        dom();
        await expect(out()).toMatchFileSnapshot(snap("close-last"));
    });

    it("the keyboard's close request closes the active tab", async () => {
        await mount(wsOf(["t1", "t2"]), "t2");
        h.closeRequest!();
        await flush();
        dom();
        log("-- unmount");
        cleanup();
        await expect(out()).toMatchFileSnapshot(snap("close-request"));
    });
});

describe("closing with the confirm modal", () => {
    beforeEach(() => setSettings({ "tab:confirmclose": true }));

    it("asks first, once per tab; Cancel leaves everything as it was", async () => {
        await mount(wsOf(["t1", "t2", "t3"]), "t1");
        click("t2", "close");
        log("-- asked");
        dom();
        click("t2", "close");
        log("-- asked again for the same tab");
        dom();
        modalClick("cancel");
        log("-- cancelled");
        dom();
        await expect(out()).toMatchFileSnapshot(snap("confirm-cancel"));
    });

    it("Confirm closes the modal and hides the tab in the same click", async () => {
        await mount(wsOf(["t1", "t2", "t3"]), "t1");
        const gate = deferred();
        h.closeGate = gate.promise;
        click("t2", "close");
        modalClick("confirm");
        log("-- confirmed");
        dom();
        gate.release();
        await flush();
        dom();
        await expect(out()).toMatchFileSnapshot(snap("confirm-yes"));
    });

    it("Confirm with Don't ask again writes the setting too", async () => {
        await mount(wsOf(["t1", "t2"]), "t1");
        click("t2", "close");
        modalClick("confirm-skip");
        await flush();
        dom();
        await expect(out()).toMatchFileSnapshot(snap("confirm-skip"));
    });
});

describe("keeping the active tab in view", () => {
    it("reveals the active tab after a frame, the last one to the end, and cancels the frame when it changes", async () => {
        for (const id of ["t1", "t2", "t3"]) {
            const el = document.createElement("div");
            el.id = `wrap-${id}`;
            h.wrappers.set(id, el);
        }
        await mount(wsOf(["t1", "t2", "t3"]), "t1");
        log("-- mounted");
        setActiveTabSig("t3");
        log("-- active t3");
        setActiveTabSig("t2");
        log("-- active t2");
        h.wrappers.delete("t1");
        setActiveTabSig("t1");
        log("-- active t1 (no wrapper)");
        log("-- unmount");
        cleanup();
        await expect(out()).toMatchFileSnapshot(snap("reveal"));
    });
});
