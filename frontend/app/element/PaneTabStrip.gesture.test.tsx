// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which pill gestures expand a collapsed pane (`onSelect`): a plain click only. A drag, the click a
 * drag leaves behind, a modifier-click and a right-click never do.
 * docs/specs/SPEC_PANE_TAB_MULTI_SELECT_MOVE_AND_COLLAPSED_EXPAND_2026_10_24.md §6.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setPlatform } from "@/util/platformutil";
import { PaneTabStrip } from "./PaneTabStrip";
import { markPillDragEnded, resetPillDragClock } from "./pane-tab-gesture";
import { paneTabSelectionApi, resetPaneTabSelection } from "./pane-tab-selection";

interface T {
    id: string;
    label: string;
}
const TABS: T[] = [
    { id: "a", label: "alpha" },
    { id: "b", label: "beta" },
];

function mount() {
    const onActivate = vi.fn();
    const onSelect = vi.fn();
    const result = render(() => (
        <PaneTabStrip
            tabs={TABS}
            activeId="a"
            getId={(t: T) => t.id}
            getLabel={(t: T) => t.label}
            onActivate={onActivate}
            onSelect={onSelect}
            onReorder={() => true}
            paneKey="pane-1"
            selection={paneTabSelectionApi}
        />
    ));
    const pill = (id: string) => result.container.querySelector<HTMLElement>(`.pane-tab[data-tab-id="${id}"]`)!;
    return { pill, onActivate, onSelect };
}

beforeEach(() => {
    setPlatform("win32");
    resetPaneTabSelection();
    resetPillDragClock();
});
afterEach(() => {
    cleanup();
    resetPaneTabSelection();
    resetPillDragClock();
});

describe("what expands a collapsed pane", () => {
    it("a plain click does, active pill or not", () => {
        const { pill, onSelect, onActivate } = mount();
        fireEvent.mouseDown(pill("b"), { clientX: 20, clientY: 10 });
        fireEvent.click(pill("b"), { clientX: 20, clientY: 10 });
        expect(onSelect).toHaveBeenCalledWith("b");
        expect(onActivate).toHaveBeenCalledWith("b");

        fireEvent.click(pill("a"));
        expect(onSelect).toHaveBeenCalledWith("a");
    });

    it("a press that travels past the drag threshold before release does not", () => {
        const { pill, onSelect, onActivate } = mount();
        fireEvent.mouseDown(pill("b"), { clientX: 20, clientY: 10 });
        fireEvent.click(pill("b"), { clientX: 60, clientY: 10 });
        expect(onSelect).not.toHaveBeenCalled();
        expect(onActivate).not.toHaveBeenCalled();
    });

    it("a slip inside the threshold is still a click", () => {
        const { pill, onSelect } = mount();
        fireEvent.mouseDown(pill("b"), { clientX: 20, clientY: 10 });
        fireEvent.click(pill("b"), { clientX: 23, clientY: 11 });
        expect(onSelect).toHaveBeenCalledWith("b");
    });

    it("the click a drag leaves behind, on any pill, does not", () => {
        const { pill, onSelect, onActivate } = mount();
        markPillDragEnded();
        fireEvent.click(pill("b"));
        expect(onSelect).not.toHaveBeenCalled();
        expect(onActivate).not.toHaveBeenCalled();
    });

    it("a click long after a drag does", () => {
        const { pill, onSelect } = mount();
        markPillDragEnded(Date.now() - 1_000);
        fireEvent.click(pill("b"));
        expect(onSelect).toHaveBeenCalledWith("b");
    });

    it("Ctrl+click only picks the pill", () => {
        const { pill, onSelect, onActivate } = mount();
        fireEvent.click(pill("b"), { ctrlKey: true });
        expect(onSelect).not.toHaveBeenCalled();
        expect(onActivate).not.toHaveBeenCalled();
    });

    it("on macOS Cmd+click only picks the pill", () => {
        setPlatform("darwin");
        const { pill, onSelect } = mount();
        fireEvent.click(pill("b"), { metaKey: true });
        expect(onSelect).not.toHaveBeenCalled();
    });

    it("Shift+click activates the tab and expands nothing", () => {
        const { pill, onSelect, onActivate } = mount();
        fireEvent.click(pill("b"), { shiftKey: true });
        expect(onSelect).not.toHaveBeenCalled();
        expect(onActivate).toHaveBeenCalledWith("b");
    });

    it("a right-click does not", () => {
        const { pill, onSelect, onActivate } = mount();
        fireEvent.click(pill("b"), { button: 2 });
        fireEvent.mouseDown(pill("b"), { button: 2 });
        expect(onSelect).not.toHaveBeenCalled();
        expect(onActivate).not.toHaveBeenCalled();
    });

    it("the press of one gesture does not leak into the next click", () => {
        const { pill, onSelect } = mount();
        fireEvent.mouseDown(pill("b"), { clientX: 20, clientY: 10 });
        fireEvent.click(pill("b"), { clientX: 80, clientY: 10 }); // a drag-sized move: ignored
        fireEvent.click(pill("b"), { clientX: 80, clientY: 10 }); // a click with no new press: plain
        expect(onSelect).toHaveBeenCalledTimes(1);
    });
});
