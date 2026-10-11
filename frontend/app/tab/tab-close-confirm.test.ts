// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { asksBeforeClosingTab, DONT_ASK_AGAIN } from "./tab-close-confirm";

describe("closing a tab", () => {
    it("closes at once unless the user turned the confirmation on", () => {
        expect(asksBeforeClosingTab(undefined)).toBe(false);
        expect(asksBeforeClosingTab({})).toBe(false);
        expect(asksBeforeClosingTab({ "tab:confirmclose": false })).toBe(false);
        expect(asksBeforeClosingTab({ "tab:confirmclose": true })).toBe(true);
    });

    it("no longer reads the old skip setting either way", () => {
        expect(asksBeforeClosingTab({ "tab:skipcloseconfirm": false })).toBe(false);
        expect(asksBeforeClosingTab({ "tab:skipcloseconfirm": true, "tab:confirmclose": true })).toBe(true);
    });

    it("\"Don't ask again\" turns the confirmation off", () => {
        expect(DONT_ASK_AGAIN).toEqual({ "tab:confirmclose": false });
    });
});
