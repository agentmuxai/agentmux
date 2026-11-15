// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * When the layout a drag began in shows the dragged pane itself (homeLayout.ts), so that the hand carries no
 * picture of it: on its own window tab, and not while the pane is in no place there.
 */

import { describe, expect, it, vi } from "vitest";

const active = vi.hoisted(() => ({ tabId: "tab-1" }));
vi.mock("@/store/global", () => ({ atoms: { activeTabId: () => active.tabId } }));

import { homeLayoutOf } from "./homeLayout";
import type { LayoutModel } from "./layoutModel";

function layout(state: { tearOffLayout?: boolean; tearOffOffered?: boolean } = {}) {
    return {
        displayContainerRef: { current: null },
        tabAtom: () => ({ oid: "tab-1" }),
        tearOffLayout: () => (state.tearOffLayout ? {} : undefined),
        tearOffOffered: () => !!state.tearOffOffered,
    } as unknown as LayoutModel;
}

describe("the layout a pane's drag began in", () => {
    it("shows the pane itself while a drop or its own place is shown, on its own window tab", () => {
        active.tabId = "tab-1";
        expect(homeLayoutOf(layout(), () => true).holdsPane()).toBe(true);
        active.tabId = "tab-2";
        expect(homeLayoutOf(layout(), () => true).holdsPane()).toBe(false);
        active.tabId = "tab-1";
    });

    it("does not while the pane's own place offers the tear-off: the hand carries the picture over the red place", () => {
        const state = { tearOffOffered: true };
        const home = homeLayoutOf(layout(state), () => true);
        expect(home.holdsPane()).toBe(false);
        // The layout has shown a drop: the offer is over, and the pane stands where the drop leaves it.
        state.tearOffOffered = false;
        expect(home.holdsPane()).toBe(true);
    });

    it("does not while it shows the tear-off", () => {
        expect(homeLayoutOf(layout({ tearOffLayout: true }), () => true).holdsPane()).toBe(false);
    });
});
