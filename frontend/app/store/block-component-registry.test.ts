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

import { createRoot } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";
import {
    getAllBlockComponentModelEntries,
    getAllBlockComponentModels,
    isBlockDormant,
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

// SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md: added alongside agent
// keep-alive so a component (AgentQuestionPanel's auto-timeout,
// useAgentFailure's auto-retry) can reactively pause while its own tab is
// backgrounded, instead of only being queryable via the plain-Set snapshot
// `getAllBlockComponentModels` already used.
describe("isBlockDormant", () => {
    it("defaults to false for a blockId never marked dormant", () => {
        createRoot((dispose) => {
            expect(isBlockDormant("never-marked")()).toBe(false);
            dispose();
        });
    });

    it("reactively flips when setKeepAliveBlockDormant is called for that blockId", () => {
        createRoot((dispose) => {
            const dormant = isBlockDormant("b1");
            expect(dormant()).toBe(false);

            setKeepAliveBlockDormant("b1", true);
            expect(dormant()).toBe(true);

            setKeepAliveBlockDormant("b1", false);
            expect(dormant()).toBe(false);

            dispose();
        });
    });

    it("tracks each blockId independently", () => {
        createRoot((dispose) => {
            const b1Dormant = isBlockDormant("b1");
            const b2Dormant = isBlockDormant("b2");

            setKeepAliveBlockDormant("b1", true);
            expect(b1Dormant()).toBe(true);
            expect(b2Dormant()).toBe(false);

            dispose();
        });
    });

    it("resets to false after unregisterBlockComponentModel, not stuck at a stale true", () => {
        createRoot((dispose) => {
            registerBlockComponentModel("b1", { viewModel: { viewType: "agent" } } as any);
            const dormant = isBlockDormant("b1");
            setKeepAliveBlockDormant("b1", true);
            expect(dormant()).toBe(true);

            unregisterBlockComponentModel("b1");

            // A fresh read after unregistration — the accessor captured
            // above was for the OLD signal instance (deleted alongside the
            // rest of this blockId's bookkeeping); a new one is created
            // false, matching a blockId that was never marked.
            expect(isBlockDormant("b1")()).toBe(false);
            dispose();
        });
    });
});
