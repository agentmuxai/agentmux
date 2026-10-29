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
        closeNode: vi.fn(async (..._a: unknown[]) => undefined),
        blocks: {} as Record<string, { meta: Record<string, unknown> }>,
        globalMock: undefined as unknown as () => Promise<Record<string, unknown>>,
    };
    state.globalMock = async () => {
        const signals = await import("@/app/store/config-signals");
        return {
            atoms: { fullConfigAtom: signals.fullConfigAtom },
            replaceBlock: state.replaceBlock,
            createBlock: vi.fn(),
            openOrFocusPaneByView: vi.fn(),
            MOS: { makeORef: (_t: string, id: string) => id, getMuxObjectAtom: (id: string) => () => state.blocks[id] },
        };
    };
    return state;
});

vi.mock("@/app/asset/logo.svg?url", () => ({ default: "logo.svg" }));
// The same module under both of its import names.
vi.mock("@/app/store/global", () => h.globalMock());
vi.mock("@/store/global", () => h.globalMock());
vi.mock("@/layout/index", () => ({
    getLayoutModelForStaticTab: () => ({
        getNodeByBlockId: (blockId: string) => (h.blocks[blockId] ? { id: `node-of-${blockId}` } : undefined),
        closeNode: h.closeNode,
    }),
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

describe("Shortcuts pane: Escape", () => {
    function open(meta: Record<string, unknown>) {
        config();
        h.blocks.sc = { meta: { view: "shortcuts", ...meta } };
        const m = new ShortcutsViewModel("sc");
        m.gridLayout = { columns: 3, tileWidth: 90, tileHeight: 90, showLabel: true };
        return m;
    }

    it("closes a pane a split opened when the search is empty, with nothing asked", () => {
        const m = open({ "shortcuts:from": "src" });
        expect(m.keyDownHandler(escape())).toBe(true);
        expect(h.closeNode).toHaveBeenCalledOnce();
        expect(h.closeNode).toHaveBeenCalledWith("node-of-sc");
    });

    it("only clears the search first, then closes on the next Escape", () => {
        const m = open({ "shortcuts:from": "src" });
        m.searchTerm._set("ter");
        m.keyDownHandler(escape());
        expect(m.searchTerm()).toBe("");
        expect(h.closeNode).not.toHaveBeenCalled();
        m.keyDownHandler(escape());
        expect(h.closeNode).toHaveBeenCalledOnce();
    });

    it("pops out of a group before it closes", () => {
        const m = open({ "shortcuts:from": "src" });
        m.activeParent._set(w("Messengers", 9, "messengers", { children: ["discord"] }));
        m.keyDownHandler(escape());
        expect(m.activeParent()).toBeNull();
        expect(h.closeNode).not.toHaveBeenCalled();
    });

    it("never closes a pane that no split opened (app:defaultnewblock)", () => {
        const m = open({});
        expect(m.openedBySplit()).toBe(false);
        m.keyDownHandler(escape());
        expect(h.closeNode).not.toHaveBeenCalled();
    });
});
