// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Shortcuts pane's model: tile order (last used first), a tile replacing the pane in place
 * with the source pane's directory, and Escape closing a pane a split opened.
 * docs/specs/SPEC_SHORTCUTS_PANE_FOR_SPLITS_2026_10_28.md §2.2, §2.4, §2.5, §3.3.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => {
    const state = {
        replaceBlock: vi.fn(async (..._a: unknown[]) => "new-block"),
        closeBlockInStack: vi.fn(async (..._a: unknown[]) => undefined),
        openOrFocusPaneByView: vi.fn(async (..._a: unknown[]) => undefined),
        createBlock: vi.fn(async (..._a: unknown[]) => "created"),
        /** The views with a pane open in this window. */
        openViews: new Set<string>(),
        order: [] as string[],
        blocks: {} as Record<string, { meta: Record<string, unknown> }>,
        globalMock: undefined as unknown as () => Promise<Record<string, unknown>>,
    };
    state.globalMock = async () => {
        const signals = await import("@/app/store/config-signals");
        return {
            atoms: { fullConfigAtom: signals.fullConfigAtom },
            replaceBlock: state.replaceBlock,
            createBlock: state.createBlock,
            openOrFocusPaneByView: state.openOrFocusPaneByView,
            MOS: { makeORef: (_t: string, id: string) => id, getMuxObjectAtom: (id: string) => () => state.blocks[id] },
        };
    };
    return state;
});

vi.mock("@/app/asset/logo.svg?url", () => ({ default: "logo.svg" }));
// The same module under both of its import names.
vi.mock("@/app/store/global", () => h.globalMock());
vi.mock("@/store/global", () => h.globalMock());
const layoutModel = {
    getNodeByBlockId: (blockId: string) => (h.blocks[blockId] ? { id: `node-of-${blockId}` } : undefined),
};
vi.mock("@/app/store/block-component-registry", () => ({
    getAllBlockComponentModels: () => [...h.openViews].map((viewType) => ({ viewModel: { viewType } })),
}));
vi.mock("@/layout/index", () => ({
    getLayoutModelForStaticTab: () => layoutModel,
    closeBlockInStack: h.closeBlockInStack,
}));

import { setFullConfigAtom } from "@/app/store/config-signals";
import { adaptFromReactOrNativeKeyEvent } from "@/util/keyutil";
import { ShortcutsViewModel, shortcutsPaneTab } from "./shortcuts";

const w = (label: string, order: number, view = label.toLowerCase(), extra: Partial<WidgetConfigType> = {}): WidgetConfigType =>
    ({ label, icon: "circle", "display:order": order, blockdef: { meta: { view } }, ...extra }) as WidgetConfigType;
const widgets = (): Record<string, WidgetConfigType> => ({
    "defwidget@agent": w("Agent", 1),
    "defwidget@terminal": w("Terminal", 2, "term"),
    "defwidget@files": w("Hangar", 3, "files"),
    "defwidget@browser": w("Browser", 4),
    "defwidget@drone": w("Drone", 5),
    "ext@acme.board/main": w("Board", 6, "ext:acme.board/main"),
});
const config = (settings: Record<string, unknown> = {}) =>
    setFullConfigAtom({ settings, widgets: widgets() } as unknown as FullConfigType);
const labels = (m: ShortcutsViewModel) => m.filteredWidgets().map((x) => x.label);
const escape = () => adaptFromReactOrNativeKeyEvent(new KeyboardEvent("keydown", { key: "Escape", code: "Escape" }));

beforeEach(() => {
    vi.clearAllMocks();
    h.blocks = {};
    h.openViews.clear();
});

describe("Shortcuts pane: the manifest", () => {
    it("is the view `shortcuts`, with `launcher` as the old name saved layouts still carry", () => {
        expect(shortcutsPaneTab.view).toBe("shortcuts");
        expect(shortcutsPaneTab.aliases).toEqual(["launcher"]);
        expect(shortcutsPaneTab.label).toBe("Shortcuts");
        expect(shortcutsPaneTab.capabilities?.header).toBe("none");
    });
});

describe("Shortcuts pane: tile order", () => {
    it("lists every pane type and installed widget in the configured order when nothing was used", () => {
        config();
        expect(labels(new ShortcutsViewModel("b1"))).toEqual(["Agent", "Terminal", "Hangar", "Browser", "Drone", "Board"]);
    });

    it("puts the last used first, then the rest in their configured place", () => {
        config({ "launcher:recent": ["ext@acme.board/main", "defwidget@files"] });
        expect(labels(new ShortcutsViewModel("b1"))).toEqual(["Board", "Hangar", "Agent", "Terminal", "Browser", "Drone"]);
    });

    it("leaves out a hidden entry even when it was used last, and ignores a key that no longer exists", () => {
        config({ "widget:hidden": ["defwidget@drone"], "launcher:recent": ["defwidget@drone", "defwidget@gone", "defwidget@browser"] });
        expect(labels(new ShortcutsViewModel("b1"))).toEqual(["Browser", "Agent", "Terminal", "Hangar", "Board"]);
    });

    it("filters by label as before", () => {
        config({ "launcher:recent": ["defwidget@browser"] });
        const m = new ShortcutsViewModel("b1");
        m.searchTerm._set("o");
        expect(labels(m)).toEqual(["Browser", "Drone", "Board"]);
    });
});

