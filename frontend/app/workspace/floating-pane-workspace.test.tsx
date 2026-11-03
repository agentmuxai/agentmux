// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Characterisation tests for FloatingPaneWorkspace: what the floating window does with the host
 * API when the user drags its header, drags an edge, double-clicks, or empties its tab. They pin
 * today's behaviour (written against the unsplit component) so the component can be split into
 * named pieces without changing it. The host API, the platform checks, the muxobj store and the
 * redock arming state machine are mocked; the DOM events, the edge-band geometry, the header-press
 * rule and the redock guard are real.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => {
    const unresolved = <T,>() => {
        let resolve!: (v: T) => void;
        const promise = new Promise<T>((r) => (resolve = r));
        return { promise, resolve };
    };
    return {
        unresolved,
        platform: { windows: false, mac: true, linux: false, wayland: false },
        maximized: false,
        blockIds: ["b1"] as string[],
        listeners: {} as Record<string, (payload: Record<string, unknown>) => void>,
        reapply: vi.fn<(...a: unknown[]) => void>(),
        redockFloaterPane: vi.fn<(...a: unknown[]) => Promise<unknown>>(async () => {}),
        reload: vi.fn<(...a: unknown[]) => Promise<unknown>>(),
        arming: {
            reset: vi.fn(),
            isArmed: vi.fn(() => false),
            target: vi.fn((): string | null => null),
            sample: vi.fn<(s: unknown) => { clearIndicator: boolean }>(() => ({ clearIndicator: false })),
        },
        api: null as unknown as ReturnType<typeof makeApi>,
    };
});

function makeApi() {
    return {
        closeWindowByLabel: vi.fn<(label: string) => Promise<void>>(async () => {}),
        listen: vi.fn<(event: string, handler: (payload: Record<string, unknown>) => void) => Promise<() => void>>(
            async (event, handler) => {
                h.listeners[event] = handler;
                return () => {
                    delete h.listeners[event];
                };
            }
        ),
        windows: {
            getPosition: vi.fn<(label: string) => Promise<{ x: number; y: number }>>(async () => ({ x: 100, y: 200 })),
            isMaximized: vi.fn<(label: string) => Promise<boolean | null>>(async () => false),
            setPosition: vi.fn<(label: string, x: number, y: number) => Promise<void>>(async () => {}),
            getRect: vi.fn<(label: string) => Promise<{ x: number; y: number; width: number; height: number }>>(
                async () => ({ x: 100, y: 200, width: 800, height: 600 })
            ),
            setRect: vi.fn<(label: string, r: unknown) => Promise<void>>(async () => {}),
            startDrag: vi.fn<(label: string) => Promise<void>>(async () => {}),
            toggleFloatingMaximize: vi.fn<(label: string, blockId: string) => Promise<void>>(async () => {}),
            updateFloatingRedockHover: vi.fn<(a: unknown) => Promise<{ target_label: string | null }>>(async () => ({
                target_label: null,
            })),
            clearFloatingRedockHover: vi.fn<() => Promise<void>>(async () => {}),
            getFloatingRedockTarget: vi.fn<(label: string) => Promise<{ block_id: string; dir: number }>>(
                async () => ({ block_id: "ghost-block", dir: 2 })
            ),
            resolveWindowAtCursor: vi.fn<(a: unknown) => Promise<{ label: string | null; window_id: string | null }>>(
                async () => ({ label: null, window_id: null })
            ),
        },
    };
}

vi.mock("@/util/platformutil", () => ({
    isWindows: () => h.platform.windows,
    isMacOS: () => h.platform.mac,
    isLinux: () => h.platform.linux,
}));
vi.mock("@/app/host/window-system", () => ({ isNativeWayland: () => h.platform.wayland }));
vi.mock("@/app/element/errorboundary", () => ({ ErrorBoundary: (p: { children: unknown }) => p.children }));
vi.mock("@/app/element/quickelems", () => ({
    CenteredDiv: (p: { children: unknown }) => {
        const el = document.createElement("div");
        el.className = "centered";
        el.append(String(p.children));
        return el;
    },
}));
vi.mock("@/app/modals/modalsrenderer", () => ({ ModalsRenderer: () => null }));
vi.mock("@/app/tab/tabcontent", () => ({
    TabContent: (p: { tabId: string }) => {
        const el = document.createElement("div");
        el.className = "tab-content";
        el.dataset.tab = p.tabId;
        el.innerHTML =
            '<div data-role="block-header"><button class="hbtn"></button><div class="gap"></div></div><div class="body"></div>';
        return el;
    },
}));
vi.mock("@/store/global", async () => {
    const { createSignal } = await import("solid-js");
    const [activeTabId, setTab] = createSignal<string | null>("tab1");
    const [workspace, setWs] = createSignal<{ oid: string } | null>({ oid: "ws1" });
    return { atoms: { activeTabId, workspace }, getApi: () => h.api, __set: { setTab, setWs } };
});
vi.mock("@/store/mos", async () => {
    const { createSignal } = await import("solid-js");
    const [tab, setTab] = createSignal<{ blockids: string[] } | null>({ blockids: ["b1"] });
    return {
        makeORef: (type: string, id: string) => `${type}:${id}`,
        useMuxObjectValue: () => [tab],
        getObjectValue: () => ({ meta: {} }),
        reloadMuxObject: (...a: unknown[]) => h.reload(...a),
        __setTab: setTab,
    };
});
vi.mock("@/app/block/floating-maximize", () => ({ isFloatingMaximized: () => h.maximized }));
vi.mock("@/app/block/floating-ontop", () => ({ useReapplyFloatingOnTop: (...a: unknown[]) => h.reapply(...a) }));
vi.mock("./floater-panes", () => ({
    floaterPaneBlockId: (tabId: string) => (tabId ? h.blockIds[0] : undefined),
    floaterPanes: () => [{ blockId: "b1", members: ["b1"], activeBlockId: "b1" }],
    redockFloaterPane: (...a: unknown[]) => h.redockFloaterPane(...a),
}));
vi.mock("./redock-arming", () => ({ createRedockArming: () => h.arming }));

