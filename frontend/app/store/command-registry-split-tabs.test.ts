// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Split Right, Split Down, Next Tab and Previous Tab are one implementation: the key and the palette
 * row run the same function (SPEC_COMMAND_PALETTE_TOP_CENTRE_2026_11_17.md §9.3 step 4). The keys'
 * own handlers are gone; a key press for these commands falls through to the registry.
 */
import { describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    handleSplitHorizontal: vi.fn(),
    handleSplitVertical: vi.fn(),
    switchTab: vi.fn(),
}));
vi.mock("@/app/store/keymodel-blockcreate", () => ({ handleCmdN: vi.fn(), ...h }));
vi.mock("@/app/store/keymodel-nav", () => ({ genericClose: vi.fn(), simpleCloseStaticTab: vi.fn(), switchTab: h.switchTab }));

import { commandRegistry, registerDefaultCommands } from "./command-registry";
import { keyCommands } from "./keymodel-dispatch";

describe("splits and tab steps", () => {
    registerDefaultCommands();

    it.each([
        ["split:right", () => h.handleSplitHorizontal, ["after"]],
        ["split:down", () => h.handleSplitVertical, ["after"]],
        ["tab:next", () => h.switchTab, [1]],
        ["tab:prev", () => h.switchTab, [-1]],
    ] as const)("%s runs what its key ran", async (id, fn, args) => {
        const run = fn();
        run.mockClear();
        expect(commandRegistry.run(id)).toBe(true);
        await vi.waitFor(() => expect(run).toHaveBeenCalledExactlyOnceWith(...args));
    });

    it("have no key handler of their own: the key reaches the registry's command", async () => {
        const { registerGlobalKeys } = await import("./keymodel");
        keyCommands.clear();
        registerGlobalKeys();
        for (const id of ["split:right", "split:down", "tab:next", "tab:prev"]) expect(keyCommands.has(id)).toBe(false);
    });
});
