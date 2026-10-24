// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it } from "vitest";
import {
    clearPaneTabSelection,
    dragIdsFor,
    isPaneTabSelected,
    paneTabSelection,
    prunePaneTabSelection,
    resetPaneTabSelection,
    togglePaneTabSelected,
} from "./pane-tab-selection";

beforeEach(() => resetPaneTabSelection());

describe("the Pane Tab selection", () => {
    it("toggles a block in and out", () => {
        togglePaneTabSelected("a");
        expect(isPaneTabSelected("a")).toBe(true);
        togglePaneTabSelected("a");
        expect(isPaneTabSelected("a")).toBe(false);
    });

    it("prunes blocks that are no longer in the layout on screen, and only those", () => {
        togglePaneTabSelected("a");
        togglePaneTabSelected("b");
        togglePaneTabSelected("c");
        prunePaneTabSelection(new Set(["a", "c", "z"]));
        expect([...paneTabSelection()].sort()).toEqual(["a", "c"]);
    });

    it("pruning leaves the same set object when nothing is dropped, so nothing re-renders", () => {
        togglePaneTabSelected("a");
        const before = paneTabSelection();
        prunePaneTabSelection(new Set(["a", "b"]));
        expect(paneTabSelection()).toBe(before);
    });

    it("clears, and says whether there was anything to clear", () => {
        expect(clearPaneTabSelection()).toBe(false);
        togglePaneTabSelected("a");
        expect(clearPaneTabSelection()).toBe(true);
        expect(paneTabSelection().size).toBe(0);
    });

    describe("what a drag carries", () => {
        it("is the whole selection when the grabbed pill is in it and there is more than one", () => {
            togglePaneTabSelected("a");
            togglePaneTabSelected("b");
            expect([...dragIdsFor("a")].sort()).toEqual(["a", "b"]);
        });

        it("is only the pill when it is not selected, whatever else is", () => {
            togglePaneTabSelected("a");
            togglePaneTabSelected("b");
            expect(dragIdsFor("c")).toEqual(["c"]);
        });

        it("is only the pill when it is the one selected tab", () => {
            togglePaneTabSelected("a");
            expect(dragIdsFor("a")).toEqual(["a"]);
        });
    });
});