import * as Global from "@/store/global";
import * as MOS from "@/store/mos";
import { FLOATER_RESIZE_CURSORS } from "./floater-resize";
import { FloatingPaneWorkspace } from "./floating-pane-workspace";

const LABEL = "floating-test";
const globalSet = (Global as unknown as { __set: { setTab: (v: string | null) => void; setWs: (v: { oid: string } | null) => void } })
    .__set;
const setTab = globalSet.setTab;
const setWs = globalSet.setWs;
const setTabValue = (MOS as unknown as { __setTab: (v: { blockids: string[] } | null) => void }).__setTab;

const flush = () => new Promise<void>((r) => setTimeout(r, 0));

function mount(label: string = LABEL) {
    window.history.replaceState({}, "", label ? `/?windowLabel=${label}` : "/");
    const view = render(() => <FloatingPaneWorkspace />);
    const q = (s: string) => view.container.querySelector<HTMLElement>(s)!;
    return { ...view, gap: q(".gap"), button: q(".hbtn"), body: q(".body") };
}

const mouse = (type: string, init: MouseEventInit = {}) =>
    new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, ...init });

beforeEach(() => {
    h.platform = { windows: false, mac: true, linux: false, wayland: false };
    h.maximized = false;
    h.blockIds = ["b1"];
    h.listeners = {};
    h.api = makeApi();
    h.reapply.mockReset();
    h.redockFloaterPane.mockReset().mockResolvedValue(undefined);
    h.reload.mockReset();
    h.arming.reset.mockReset();
    h.arming.isArmed.mockReset().mockReturnValue(false);
    h.arming.target.mockReset().mockReturnValue(null);
    h.arming.sample.mockReset().mockReturnValue({ clearIndicator: false });
    setTab("tab1");
    setWs({ oid: "ws1" });
    setTabValue({ blockids: ["b1"] });
});

afterEach(() => {
    cleanup();
    delete document.documentElement.dataset.floaterEdge;
});

describe("FloatingPaneWorkspace: what it renders", () => {
    it("shows the tab's content once the workspace and tab are known", () => {
        const { container } = mount();
        expect(container.querySelector(".tab-content")?.getAttribute("data-tab")).toBe("tab1");
    });

    it("shows a loading line until there is a workspace and a tab", () => {
        setTab(null);
        const { container } = mount();
        expect(container.querySelector(".tab-content")).toBeNull();
        expect(container.querySelector(".centered")?.textContent).toBe("Loading pane…");
        setTab("tab1");
        expect(container.querySelector(".tab-content")?.getAttribute("data-tab")).toBe("tab1");
    });

    it("re-applies a stored always-on-top tack, with the window label and the pane's first block", () => {
        mount();
        expect(h.reapply).toHaveBeenCalledTimes(1);
        const [label, blockId] = h.reapply.mock.calls[0] as [() => string, () => string | undefined];
        expect(label()).toBe(LABEL);
        expect(blockId()).toBe("b1");
    });
});

describe("FloatingPaneWorkspace: closing when the tab empties", () => {
    it("closes its own window when the tab goes from having blocks to none", () => {
        mount();
        expect(h.api.closeWindowByLabel).not.toHaveBeenCalled();
        setTabValue({ blockids: [] });
        expect(h.api.closeWindowByLabel).toHaveBeenCalledWith(LABEL);
    });

    it("does not close on an empty tab that never had a block (the initial load)", () => {
        setTabValue({ blockids: [] });
        mount();
        expect(h.api.closeWindowByLabel).not.toHaveBeenCalled();
    });

    it("does not close while a redock is in flight, and closes when it lets go", async () => {
        const { holdRedock } = await import("./floating-redock-guard");
        mount();
        const gate = h.unresolved<void>();
        const held = holdRedock(() => gate.promise);
        setTabValue({ blockids: [] });
        expect(h.api.closeWindowByLabel).not.toHaveBeenCalled();
        gate.resolve();
        await held;
        expect(h.api.closeWindowByLabel).toHaveBeenCalledWith(LABEL);
    });

    it("has no window to close without a label", () => {
        mount("");
        setTabValue({ blockids: [] });
        expect(h.api.closeWindowByLabel).not.toHaveBeenCalled();
    });
});