describe("Shortcuts pane: a group's tiles", () => {
    it("the last used of a group's tiles come first, as at the top level", () => {
        setFullConfigAtom({
            settings: { "launcher:recent": ["defwidget@teams"] },
            widgets: {
                ...widgets(),
                "defwidget@messengers": w("Messengers", 7, "messengers", { children: ["discord", "teams", "slack"] }),
                "defwidget@discord": w("Discord", 8, "browser", { "display:grouponly": true }),
                "defwidget@teams": w("Teams", 9, "browser", { "display:grouponly": true }),
                "defwidget@slack": w("Slack", 10, "browser", { "display:grouponly": true }),
            },
        } as unknown as FullConfigType);
        const m = new ShortcutsViewModel("b1");
        m.drillInto(m.filteredWidgets().find((x) => x.label === "Messengers")!);
        expect(labels(m)).toEqual(["Teams", "Discord", "Slack"]);
    });
});

describe("Shortcuts pane: choosing a tile", () => {
    const tile = (m: ShortcutsViewModel, label: string) => m.filteredWidgets().find((x) => x.label === label)!;

    it("replaces the pane in place; a Terminal tile starts in the split pane's directory and connection", async () => {
        config();
        h.blocks.src = { meta: { view: "term", "cmd:cwd": "C:/work", connection: "wsl://Ubuntu" } };
        h.blocks.sc = { meta: { view: "shortcuts", "shortcuts:from": "src" } };
        const m = new ShortcutsViewModel("sc");

        await m.handleWidgetSelect(tile(m, "Terminal"));

        expect(h.replaceBlock).toHaveBeenCalledWith("sc", { meta: { "cmd:cwd": "C:/work", connection: "wsl://Ubuntu", view: "term" } }, true);
    });

    it("a Hangar tile starts in that directory; an Agent tile opens the picker as the widget defines it", async () => {
        config();
        h.blocks.src = { meta: { view: "term", "cmd:cwd": "C:/work" } };
        h.blocks.sc = { meta: { view: "shortcuts", "shortcuts:from": "src" } };
        const m = new ShortcutsViewModel("sc");

        await m.handleWidgetSelect(tile(m, "Hangar"));
        await m.handleWidgetSelect(tile(m, "Agent"));

        expect(h.replaceBlock.mock.calls[0]).toEqual(["sc", { meta: { "files:path": "C:/work", view: "files" } }, true]);
        expect(h.replaceBlock.mock.calls[1]).toEqual(["sc", widgets()["defwidget@agent"].blockdef, true]);
    });

    it("with no pane that was split, or one that is gone, the tile is as configured", async () => {
        config();
        h.blocks.sc = { meta: { view: "shortcuts" } };
        h.blocks.gone = { meta: { view: "shortcuts", "shortcuts:from": "closed" } };
        for (const id of ["sc", "gone"]) {
            const m = new ShortcutsViewModel(id);
            await m.handleWidgetSelect(tile(m, "Terminal"));
        }
        expect(h.replaceBlock.mock.calls.map((c) => c[1])).toEqual([widgets()["defwidget@terminal"].blockdef, widgets()["defwidget@terminal"].blockdef]);
    });
});

