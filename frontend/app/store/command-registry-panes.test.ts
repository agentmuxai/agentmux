// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Settings and Toolchain left the ☰ menu (SPEC_WORKSPACES_2026_10_23.md §3):
 * the command palette must still open them.
 */

import { describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ openOrFocusPaneByView: vi.fn(() => Promise.resolve()) }));
vi.mock("@/app/store/block-component-registry", () => ({ openOrFocusPaneByView: h.openOrFocusPaneByView }));

import { commandRegistry, registerDefaultCommands } from "./command-registry";

describe("palette commands for panes that left the ☰ menu", () => {
    registerDefaultCommands();

    it.each([
        ["app:settings", "settings"],
        ["open:toolchain", "toolchain"],
    ])("%s opens the %s pane", async (id, view) => {
        h.openOrFocusPaneByView.mockClear();
        expect(commandRegistry.run(id)).toBe(true);
        await Promise.resolve();
        expect(h.openOrFocusPaneByView).toHaveBeenCalledWith(view);
    });
});