describe("FloatingPaneWorkspace: edge resize", () => {
    const down = (el: Element, x: number, y: number, screenX = 1000, screenY = 500) =>
        el.dispatchEvent(mouse("pointerdown", { clientX: x, clientY: y, screenX, screenY }));
    const move = (el: Element, screenX: number, screenY: number, x = 2, y = 400) =>
        el.dispatchEvent(mouse("pointermove", { clientX: x, clientY: y, screenX, screenY }));

    it("takes a press in the left edge band, reads the window rect, and sends the resized rect on move", async () => {
        const { body } = mount();
        const ev = mouse("pointerdown", { clientX: 2, clientY: 400, screenX: 1000, screenY: 500 });
        body.dispatchEvent(ev);
        expect(ev.defaultPrevented).toBe(true);
        expect(h.api.windows.getRect).toHaveBeenCalledWith(LABEL);
        await flush();
        move(body, 990, 500);
        expect(h.api.windows.setRect).toHaveBeenCalledWith(LABEL, { x: 90, y: 200, width: 810, height: 600 });
    });

    it("keeps the minimum size and pins the opposite edge when a left edge is dragged past it", async () => {
        const { body } = mount();
        down(body, 2, 400);
        await flush();
        move(body, 1700, 500);
        expect(h.api.windows.setRect).toHaveBeenCalledWith(LABEL, { x: 660, y: 200, width: 240, height: 600 });
    });

    it("resizes from the bottom-right corner in both axes", async () => {
        const { body } = mount();
        down(body, 1022, 766);
        await flush();
        move(body, 1030, 520, 1022, 766);
        expect(h.api.windows.setRect).toHaveBeenCalledWith(LABEL, { x: 100, y: 200, width: 830, height: 620 });
    });

    it("sends only the last rect when moves arrive while one is still in flight", async () => {
        const first = h.unresolved<void>();
        const { body } = mount();
        h.api.windows.setRect.mockImplementationOnce(() => first.promise);
        down(body, 2, 400);
        await flush();
        move(body, 990, 500);
        move(body, 980, 500);
        move(body, 970, 500);
        expect(h.api.windows.setRect).toHaveBeenCalledTimes(1);
        first.resolve();
        await flush();
        expect(h.api.windows.setRect).toHaveBeenCalledTimes(2);
        expect(h.api.windows.setRect).toHaveBeenLastCalledWith(LABEL, { x: 70, y: 200, width: 830, height: 600 });
    });

    it("ignores a press away from the edges, a non-primary button, and a press on a band control", () => {
        const { body, button } = mount();
        down(body, 500, 400);
        body.dispatchEvent(mouse("pointerdown", { button: 2, clientX: 2, clientY: 400 }));
        down(button, 500, 400);
        expect(h.api.windows.getRect).not.toHaveBeenCalled();
    });

    it("shows the edge's cursor on hover, and clears it when the press ends", async () => {
        const { body } = mount();
        move(body, 0, 0, 2, 400);
        expect(document.documentElement.dataset.floaterEdge).toBe(FLOATER_RESIZE_CURSORS[1]);
        move(body, 0, 0, 500, 400);
        expect(document.documentElement.dataset.floaterEdge).toBeUndefined();
        down(body, 2, 400);
        await flush();
        expect(document.documentElement.dataset.floaterEdge).toBe(FLOATER_RESIZE_CURSORS[1]);
        body.dispatchEvent(mouse("pointerup", {}));
        expect(document.documentElement.dataset.floaterEdge).toBeUndefined();
        move(body, 900, 500);
        expect(h.api.windows.setRect).not.toHaveBeenCalled();
    });

    it("gives up when the host returns an empty rect", async () => {
        const { body } = mount();
        h.api.windows.getRect.mockResolvedValueOnce({ x: 0, y: 0, width: 0, height: 0 });
        down(body, 2, 400);
        await flush();
        move(body, 990, 500);
        expect(h.api.windows.setRect).not.toHaveBeenCalled();
    });

    it("does not resize a window whose label is not a floater's", () => {
        const { body } = mount("main-1");
        down(body, 2, 400);
        expect(h.api.windows.getRect).not.toHaveBeenCalled();
    });
});