describe("Shortcuts pane: a singleton tile", () => {
    const tile = (m: ShortcutsViewModel, label: string) => m.filteredWidgets().find((x) => x.label === label)!;
    function open(singletons: Record<string, boolean> = {}) {
        setFullConfigAtom({
            settings: {},
            widgets: {
                "defwidget@memory": w("Memory", 1, "memory", { singleton: singletons.memory }),
                "defwidget@terminal": w("Terminal", 2, "term"),
                "defwidget@agent": w("Agent", 3),
            },
        } as unknown as FullConfigType);
        h.blocks.sc = { meta: { view: "shortcuts", "shortcuts:from": "src" } };
        h.order.length = 0;
        h.closeBlockInStack.mockImplementation(async () => void h.order.push("close"));
        h.openOrFocusPaneByView.mockImplementation(async () => void h.order.push("focus"));
        return new ShortcutsViewModel("sc");
    }

    it("brings back the open pane and closes this one, this one first so the focus is not handed on after", async () => {
        h.openViews.add("memory");
        const m = open();
        await m.handleWidgetSelect(tile(m, "Memory"));
        expect(h.openOrFocusPaneByView).toHaveBeenCalledWith("memory", { meta: { view: "memory" } }, undefined);
        expect(h.closeBlockInStack).toHaveBeenCalledWith(layoutModel, "node-of-sc", "sc");
        expect(h.order).toEqual(["close", "focus"]);
        expect(h.replaceBlock).not.toHaveBeenCalled();
    });

    it("the entry's own singleton flag decides, then the view's policy", async () => {
        h.openViews.add("term").add("browser");
        const m = open();
        // Terminal is not one (no flag, not a built-in singleton): in place.
        await m.handleWidgetSelect(tile(m, "Terminal"));
        expect(h.replaceBlock).toHaveBeenCalledOnce();
        expect(h.openOrFocusPaneByView).not.toHaveBeenCalled();
        h.replaceBlock.mockClear();
        // A flag on the entry makes it one.
        const m2 = open({ memory: true });
        h.openViews.add("memory");
        await m2.handleWidgetSelect(tile(m2, "Memory"));
        expect(h.openOrFocusPaneByView).toHaveBeenCalledOnce();
    });

    it("a singleton that is not open opens in this pane's place", async () => {
        const m = open();
        await m.handleWidgetSelect(tile(m, "Memory"));
        expect(h.replaceBlock).toHaveBeenCalledOnce();
        expect(h.openOrFocusPaneByView).not.toHaveBeenCalled();
        expect(h.closeBlockInStack).not.toHaveBeenCalled();
    });

    it("Shift (forceNew) opens a new pane in place even when one is open", async () => {
        h.openViews.add("memory");
        const m = open();
        await m.handleWidgetSelect(tile(m, "Memory"), { forceNew: true });
        expect(h.replaceBlock).toHaveBeenCalledOnce();
        expect(h.openOrFocusPaneByView).not.toHaveBeenCalled();
        expect(h.closeBlockInStack).not.toHaveBeenCalled();
    });

    it("Enter on the chosen tile brings it back; Shift+Enter opens a new one", async () => {
        h.openViews.add("memory");
        const m = open();
        m.gridLayout = { columns: 3, tileWidth: 90, tileHeight: 90, showLabel: true };
        m.selectedIndex._set(m.filteredWidgets().findIndex((x) => x.label === "Memory"));
        m.keyDownHandler(adaptFromReactOrNativeKeyEvent({ type: "keydown", key: "Enter", code: "Enter" } as KeyboardEvent));
        await vi.waitFor(() => expect(h.openOrFocusPaneByView).toHaveBeenCalledOnce());
        m.keyDownHandler(
            adaptFromReactOrNativeKeyEvent({ type: "keydown", key: "Enter", code: "Enter", shiftKey: true } as KeyboardEvent)
        );
        await vi.waitFor(() => expect(h.replaceBlock).toHaveBeenCalledOnce());
        expect(h.openOrFocusPaneByView).toHaveBeenCalledOnce();
    });
});

describe("Shortcuts pane: Escape", () => {
    function open(meta: Record<string, unknown>) {
        config();
        h.blocks.sc = { meta: { view: "shortcuts", ...meta } };
        const m = new ShortcutsViewModel("sc");
        m.gridLayout = { columns: 3, tileWidth: 90, tileHeight: 90, showLabel: true };
        return m;
    }

    it("closes a pane a split opened when the search is empty, with nothing asked, and only its own tab", () => {
        const m = open({ "shortcuts:from": "src" });
        expect(m.keyDownHandler(escape())).toBe(true);
        expect(h.closeBlockInStack).toHaveBeenCalledOnce();
        // The tab, not the pane: the other tabs of a stack it was added to stay.
        expect(h.closeBlockInStack).toHaveBeenCalledWith(layoutModel, "node-of-sc", "sc");
    });

    it("only clears the search first, then closes on the next Escape", () => {
        const m = open({ "shortcuts:from": "src" });
        m.searchTerm._set("ter");
        m.keyDownHandler(escape());
        expect(m.searchTerm()).toBe("");
        expect(h.closeBlockInStack).not.toHaveBeenCalled();
        m.keyDownHandler(escape());
        expect(h.closeBlockInStack).toHaveBeenCalledOnce();
    });

    it("pops out of a group before it closes", () => {
        const m = open({ "shortcuts:from": "src" });
        m.activeParent._set(w("Messengers", 9, "messengers", { children: ["discord"] }));
        m.keyDownHandler(escape());
        expect(m.activeParent()).toBeNull();
        expect(h.closeBlockInStack).not.toHaveBeenCalled();
    });

    it("never closes a pane that no split opened (app:defaultnewblock)", () => {
        const m = open({});
        expect(m.openedBySplit()).toBe(false);
        m.keyDownHandler(escape());
        expect(h.closeBlockInStack).not.toHaveBeenCalled();
    });
});
