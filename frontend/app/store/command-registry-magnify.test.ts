// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Toggle Magnify is one implementation: the registry's command, which its key now reaches
 * (SPEC_COMMAND_PALETTE_TOP_CENTRE_2026_11_17.md §9.3 step 4). The key's own handler is gone.
 */
import { describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    magnifyNodeToggle: vi.fn(),
    focused: { id: "node-1" } as { id: string } | null,
}));
vi.mock("@/layout/index", () => ({
    NavigateDirection: {},
    getLayoutModelForStaticTab: () => ({ focusedNode: () => h.focused, magnifyNodeToggle: h.magnifyNodeToggle }),
}));

import { commandRegistry, registerDefaultCommands } from "./command-registry";
import { keyCommands } from "./keymodel-dispatch";

describe("Toggle Magnify", () => {
    registerDefaultCommands();

    it("Toggle Magnify magnifies the focused pane, and does nothing with none focused", async () => {
        h.magnifyNodeToggle.mockClear();
        expect(commandRegistry.run("pane:magnify")).toBe(true);
        await vi.waitFor(() => expect(h.magnifyNodeToggle).toHaveBeenCalledExactlyOnceWith("node-1"));
        h.magnifyNodeToggle.mockClear();
        h.focused = null;
        commandRegistry.run("pane:magnify");
        await Promise.resolve();
        expect(h.magnifyNodeToggle).not.toHaveBeenCalled();
        h.focused = { id: "node-1" };
    });

    it("has no key handler of its own: the key reaches the registry's command", async () => {
        const { registerGlobalKeys } = await import("./keymodel");
        keyCommands.clear();
        registerGlobalKeys();
        expect(keyCommands.has("pane:magnify")).toBe(false);
    });
});