describe("FloatingPaneWorkspace: header drag on macOS and Linux (JS-driven)", () => {
    const press = (el: Element, init: MouseEventInit = {}) => {
        const ev = mouse("mousedown", { screenX: 500, screenY: 300, clientX: 50, clientY: 10, ...init });
        el.dispatchEvent(ev);
        return ev;
    };
    const moveTo = (screenX: number, screenY: number, init: MouseEventInit = {}) =>
        document.dispatchEvent(mouse("mousemove", { screenX, screenY, buttons: 1, ...init }));
    const release = (screenX = 700, screenY = 400) => document.dispatchEvent(mouse("mouseup", { screenX, screenY }));

    it("blocks the browser's own drag, asks the host where the window is, and moves it by the cursor's delta", async () => {
        const { gap } = mount();
        const ev = press(gap);
        expect(ev.defaultPrevented).toBe(true);
        expect(h.api.windows.getPosition).toHaveBeenCalledWith(LABEL);
        expect(h.api.windows.isMaximized).toHaveBeenCalledWith(LABEL);
        await flush();
        moveTo(510, 330);
        expect(h.api.windows.setPosition).toHaveBeenCalledWith(LABEL, 110, 230);
    });

    it("works the same on Linux", async () => {
        h.platform = { windows: false, mac: false, linux: true, wayland: false };
        const { gap } = mount();
        press(gap);
        await flush();
        moveTo(490, 280);
        expect(h.api.windows.setPosition).toHaveBeenCalledWith(LABEL, 90, 180);
    });

    it("leaves a press on a header control, a press outside the header, and a non-primary press alone", async () => {
        const { button, body, gap } = mount();
        const onButton = press(button);
        const onBody = press(body);
        const rightClick = press(gap, { button: 2 });
        await flush();
        expect(onButton.defaultPrevented).toBe(false);
        expect(onBody.defaultPrevented).toBe(false);
        expect(rightClick.defaultPrevented).toBe(false);
        expect(h.api.windows.getPosition).not.toHaveBeenCalled();
    });

    it("sends one position at a time and only the last of those that arrived meanwhile", async () => {
        const first = h.unresolved<void>();
        const { gap } = mount();
        press(gap);
        await flush();
        h.api.windows.setPosition.mockImplementationOnce(() => first.promise);
        moveTo(510, 300);
        moveTo(520, 300);
        moveTo(530, 300);
        expect(h.api.windows.setPosition).toHaveBeenCalledTimes(1);
        first.resolve();
        await flush();
        expect(h.api.windows.setPosition).toHaveBeenCalledTimes(2);
        expect(h.api.windows.setPosition).toHaveBeenLastCalledWith(LABEL, 130, 200);
    });

    it("drops a position left waiting from an earlier drag when a new press starts", async () => {
        const first = h.unresolved<void>();
        const { gap } = mount();
        press(gap);
        await flush();
        h.api.windows.setPosition.mockImplementationOnce(() => first.promise);
        moveTo(510, 300);
        moveTo(520, 300);
        release();
        press(gap);
        first.resolve();
        await flush();
        expect(h.api.windows.setPosition).toHaveBeenCalledTimes(1);
    });

    it("catches up with the cursor's motion during the position request", async () => {
        const pos = h.unresolved<{ x: number; y: number }>();
        const { gap } = mount();
        h.api.windows.getPosition.mockImplementationOnce(() => pos.promise);
        press(gap);
        moveTo(520, 310);
        expect(h.api.windows.setPosition).not.toHaveBeenCalled();
        pos.resolve({ x: 100, y: 200 });
        await flush();
        expect(h.api.windows.setPosition).toHaveBeenCalledWith(LABEL, 120, 210);
    });

    it("asks the host for the drop target under the cursor and feeds the answer to the arming state", async () => {
        const { gap } = mount();
        h.api.windows.updateFloatingRedockHover.mockResolvedValue({ target_label: "main-1" });
        press(gap);
        await flush();
        moveTo(510, 330);
        expect(h.api.windows.updateFloatingRedockHover).toHaveBeenCalledWith({
            source_label: LABEL,
            x: 510,
            y: 330,
        });
        await flush();
        expect(h.arming.sample).toHaveBeenCalledWith(expect.objectContaining({ target: "main-1", x: 510, y: 330 }));
    });

    it("clears the drop indicator when the arming state says to", async () => {
        const { gap } = mount();
        h.arming.sample.mockReturnValue({ clearIndicator: true });
        press(gap);
        await flush();
        moveTo(510, 330);
        await flush();
        expect(h.api.windows.clearFloatingRedockHover).toHaveBeenCalled();
    });

    it("on release tells the panes the drag ended, clears the hover, and does not redock a plain click", async () => {
        const { gap } = mount();
        const ended = vi.fn();
        window.addEventListener("floating-pane-js-drag-ended", ended);
        h.arming.isArmed.mockReturnValue(true);
        press(gap);
        await flush();
        release();
        window.removeEventListener("floating-pane-js-drag-ended", ended);
        expect(ended).toHaveBeenCalledTimes(1);
        expect((ended.mock.calls[0][0] as CustomEvent).detail).toEqual({ label: LABEL });
        expect(h.api.windows.clearFloatingRedockHover).toHaveBeenCalled();
        await flush();
        expect(h.api.windows.resolveWindowAtCursor).not.toHaveBeenCalled();
    });

    it("on release after a move, does not look for a window to dock into unless the arming state is armed", async () => {
        const { gap } = mount();
        press(gap);
        await flush();
        moveTo(510, 330);
        release();
        await flush();
        expect(h.api.windows.resolveWindowAtCursor).not.toHaveBeenCalled();
        expect(h.redockFloaterPane).not.toHaveBeenCalled();
    });

    describe("an armed release after a move", () => {
        const armedDrag = async () => {
            h.arming.isArmed.mockReturnValue(true);
            h.arming.target.mockReturnValue("main-1");
            h.api.windows.resolveWindowAtCursor.mockResolvedValue({ label: "main-1", window_id: "w1" });
            h.reload.mockImplementation(async (oref: string) =>
                oref === "window:w1" ? { workspaceid: "ws2" } : { oid: "ws2", activetabid: "tab9" }
            );
            const { gap } = mount();
            press(gap);
            await flush();
            moveTo(510, 330);
            return gap;
        };

        it("docks the pane into the armed window's active tab, with the ghost captured before the hover was cleared", async () => {
            await armedDrag();
            release(700, 400);
            expect(h.api.windows.getFloatingRedockTarget).toHaveBeenCalledWith("main-1");
            await vi.waitFor(() => expect(h.redockFloaterPane).toHaveBeenCalledTimes(1));
            expect(h.api.windows.resolveWindowAtCursor).toHaveBeenCalledWith({
                x: 700,
                y: 400,
                exclude_label: LABEL,
            });
            expect(h.redockFloaterPane).toHaveBeenCalledWith(
                { blockId: "b1", members: ["b1"], activeBlockId: "b1" },
                { tabId: "tab1", workspaceId: "ws1" },
                { tabId: "tab9", workspaceId: "ws2" },
                { blockId: "ghost-block", dir: 2 }
            );
            const order = (m: { mock: { invocationCallOrder: number[] } }) => m.mock.invocationCallOrder[0];
            expect(order(h.api.windows.getFloatingRedockTarget)).toBeLessThan(
                order(h.api.windows.clearFloatingRedockHover)
            );
        });

        it("docks with no ghost when the window under the cursor is not the one the ghost was for", async () => {
            await armedDrag();
            h.arming.target.mockReturnValue("other-window");
            release();
            await vi.waitFor(() => expect(h.redockFloaterPane).toHaveBeenCalledTimes(0));
            await flush();
            // The armed target ("other-window") is not the window the cursor resolves to: aborted.
            expect(h.redockFloaterPane).not.toHaveBeenCalled();
        });

        it("does nothing when the cursor is over no window", async () => {
            await armedDrag();
            h.api.windows.resolveWindowAtCursor.mockResolvedValue({ label: null, window_id: null });
            release();
            await flush();
            await flush();
            expect(h.api.windows.resolveWindowAtCursor).toHaveBeenCalled();
            expect(h.redockFloaterPane).not.toHaveBeenCalled();
        });

        it("does nothing when the target window has no active tab", async () => {
            await armedDrag();
            h.reload.mockImplementation(async (oref: string) =>
                oref === "window:w1" ? { workspaceid: "ws2" } : { oid: "ws2", activetabid: "" }
            );
            const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
            release();
            await flush();
            await flush();
            warn.mockRestore();
            expect(h.redockFloaterPane).not.toHaveBeenCalled();
        });

        it("logs and leaves the floater where it is when the redock call fails", async () => {
            await armedDrag();
            h.redockFloaterPane.mockRejectedValue(new Error("boom"));
            const error = vi.spyOn(console, "error").mockImplementation(() => {});
            release();
            await vi.waitFor(() => expect(h.redockFloaterPane).toHaveBeenCalledTimes(1));
            await flush();
            expect(error).toHaveBeenCalledWith("[floating-pane] RedockFloatingPane failed", expect.any(Error));
            error.mockRestore();
        });

        it("keeps the redock guard held while it docks, so the emptied tab does not close the window early", async () => {
            await armedDrag();
            const gate = h.unresolved<void>();
            h.redockFloaterPane.mockImplementation(() => gate.promise);
            release();
            await vi.waitFor(() => expect(h.redockFloaterPane).toHaveBeenCalledTimes(1));
            setTabValue({ blockids: [] });
            expect(h.api.closeWindowByLabel).not.toHaveBeenCalled();
            gate.resolve();
            await vi.waitFor(() => expect(h.api.closeWindowByLabel).toHaveBeenCalledWith(LABEL));
        });
    });

    it("an Esc-cancel from the host before the position arrives stops the drag from starting", async () => {
        const { gap } = mount();
        await flush();
        press(gap);
        h.listeners["window_drag_cancelled"]({ label: LABEL });
        await flush();
        moveTo(510, 330);
        expect(h.api.windows.setPosition).not.toHaveBeenCalled();
        expect(h.arming.reset).toHaveBeenCalled();
    });

    it("ignores a cancel for another window", async () => {
        const { gap } = mount();
        await flush();
        press(gap);
        h.listeners["window_drag_cancelled"]({ label: "other" });
        await flush();
        moveTo(510, 330);
        expect(h.api.windows.setPosition).toHaveBeenCalledWith(LABEL, 110, 230);
    });

    describe("of a maximized floater", () => {
        it("restores it on the first move, then holds the cursor's place on the restored header", async () => {
            h.api.windows.isMaximized.mockResolvedValue(true);
            const { gap } = mount();
            press(gap);
            await flush();
            expect(h.api.windows.toggleFloatingMaximize).not.toHaveBeenCalled();
            moveTo(510, 330);
            expect(h.api.windows.toggleFloatingMaximize).toHaveBeenCalledWith(LABEL, "b1");
            await flush();
            window.dispatchEvent(new Event("resize"));
            await vi.waitFor(() => expect(h.api.windows.setPosition).toHaveBeenCalledTimes(1));
            // x: 510 - (50 / 1024) * 800 rect width; y: 330 - 10 (the grab offset in the header).
            expect(h.api.windows.setPosition).toHaveBeenCalledWith(LABEL, 471, 320);
        });

        it("does not restore on a plain click", async () => {
            h.api.windows.isMaximized.mockResolvedValue(true);
            const { gap } = mount();
            press(gap);
            await flush();
            release();
            await flush();
            expect(h.api.windows.toggleFloatingMaximize).not.toHaveBeenCalled();
            expect(h.api.windows.setPosition).not.toHaveBeenCalled();
        });

        it("goes by the stored placement when the host cannot say", async () => {
            h.api.windows.isMaximized.mockResolvedValue(null);
            h.maximized = true;
            const { gap } = mount();
            press(gap);
            await flush();
            moveTo(510, 330);
            expect(h.api.windows.toggleFloatingMaximize).toHaveBeenCalledWith(LABEL, "b1");
        });
    });
});

