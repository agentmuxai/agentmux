// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/block-component-registry", () => ({
    openOrFocusPaneByView: vi.fn(() => Promise.resolve()),
}));

import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import { openConnectors, openMemory } from "./panes";

afterEach(() => vi.mocked(openOrFocusPaneByView).mockClear());

describe("openConnectors / openMemory", () => {
    it("open or focus the pane on the given section", async () => {
        await openConnectors("mcp");
        expect(openOrFocusPaneByView).toHaveBeenLastCalledWith(
            "connectors",
            { meta: { view: "connectors", "connectors:section": "mcp" } },
            { "connectors:section": "mcp" }
        );
        await openMemory("bundles");
        expect(openOrFocusPaneByView).toHaveBeenLastCalledWith(
            "memory",
            { meta: { view: "memory", "memory:section": "bundles" } },
            { "memory:section": "bundles" }
        );
    });

    it("default to Accounts and Global", async () => {
        await openConnectors();
        expect(vi.mocked(openOrFocusPaneByView).mock.calls[0][2]).toEqual({ "connectors:section": "accounts" });
        await openMemory();
        expect(vi.mocked(openOrFocusPaneByView).mock.calls[1][2]).toEqual({ "memory:section": "global" });
    });
});
