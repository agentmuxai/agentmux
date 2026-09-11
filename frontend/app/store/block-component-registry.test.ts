// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Regression coverage for Codex P1/P2 + ReAgent P1 on PR #3187: once a pane
 * can keep multiple stack members' `<Block>`s mounted simultaneously
 * (terminal keep-alive, `pane-leaf-chrome.tsx`), every registered blockId
 * is no longer automatically "the one currently-visible pane" — a dormant,
 * hidden stack member registers here too. `getAllBlockComponentModels`/
 * `getAllBlockComponentModelEntries` must still expose only ACTIVE panes,
 * since every consumer (multi-input broadcast, all-panes zoom, the
 * multi-input-eligible terminal count) assumes that — and must do so
 * WITHOUT depending on which tab a block lives in (ReAgent P1: an earlier
 * version resolved this via the active tab's own layout tree, which
 * silently excluded every pane in every background tab too).
 */

import { afterEach, describe, expect, it } from "vitest";
import {
    getAllBlockComponentModelEntries,
    getAllBlockComponentModels,
    registerBlockComponentModel,
    setKeepAliveBlockDormant,
    unregisterBlockComponentModel,
} from "./block-component-registry";

afterEach(() => {
    // No exposed "clear" — unregister everything this suite may have
    // registered so state doesn't leak between tests.
    for (const id of ["b1", "b2", "b3"]) {
        unregisterBlockComponentModel(id);
        setKeepAliveBlockDormant(id, false);
    }
});

describe("getAllBlockComponentModels / getAllBlockComponentModelEntries", () => {
    it("excludes a blockId explicitly marked dormant", () => {
        registerBlockComponentModel("b1", { viewModel: { viewType: "term" } } as any);
        registerBlockComponentModel("b2", { viewModel: { viewType: "term" } } as any);
        setKeepAliveBlockDormant("b2", true);

        expect(getAllBlockComponentModels()).toHaveLength(1);
        expect(getAllBlockComponentModelEntries().map(([id]) => id)).toEqual(["b1"]);
    });

    it("includes a block again once it's un-marked (e.g. it becomes the active tab)", () => {
        registerBlockComponentModel("b1", { viewModel: { viewType: "term" } } as any);
        registerBlockComponentModel("b2", { viewModel: { viewType: "term" } } as any);
        setKeepAliveBlockDormant("b1", true);

        expect(getAllBlockComponentModelEntries().map(([id]) => id)).toEqual(["b2"]);

        // Switch: "b1" becomes active, "b2" becomes dormant.
        setKeepAliveBlockDormant("b1", false);
        setKeepAliveBlockDormant("b2", true);

        expect(getAllBlockComponentModelEntries().map(([id]) => id)).toEqual(["b1"]);
    });

    it("includes every registered block when nothing is marked dormant (every non-keep-alive pane type)", () => {
        registerBlockComponentModel("b1", { viewModel: { viewType: "agent" } } as any);
        registerBlockComponentModel("b2", { viewModel: { viewType: "editor" } } as any);
        registerBlockComponentModel("b3", { viewModel: { viewType: "browser" } } as any);

        expect(getAllBlockComponentModels()).toHaveLength(3);
    });

    // ReAgent P1's actual regression: a filter keyed on "does this blockId
    // resolve in the ACTIVE tab's layout tree" would incorrectly treat
    // every block belonging to a different, background tab as excluded.
    // This suite never touches the layout model at all — proving the fix
    // doesn't depend on it, so a block from any tab is included by default.
    it("includes a registered block with no layout/tab resolution involved at all", () => {
        registerBlockComponentModel("b1", { viewModel: { viewType: "browser" } } as any);
        expect(getAllBlockComponentModels().map((bcm) => bcm.viewModel.viewType)).toEqual(["browser"]);
    });

    it("unregistering a block clears any stale dormancy marker instead of leaking it to a future blockId reuse", () => {
        registerBlockComponentModel("b1", { viewModel: { viewType: "term" } } as any);
        setKeepAliveBlockDormant("b1", true);
        unregisterBlockComponentModel("b1");

        registerBlockComponentModel("b1", { viewModel: { viewType: "term" } } as any);
        expect(getAllBlockComponentModels()).toHaveLength(1);
    });
});