describe("FloatingPaneWorkspace: header drag on Windows (host-side move loop)", () => {
    beforeEach(() => {
        h.platform = { windows: true, mac: false, linux: false, wayland: false };
    });
    const press = (el: Element, init: MouseEventInit = {}) =>
        el.dispatchEvent(mouse("mousedown", { screenX: 500, screenY: 300, clientX: 50, clientY: 10, ...init }));

    it("hands the move to the host and does not poll the window position", async () => {
        const { gap } = mount();
        press(gap);
        expect(h.api.windows.startDrag).toHaveBeenCalledWith(LABEL);
        expect(h.api.windows.getPosition).not.toHaveBeenCalled();
        document.dispatchEvent(mouse("mousemove", { screenX: 510, screenY: 330, buttons: 1 }));
        expect(h.api.windows.setPosition).not.toHaveBeenCalled();
    });

    it("waits for real movement before starting the host loop on a maximized floater", () => {
        h.maximized = true;
        const { gap } = mount();
        press(gap);
        expect(h.api.windows.startDrag).not.toHaveBeenCalled();
        document.dispatchEvent(mouse("mousemove", { clientX: 52, clientY: 11, buttons: 1 }));
        expect(h.api.windows.startDrag).not.toHaveBeenCalled();
        document.dispatchEvent(mouse("mousemove", { clientX: 60, clientY: 10, buttons: 1 }));
        expect(h.api.windows.startDrag).toHaveBeenCalledTimes(1);
    });

    it("drops the pending maximized press when the button came up unseen", () => {
        h.maximized = true;
        const { gap } = mount();
        press(gap);
        document.dispatchEvent(mouse("mousemove", { clientX: 80, clientY: 10, buttons: 0 }));
        document.dispatchEvent(mouse("mousemove", { clientX: 90, clientY: 10, buttons: 1 }));
        expect(h.api.windows.startDrag).not.toHaveBeenCalled();
    });

    describe("when the host says the drag ended", () => {
        const ended = (payload: Record<string, unknown>) =>
            h.listeners["window_drag_ended"]({ label: LABEL, moved: true, ...payload });

        it("docks at the host's cursor position when it moved and the arming state is armed", async () => {
            h.arming.isArmed.mockReturnValue(true);
            h.arming.target.mockReturnValue("main-1");
            h.api.windows.resolveWindowAtCursor.mockResolvedValue({ label: "main-1", window_id: "w1" });
            h.reload.mockImplementation(async (oref: string) =>
                oref === "window:w1" ? { workspaceid: "ws2" } : { oid: "ws2", activetabid: "tab9" }
            );
            const { gap } = mount();
            await flush();
            press(gap);
            ended({ cursor_x: 700, cursor_y: 400 });
            expect(h.api.windows.getFloatingRedockTarget).toHaveBeenCalledWith("main-1");
            expect(h.api.windows.clearFloatingRedockHover).toHaveBeenCalled();
            await vi.waitFor(() => expect(h.redockFloaterPane).toHaveBeenCalledTimes(1));
            expect(h.api.windows.resolveWindowAtCursor).toHaveBeenCalledWith({
                x: 700,
                y: 400,
                exclude_label: LABEL,
            });
        });

        it("does not dock when the drag did not move, or was not armed", async () => {
            mount();
            await flush();
            h.arming.isArmed.mockReturnValue(true);
            ended({ moved: false, cursor_x: 700, cursor_y: 400 });
            h.arming.isArmed.mockReturnValue(false);
            ended({ cursor_x: 700, cursor_y: 400 });
            await flush();
            expect(h.api.windows.resolveWindowAtCursor).not.toHaveBeenCalled();
            expect(h.api.windows.clearFloatingRedockHover).toHaveBeenCalledTimes(2);
        });

        it("ignores the end of another window's drag", async () => {
            mount();
            await flush();
            h.arming.isArmed.mockReturnValue(true);
            h.listeners["window_drag_ended"]({ label: "other", moved: true, cursor_x: 1, cursor_y: 1 });
            await flush();
            expect(h.api.windows.clearFloatingRedockHover).not.toHaveBeenCalled();
            expect(h.api.windows.resolveWindowAtCursor).not.toHaveBeenCalled();
        });

        it("docks at the position the DOM mouseup recorded, when the DOM release came first and carried the arm decision", async () => {
            h.arming.isArmed.mockReturnValue(true);
            h.arming.target.mockReturnValue("main-1");
            h.api.windows.resolveWindowAtCursor.mockResolvedValue({ label: "main-1", window_id: "w1" });
            h.reload.mockImplementation(async (oref: string) =>
                oref === "window:w1" ? { workspaceid: "ws2" } : { oid: "ws2", activetabid: "tab9" }
            );
            const { gap } = mount();
            await flush();
            press(gap);
            document.dispatchEvent(mouse("mouseup", { screenX: 640, screenY: 360 }));
            // The release only recorded the decision on Windows: the host's event commits the redock.
            expect(h.api.windows.resolveWindowAtCursor).not.toHaveBeenCalled();
            ended({});
            await vi.waitFor(() => expect(h.redockFloaterPane).toHaveBeenCalledTimes(1));
            expect(h.api.windows.resolveWindowAtCursor).toHaveBeenCalledWith({
                x: 640,
                y: 360,
                exclude_label: LABEL,
            });
        });
    });

    describe("the host's hover samples", () => {
        const hover = (payload: Record<string, unknown>) => h.listeners["floating-redock:hover-state"](payload);

        it("feeds them to the arming state while a drag is on, in CSS px", async () => {
            const { gap } = mount();
            await flush();
            press(gap);
            hover({ target_label: "main-1", cursor_x: 700, cursor_y: 400 });
            expect(h.arming.sample).toHaveBeenCalledWith(
                expect.objectContaining({ target: "main-1", x: 700, y: 400 })
            );
        });

        it("passes no position when the sample has none, and clears the indicator when told to", async () => {
            const { gap } = mount();
            await flush();
            press(gap);
            h.arming.sample.mockReturnValue({ clearIndicator: true });
            hover({ target_label: null });
            expect(h.arming.sample).toHaveBeenCalledWith(
                expect.objectContaining({ target: null, x: undefined, y: undefined })
            );
            expect(h.api.windows.clearFloatingRedockHover).toHaveBeenCalled();
        });

        it("ignores them when no drag is on", async () => {
            mount();
            await flush();
            hover({ target_label: "main-1", cursor_x: 1, cursor_y: 1 });
            expect(h.arming.sample).not.toHaveBeenCalled();
        });
    });

    it("does not listen for hover samples on other platforms", async () => {
        h.platform = { windows: false, mac: true, linux: false, wayland: false };
        mount();
        await flush();
        expect(h.listeners["floating-redock:hover-state"]).toBeUndefined();
        expect(h.listeners["window_drag_ended"]).toBeDefined();
        expect(h.listeners["window_drag_cancelled"]).toBeDefined();
    });
});

