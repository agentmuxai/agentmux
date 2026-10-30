// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Shortcuts pane's filter bar: it is in the pane, has the focus once the pane is shown, narrows
 * the tiles as you type, and the arrows, Enter and Escape work from it.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => {
    const state = {
        replaceBlock: vi.fn(async (..._a: unknown[]) => "new-block"),
        closeBlockInStack: vi.fn(async (..._a: unknown[]) => undefined),
        globalMock: undefined as unknown as () => Promise<Record<string, unknown>>,
    };
    state.globalMock = async () => {
        const signals = await import("@/app/store/config-signals");
        return {
            atoms: { fullConfigAtom: signals.fullConfigAtom },
            replaceBlock: state.replaceBlock,
            createBlock: vi.fn(),
            openOrFocusPaneByView: vi.fn(),
            MOS: { makeORef: (_t: string, id: string) => id, getMuxObjectAtom: () => () => undefined },
        };
    };
    return state;
});

vi.mock("@/app/store/global", () => h.globalMock());
vi.mock("@/store/global", () => h.globalMock());
vi.mock("@/app/util/reveal-block", () => ({ findOpenBlockByView: async () => null }));
vi.mock("@/layout/index", () => ({
    getLayoutModelForStaticTab: () => ({ getNodeByBlockId: () => undefined }),
    closeBlockInStack: h.closeBlockInStack,
}));

import { setFullConfigAtom } from "@/app/store/config-signals";
import { shortcutsPaneTab } from "./shortcuts";

const w = (label: string, order: number, view = label.toLowerCase()): WidgetConfigType =>
    ({ label, icon: "circle", "display:order": order, blockdef: { meta: { view } } }) as WidgetConfigType;

function mountPane() {
    setFullConfigAtom({
        settings: {},
        widgets: {
            "defwidget@agent": w("Agent", 1),
            "defwidget@terminal": w("Terminal", 2, "term"),
            "defwidget@browser": w("Browser", 3),
            "defwidget@drone": w("Drone", 4),
        },
    } as unknown as FullConfigType);
    const tab = shortcutsPaneTab.create({ blockId: "b1" } as never);
    const result = render(() => <>{tab.component({ ctx: {} as never })}</>);
    const filter = screen.getByTestId("shortcuts-filter-input") as HTMLInputElement;
    return { ...result, filter };
}

const tiles = () => ["Agent", "Terminal", "Browser", "Drone"].filter((l) => screen.queryByText(l) != null);

beforeEach(() => {
    vi.clearAllMocks();
    // jsdom has no layout: the grid falls back to one column, which is all these tests need.
    vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
});
afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
});

describe("Shortcuts pane: the filter bar", () => {
    it("is a visible filter box that has the focus as soon as the pane is shown", async () => {
        const { filter } = mountPane();
        expect(filter.placeholder).toBe("Filter shortcuts");
        await Promise.resolve();
        expect(document.activeElement).toBe(filter);
    });

    it("narrows the tiles as you type, and Escape brings them all back", () => {
        const { filter } = mountPane();
        expect(tiles()).toEqual(["Agent", "Terminal", "Browser", "Drone"]);
        fireEvent.input(filter, { target: { value: "dr" } });
        expect(tiles()).toEqual(["Drone"]);
        fireEvent.keyDown(filter, { key: "Escape", code: "Escape" });
        expect(filter.value).toBe("");
        expect(tiles()).toEqual(["Agent", "Terminal", "Browser", "Drone"]);
    });

    it("Enter launches the first match", async () => {
        const { filter } = mountPane();
        fireEvent.input(filter, { target: { value: "dr" } });
        fireEvent.keyDown(filter, { key: "Enter", code: "Enter" });
        await vi.waitFor(() => expect(h.replaceBlock).toHaveBeenCalledTimes(1));
        expect(h.replaceBlock.mock.calls[0][1]).toMatchObject({ meta: { view: "drone" } });
    });

    it("the arrow keys move the selection from the bar, and Enter launches that tile", async () => {
        const { filter } = mountPane();
        fireEvent.input(filter, { target: { value: "r" } });
        expect(tiles()).toEqual(["Terminal", "Browser", "Drone"]);
        fireEvent.keyDown(filter, { key: "ArrowDown", code: "ArrowDown" });
        fireEvent.keyDown(filter, { key: "Enter", code: "Enter" });
        await vi.waitFor(() => expect(h.replaceBlock).toHaveBeenCalledTimes(1));
        expect(h.replaceBlock.mock.calls[0][1]).toMatchObject({ meta: { view: "browser" } });
    });

    it("says which keys do what, and what to do when nothing matches", () => {
        const { filter } = mountPane();
        const hints = screen.getByTestId("shortcuts-hints");
        expect(Array.from(hints.querySelectorAll("kbd")).map((k) => k.textContent)).toEqual(["↑", "↓", "←", "→", "Enter"]);
        fireEvent.input(filter, { target: { value: "zzz" } });
        expect(hints.textContent).toContain('Nothing matches "zzz"');
        expect(hints.querySelector("kbd")?.textContent).toBe("Esc");
    });
});
