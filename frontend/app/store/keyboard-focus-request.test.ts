// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

const openOrFocusPaneIdByView = vi.hoisted(() => vi.fn(async () => "settings-1"));
vi.mock("./block-component-registry", () => ({ openOrFocusPaneIdByView }));

import { clearKeyboardRowRequest, keyboardRowRequest, openKeyboardSettings } from "./keyboard-focus-request";

describe("openKeyboardSettings", () => {
    it("goes to a Settings pane open in another tab instead of opening a second one", async () => {
        await openKeyboardSettings("app:newtab");
        expect(openOrFocusPaneIdByView).toHaveBeenCalledWith(
            "settings",
            { meta: { view: "settings", "settings:section": "keyboard" } },
            { "settings:section": "keyboard" },
            { switchTabs: true }
        );
    });

    it("asks the pane it opened, and only that pane, to show the command", async () => {
        clearKeyboardRowRequest();
        await openKeyboardSettings("app:newtab");
        expect(keyboardRowRequest()).toEqual({ command: "app:newtab", blockId: "settings-1" });
    });
});
