// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { isInsideHiddenTabContent } from "./use-pane-rect-sync";

function nest(): { tab: HTMLDivElement; pane: HTMLDivElement } {
    const tab = document.createElement("div");
    const middle = document.createElement("div");
    const pane = document.createElement("div");
    tab.appendChild(middle);
    middle.appendChild(pane);
    document.body.appendChild(tab);
    return { tab, pane };
}

describe("isInsideHiddenTabContent", () => {
    it("is false in a shown tab", () => {
        expect(isInsideHiddenTabContent(nest().pane)).toBe(false);
    });

    /** Codex P1 on #3686: a native browser pane composites above the DOM,
     *  so a tab kept laid out and hidden by `visibility` must still collapse it. */
    it("is true inside a tab marked hidden-but-laid-out", () => {
        const { tab, pane } = nest();
        tab.dataset.tabHiddenLaidOut = "true";
        expect(isInsideHiddenTabContent(pane)).toBe(true);
    });

    it("ignores visibility alone, which the reveal gate also sets on the tab being shown", () => {
        const { tab, pane } = nest();
        tab.style.visibility = "hidden";
        expect(isInsideHiddenTabContent(pane)).toBe(false);
    });
});