describe("FloatingPaneWorkspace: header drag on native Wayland (compositor move)", () => {
    beforeEach(() => {
        h.platform = { windows: false, mac: false, linux: true, wayland: true };
    });
    const press = (el: Element) =>
        el.dispatchEvent(mouse("mousedown", { screenX: 500, screenY: 300, clientX: 50, clientY: 10 }));
    const moveBy = (dx: number, buttons = 1) =>
        document.dispatchEvent(mouse("mousemove", { clientX: 50 + dx, clientY: 10, buttons }));

    it("starts the compositor's move once the press has moved, and does not poll the position", () => {
        const { gap } = mount();
        const ev = mouse("mousedown", { clientX: 50, clientY: 10 });
        gap.dispatchEvent(ev);
        expect(ev.defaultPrevented).toBe(true);
        expect(h.api.windows.getPosition).not.toHaveBeenCalled();
        moveBy(2);
        expect(h.api.windows.startDrag).not.toHaveBeenCalled();
        moveBy(10);
        expect(h.api.windows.startDrag).toHaveBeenCalledWith(LABEL);
        moveBy(20);
        expect(h.api.windows.startDrag).toHaveBeenCalledTimes(1);
    });

    it("forgets the press when the button came up unseen, or on a release", () => {
        const { gap } = mount();
        press(gap);
        moveBy(10, 0);
        moveBy(10);
        press(gap);
        document.dispatchEvent(mouse("mouseup", {}));
        moveBy(10);
        expect(h.api.windows.startDrag).not.toHaveBeenCalled();
    });
});

