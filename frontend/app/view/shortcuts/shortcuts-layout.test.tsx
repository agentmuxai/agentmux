// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Shortcuts pane's layout and zoom: the filter, the tiles and the hints are three rows of a
 * column, so none can sit over another (jsdom has no layout, so this checks the structure that
 * guarantees it, and the arrow keys' use of the grid's columns); and the pane's `term:zoom` scales
 * the whole view.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => {
    const state = {
        globalMock: undefined as unknown as () => Promise<Record<string, unknown>>,
    };
    state.globalMock = async () => {
        const signals = await import("@/app/store/config-signals");
        return {
            atoms: { fullConfigAtom: signals.fullConfigAtom },
            MOS: { makeORef: (_t: string, id: string) => id, getMuxObjectAtom: () => () => undefined },
        };
    };
    return state;
});

vi.mock("@/app/store/block-layout-actions", () => ({ replaceBlock: vi.fn(async () => "new-block"), createBlock: vi.fn() }));
vi.mock("@/app/store/block-component-registry", () => ({ openOrFocusPaneByView: vi.fn() }));
vi.mock("@/app/store/global", () => h.globalMock());
vi.mock("@/store/global", () => h.globalMock());
vi.mock("@/app/util/reveal-block", () => ({ findOpenBlockByView: async () => null }));
vi.mock("@/layout/index", () => ({
    getLayoutModelForStaticTab: () => ({ getNodeByBlockId: () => undefined }),
    closeBlockInStack: vi.fn(async () => undefined),
}));

import { setFullConfigAtom } from "@/app/store/config-signals";
import { shortcutsPaneTab } from "./shortcuts";

const w = (label: string, order: number, view = label.toLowerCase()): WidgetConfigType =>
    ({ label, icon: "circle", "display:order": order, blockdef: { meta: { view } } }) as WidgetConfigType;

function mountPane(meta: Record<string, unknown> = {}) {
    setFullConfigAtom({
        settings: {},
        widgets: {
            "defwidget@agent": w("Agent", 1),
            "defwidget@terminal": w("Terminal", 2, "term"),
            "defwidget@browser": w("Browser", 3),
            "defwidget@drone": w("Drone", 4),
        },
    } as unknown as FullConfigType);
    const tab = shortcutsPaneTab.create({ blockId: "b1", meta: () => meta } as never);
    const result = render(() => <>{tab.component({ ctx: {} as never })}</>);
    return { ...result, root: result.container.querySelector(".shortcuts-view") as HTMLElement };
}

const scrollIntoView = vi.fn();

beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
    Element.prototype.scrollIntoView = scrollIntoView;
});
afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
});

describe("Shortcuts pane: layout", () => {
    it("is a column of the filter, the tiles and the hints, each in a row of its own", () => {
        const { root } = mountPane();
        const rows = Array.from(root.children).map((c) => c.className.split(" ")[0]);
        expect(rows.filter((c) => c.startsWith("shortcuts-"))).toEqual(["shortcuts-top", "shortcuts-scroll", "shortcuts-hints"]);
        const [top, scroll, hints] = ["shortcuts-top", "shortcuts-scroll", "shortcuts-hints"].map((c) => root.querySelector(`.${c}`)!);
        expect(top.contains(screen.getByTestId("shortcuts-filter-input"))).toBe(true);
        expect(scroll.contains(screen.getByTestId("shortcuts-grid"))).toBe(true);
        expect(hints.contains(screen.getByTestId("shortcuts-grid"))).toBe(false);
        // Every tile is in the scrolling row, none in the filter's or the hints'.
        expect(scroll.querySelectorAll(".shortcuts-tile").length).toBe(4);
        expect(top.querySelector(".shortcuts-tile")).toBeNull();
        expect(hints.querySelector(".shortcuts-tile")).toBeNull();
    });

    it("keeps the drill-down breadcrumb in the filter's row, not among the tiles", () => {
        setFullConfigAtom({
            settings: {},
            widgets: {
                "defwidget@group": { ...w("Messengers", 1), children: ["defwidget@a"] },
                "defwidget@a": { ...w("Slack", 2), "display:grouponly": true },
            },
        } as unknown as FullConfigType);
        const tab = shortcutsPaneTab.create({ blockId: "b1", meta: () => ({}) } as never);
        const { container } = render(() => <>{tab.component({ ctx: {} as never })}</>);
        fireEvent.click(screen.getByText("Messengers"));
        const crumb = screen.getAllByText("Messengers")[0];
        expect(container.querySelector(".shortcuts-top")!.contains(crumb)).toBe(true);
        expect(container.querySelector(".shortcuts-scroll")!.contains(crumb)).toBe(false);
    });

    it("moves down a row by the columns the grid actually has, and scrolls the chosen tile into view", () => {
        const real = window.getComputedStyle.bind(window);
        vi.spyOn(window, "getComputedStyle").mockImplementation((el: Element, pseudo?: string | null) =>
            el.classList?.contains("shortcuts-grid")
                ? ({ gridTemplateColumns: "104px 104px 104px" } as CSSStyleDeclaration)
                : real(el, pseudo)
        );
        mountPane();
        const filter = screen.getByTestId("shortcuts-filter-input");
        fireEvent.keyDown(filter, { key: "ArrowDown", code: "ArrowDown" });
        // Four tiles in rows of three: down from the first lands on the fourth.
        const tiles = document.querySelectorAll(".shortcuts-tile");
        expect(scrollIntoView).toHaveBeenLastCalledWith({ block: "nearest" });
        expect(scrollIntoView.mock.contexts.at(-1)).toBe(tiles[3]);
    });
});

describe("Shortcuts pane: zoom", () => {
    it("takes part in the shared pane zoom, with a 12 px base", () => {
        expect(shortcutsPaneTab.capabilities?.paneZoom).toEqual({ baseFontSize: 12 });
    });

    it("applies the pane's term:zoom to the whole view", () => {
        const { root } = mountPane({ "term:zoom": 1.5 });
        expect(root.style.zoom).toBe("1.5");
    });

    it("is at 100% when the pane has no zoom set", () => {
        const { root } = mountPane();
        expect(root.style.zoom).toBe("1");
    });
});
