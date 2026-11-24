// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Close Tab and Close Pane are one implementation: the key and the palette row run the same
 * function (SPEC_COMMAND_PALETTE_TOP_CENTRE_2026_11_17.md §9.3 step 4). The key's own handler is
 * gone; a key press for these commands falls through to the registry.
 */
import { describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ genericClose: vi.fn(), simpleCloseStaticTab: vi.fn() }));
vi.mock("@/app/store/keymodel-nav", () => h);

import { commandRegistry, registerDefaultCommands } from "./command-registry";
import { keyCommands } from "./keymodel-dispatch";

describe("Close Tab and Close Pane", () => {
    registerDefaultCommands();

    it.each([
        ["tab:close", "simpleCloseStaticTab"],
        ["pane:close", "genericClose"],
    ] as const)("%s runs %s, as its key does", async (id, fn) => {
        h[fn].mockClear();
        expect(commandRegistry.run(id)).toBe(true);
        await vi.waitFor(() => expect(h[fn]).toHaveBeenCalledTimes(1));
    });

    it("have no key handler of their own: the key reaches the registry's command", async () => {
        const { registerGlobalKeys } = await import("./keymodel");
        keyCommands.clear();
        registerGlobalKeys();
        expect(keyCommands.has("tab:close")).toBe(false);
        expect(keyCommands.has("pane:close")).toBe(false);
    });
});
