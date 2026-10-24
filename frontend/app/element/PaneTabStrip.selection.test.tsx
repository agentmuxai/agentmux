// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Ctrl/Cmd+click multi-select on the Pane Tab strip, and the drag it starts.
 * docs/specs/SPEC_PANE_TAB_MULTI_SELECT_MOVE_AND_COLLAPSED_EXPAND_2026_10_24.md §4.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setPlatform } from "@/util/platformutil";
import { PaneTabStrip } from "./PaneTabStrip";
import {
    clearPaneTabSelection,
    isPaneTabSelected,
    paneTabSelection,
    paneTabSelectionApi,
    resetPaneTabSelection,
} from "./pane-tab-selection";

interface T {
    id: string;
    label: string;
}
const TABS: T[] = [
    { id: "a", label: "alpha" },
    { id: "b", label: "beta" },
    { id: "c", label: "gamma" },
];

function mount(extra: Record<string, unknown> = {}, withSelection = true) {
    const onActivate = vi.fn();
    const onSelect = vi.fn();
    const onDoubleClick = vi.fn();
    const outerClick = vi.fn();
    const result = render(() => (
        <div onClick={outerClick}>
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={onActivate}
                onSelect={onSelect}
                onTabDoubleClick={onDoubleClick}
                onReorder={() => true}
                paneKey="pane-1"
                selection={withSelection ? paneTabSelectionApi : undefined}
                {...extra}
            />
        </div>
    ));
    const pill = (id: string) => result.container.querySelector<HTMLElement>(`.pane-tab[data-tab-id="${id}"]`)!;
    return { ...result, pill, onActivate, onSelect, onDoubleClick, outerClick };
}

beforeEach(() => {
    setPlatform("win32");
    resetPaneTabSelection();
});
afterEach(() => {
    cleanup();
    resetPaneTabSelection();
});

describe("Ctrl+click on a pill", () => {
    it("picks the tab without activating, selecting or focusing anything", () => {
        const { pill, onActivate, onSelect, outerClick } = mount();

        fireEvent.click(pill("b"), { ctrlKey: true });

        expect(isPaneTabSelected("b")).toBe(true);
        expect(pill("b").classList.contains("pane-tab--selected")).toBe(true);
        expect(onActivate).not.toHaveBeenCalled();
        expect(onSelect).not.toHaveBeenCalled();
        expect(outerClick, "the pane's own click handler (focus) must not run").not.toHaveBeenCalled();
    });

    it("picks tabs one after another, and Ctrl+click on a picked one unpicks it", () => {
        const { pill } = mount();

        fireEvent.click(pill("b"), { ctrlKey: true });
        fireEvent.click(pill("c"), { ctrlKey: true });
        expect([...paneTabSelection()].sort()).toEqual(["b", "c"]);

        fireEvent.click(pill("b"), { ctrlKey: true });
        expect([...paneTabSelection()]).toEqual(["c"]);
        expect(pill("b").classList.contains("pane-tab--selected")).toBe(false);
    });

    it("does not pick the active tab implicitly", () => {
        const { pill } = mount();
        fireEvent.click(pill("b"), { ctrlKey: true });
        expect(isPaneTabSelected("a")).toBe(false);
    });

    it("shows a check mark a screen reader can read, besides the outline", () => {
        const { pill } = mount();
        fireEvent.click(pill("b"), { ctrlKey: true });
        const mark = pill("b").querySelector('[role="img"]');
        expect(mark?.getAttribute("aria-label")).toBe("Selected");
        expect(pill("a").querySelector('[role="img"]')).toBeNull();
    });

    it("is not a rename when double-clicked with the modifier", () => {
        const { pill, onDoubleClick } = mount();
        fireEvent.dblClick(pill("b"), { ctrlKey: true });
        expect(onDoubleClick).not.toHaveBeenCalled();
        fireEvent.dblClick(pill("b"));
        expect(onDoubleClick).toHaveBeenCalledTimes(1);
    });

    it("on macOS the modifier is Cmd, and Ctrl+click stays a plain click", () => {
        setPlatform("darwin");
        const { pill, onActivate } = mount();

        fireEvent.click(pill("b"), { ctrlKey: true });
        expect(isPaneTabSelected("b")).toBe(false);
        expect(onActivate).toHaveBeenCalledWith("b");

        fireEvent.click(pill("c"), { metaKey: true });
        expect(isPaneTabSelected("c")).toBe(true);
    });

    it("does nothing special in a strip that has no selection (document tabs)", () => {
        const { pill, onActivate } = mount({}, false);
        fireEvent.click(pill("b"), { ctrlKey: true });
        expect(onActivate).toHaveBeenCalledWith("b");
        expect(isPaneTabSelected("b")).toBe(false);
    });
});

describe("a plain click on a pill", () => {
    it("clears the selection, then activates and selects as before", () => {
        const { pill, onActivate, onSelect } = mount();
        fireEvent.click(pill("b"), { ctrlKey: true });
        fireEvent.click(pill("c"), { ctrlKey: true });

        fireEvent.click(pill("c"));

        expect(paneTabSelection().size).toBe(0);
        expect(onSelect).toHaveBeenCalledWith("c");
        expect(onActivate).toHaveBeenCalledWith("c");
    });
});

describe("clearing", () => {
    it("clearPaneTabSelection says whether there was anything to clear", () => {
        expect(clearPaneTabSelection()).toBe(false);
        paneTabSelectionApi.toggle("a");
        expect(clearPaneTabSelection()).toBe(true);
        expect(clearPaneTabSelection()).toBe(false);
    });
});
