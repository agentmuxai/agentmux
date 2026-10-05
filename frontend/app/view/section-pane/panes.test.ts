// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/block-component-registry", () => ({
    openOrFocusPaneByView: vi.fn(() => Promise.resolve()),
}));

import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import { armoryMigrationPatch, armoryTarget, openConnectors, openKnowledge } from "./panes";

afterEach(() => vi.mocked(openOrFocusPaneByView).mockClear());

describe("armoryTarget", () => {
    it.each([
        [{}, "connectors", "accounts"],
        [{ "armory:section": "accounts" }, "connectors", "accounts"],
        [{ "armory:section": "mcp" }, "connectors", "mcp"],
        [{ "armory:section": "memory" }, "knowledge", "global"],
        [{ "armory:section": "memory", "armory:memory:subsection": "global" }, "knowledge", "global"],
        [{ "armory:section": "memory", "armory:memory:subsection": "personal" }, "knowledge", "personal"],
        [{ "armory:section": "native_memory" }, "knowledge", "personal"],
        [{ "armory:section": "skills" }, "knowledge", "skills"],
        [{ "armory:section": "bundles" }, "knowledge", "bundles"],
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

describe("openConnectors / openKnowledge", () => {
    it("open or focus the pane on the given section", async () => {
        await openConnectors("mcp");
        expect(openOrFocusPaneByView).toHaveBeenLastCalledWith(
            "connectors",
            { meta: { view: "connectors", "connectors:section": "mcp" } },
            { "connectors:section": "mcp" }
        );
        await openKnowledge("bundles");
        expect(openOrFocusPaneByView).toHaveBeenLastCalledWith(
            "knowledge",
            { meta: { view: "knowledge", "knowledge:section": "bundles" } },
            { "knowledge:section": "bundles" }
        );
    });

    it("default to Accounts and Global", async () => {
        await openConnectors();
        expect(vi.mocked(openOrFocusPaneByView).mock.calls[0][2]).toEqual({ "connectors:section": "accounts" });
        await openKnowledge();
        expect(vi.mocked(openOrFocusPaneByView).mock.calls[1][2]).toEqual({ "knowledge:section": "global" });
    });
});
