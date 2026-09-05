// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A fresh agent pane (picker showing, no agent launched) should render no
 * tab strip at all — not even the collapsed "+" box it used to always
 * show. The "+" comes back as soon as the pane is a real conversation, and
 * pills appear as soon as there's a second tab to switch to.
 */

import { describe, expect, it } from "vitest";
import { shouldShowTabStrip } from "./tab-strip-visibility";

describe("shouldShowTabStrip", () => {
    it("hides the strip entirely on a fresh pane (picker, no agent, lone tab)", () => {
        expect(shouldShowTabStrip({ visibleTabCount: 0, hasAgent: false, isHistoryTab: false })).toBe(false);
    });

    it("shows the strip (just the +) once the lone tab has launched an agent", () => {
        expect(shouldShowTabStrip({ visibleTabCount: 0, hasAgent: true, isHistoryTab: false })).toBe(true);
    });

    it("shows the strip for a lone read-only history tab", () => {
        expect(shouldShowTabStrip({ visibleTabCount: 0, hasAgent: false, isHistoryTab: true })).toBe(true);
    });

    // The + on a launched pane opens a SECOND, blank picker tab. That new
    // tab has no agentId of its own — but the strip must stay up, or the
    // user lands on a blank pane with no way back to the first tab.
    it("keeps the strip up when a 2nd tab exists, even with no agent on the active tab", () => {
        expect(shouldShowTabStrip({ visibleTabCount: 2, hasAgent: false, isHistoryTab: false })).toBe(true);
    });

    it("keeps the strip up for a multi-tab pane with an agent", () => {
        expect(shouldShowTabStrip({ visibleTabCount: 3, hasAgent: true, isHistoryTab: false })).toBe(true);
    });
});