describe("FloatingPaneWorkspace: double-click on the header", () => {
    it("maximizes or restores the floater and keeps the header's own double-click from running", () => {
        const { gap } = mount();
        const ev = mouse("dblclick", {});
        const stop = vi.spyOn(ev, "stopPropagation");
        gap.dispatchEvent(ev);
        expect(h.api.windows.toggleFloatingMaximize).toHaveBeenCalledWith(LABEL, "b1");
        expect(stop).toHaveBeenCalled();
    });

    it("leaves a double-click on a header control, outside the header, or with another button alone", () => {
        const { button, body, gap } = mount();
        button.dispatchEvent(mouse("dblclick", {}));
        body.dispatchEvent(mouse("dblclick", {}));
        gap.dispatchEvent(mouse("dblclick", { button: 2 }));
        expect(h.api.windows.toggleFloatingMaximize).not.toHaveBeenCalled();
    });

    it("does nothing while the tab has no block yet", () => {
        h.blockIds = [];
        const { gap } = mount();
        gap.dispatchEvent(mouse("dblclick", {}));
        expect(h.api.windows.toggleFloatingMaximize).not.toHaveBeenCalled();
    });
});

describe("FloatingPaneWorkspace: unmounting", () => {
    it("removes its document listeners and unlistens from the host events", async () => {
        const { gap, body, unmount } = mount();
        await flush();
        expect(Object.keys(h.listeners).sort()).toEqual(["window_drag_cancelled", "window_drag_ended"]);
        unmount();
        for (const el of [gap, body]) {
            document.body.append(el);
        }
        gap.dataset.role = "block-header";
        const header = document.createElement("div");
        header.dataset.role = "block-header";
        const edge = document.createElement("div");
        document.body.append(header, edge);
        header.dispatchEvent(mouse("mousedown", { screenX: 5, screenY: 5 }));
        header.dispatchEvent(mouse("dblclick", {}));
        edge.dispatchEvent(mouse("pointerdown", { clientX: 2, clientY: 400 }));
        expect(h.api.windows.getPosition).not.toHaveBeenCalled();
        expect(h.api.windows.toggleFloatingMaximize).not.toHaveBeenCalled();
        expect(h.api.windows.getRect).not.toHaveBeenCalled();
        expect(Object.keys(h.listeners)).toEqual([]);
    });

    it("unlistens at once when it unmounts before the host answers the listen", async () => {
        const resolvers: Array<(u: () => void) => void> = [];
        const unlisten = vi.fn();
        h.api.listen = vi.fn(() => new Promise<() => void>((r) => resolvers.push(r)));
        const { unmount } = mount();
        expect(resolvers.length).toBe(2);
        unmount();
        resolvers.forEach((r) => r(unlisten));
        await flush();
        expect(unlisten).toHaveBeenCalledTimes(2);
    });

    it("does not dock after it was unmounted mid-redock", async () => {
        h.arming.isArmed.mockReturnValue(true);
        h.arming.target.mockReturnValue("main-1");
        const resolve = h.unresolved<{ label: string; window_id: string }>();
        h.api.windows.resolveWindowAtCursor.mockImplementation(() => resolve.promise);
        const { gap, unmount } = mount();
        gap.dispatchEvent(mouse("mousedown", { screenX: 500, screenY: 300, clientX: 50, clientY: 10 }));
        await flush();
        document.dispatchEvent(mouse("mousemove", { screenX: 510, screenY: 330, buttons: 1 }));
        document.dispatchEvent(mouse("mouseup", { screenX: 700, screenY: 400 }));
        await vi.waitFor(() => expect(h.api.windows.resolveWindowAtCursor).toHaveBeenCalledTimes(1));
        unmount();
        resolve.resolve({ label: "main-1", window_id: "w1" });
        await flush();
        expect(h.reload).not.toHaveBeenCalled();
        expect(h.redockFloaterPane).not.toHaveBeenCalled();
    });
});
