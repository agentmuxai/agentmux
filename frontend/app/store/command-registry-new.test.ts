// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * New Tab and New Window are one implementation each: the registry's command, which their keys now reach
 * (SPEC_COMMAND_PALETTE_TOP_CENTRE_2026_11_17.md §9.3 step 4). The keys' own handlers are gone; a host without
 * multiWindow still declines New Window, so its key goes on to the terminal.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    createTab: vi.fn(),
    openNewWindowFromHere: vi.fn(() => Promise.resolve()),
    multiWindow: true,
}));
vi.mock("@/app/store/tab-actions", () => ({ createTab: h.createTab }));
vi.mock("./window-appearance", () => ({ openNewWindowFromHere: h.openNewWindowFromHere }));
vi.mock("@/util/host-caps", () => ({ hostHas: (cap: string) => (cap === "multiWindow" ? h.multiWindow : true) }));

import { commandRegistry, registerDefaultCommands } from "./command-registry";
import { keyCommands } from "./keymodel-dispatch";

describe("New Tab and New Window", () => {
    registerDefaultCommands();
    beforeEach(() => {
        h.createTab.mockClear();
        h.openNewWindowFromHere.mockClear();
        h.multiWindow = true;
    });

    it("New Tab creates a tab", async () => {
        expect(commandRegistry.run("tab:new")).toBe(true);
        await vi.waitFor(() => expect(h.createTab).toHaveBeenCalledOnce());
    });

    it("New Window opens a window from here, and is declined without multiWindow so its key goes on", async () => {
        expect(commandRegistry.run("window:new")).toBe(true);
        await vi.waitFor(() => expect(h.openNewWindowFromHere).toHaveBeenCalledOnce());
        h.multiWindow = false;
        h.openNewWindowFromHere.mockClear();
        expect(commandRegistry.run("window:new")).toBe(false);
        await Promise.resolve();
        expect(h.openNewWindowFromHere).not.toHaveBeenCalled();
    });

    it("have no key handler of their own: the keys reach the registry's commands", async () => {
        const { registerGlobalKeys } = await import("./keymodel");
        keyCommands.clear();
        registerGlobalKeys();
        expect(keyCommands.has("tab:new")).toBe(false);
        expect(keyCommands.has("window:new")).toBe(false);
    });
});
