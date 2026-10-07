// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/block-component-registry", () => ({
    openOrFocusPaneByView: vi.fn(() => Promise.resolve()),
}));

import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import { armoryMigrationPatch, armoryTarget, openConnectors, openMemory } from "./panes";

afterEach(() => vi.mocked(openOrFocusPaneByView).mockClear());

describe("armoryTarget", () => {
    it.each([
        [{}, "connectors", "accounts"],
        [{ "armory:section": "accounts" }, "connectors", "accounts"],
        [{ "armory:section": "mcp" }, "connectors", "mcp"],
        [{ "armory:section": "memory" }, "memory", "global"],
        [{ "armory:section": "memory", "armory:memory:subsection": "global" }, "memory", "global"],
        [{ "armory:section": "memory", "armory:memory:subsection": "personal" }, "memory", "personal"],
        [{ "armory:section": "native_memory" }, "memory", "personal"],
        [{ "armory:section": "skills" }, "memory", "skills"],
        [{ "armory:section": "bundles" }, "memory", "bundles"],
        [{ "armory:section": "identities" }, "connectors", "accounts"],
    ])("%j → %s / %s", (meta, view, section) => {
        expect(armoryTarget(meta)).toMatchObject({ view, section });
    });

    it("treats missing meta as the Armory's default, Accounts", () => {
        expect(armoryTarget(undefined)).toMatchObject({ view: "connectors", section: "accounts" });
    });
});

describe("armoryMigrationPatch", () => {
    it("sets the new view and section and clears the Armory's keys, nothing else", () => {
        expect(armoryMigrationPatch({ view: "trust", "armory:section": "mcp", "term:zoom": 1.2 })).toEqual({
            view: "connectors",
            "connectors:section": "mcp",
            "armory:section": null,
            "armory:memory:subsection": null,
        });
    });
});

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
