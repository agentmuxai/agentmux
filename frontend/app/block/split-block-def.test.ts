// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * splitBlockDefFor — one rule for every split path.
 * SPEC_AGENT_PANE_SPLIT_OPENS_PICKER_2026_09_30.md.
 */

import { describe, expect, it, vi } from "vitest";
import { registerPaneTab } from "./pane-tab-registry";
import { stubPaneTab } from "./pane-tab-test-utils";
import { shortcutsSplitBlockDef, splitBlockDefFor } from "./split-block-def";

const PICKER = { meta: { view: "pickerview" } };
registerPaneTab(stubPaneTab("pickerview", { capabilities: { splitBlockDef: () => PICKER } }));
registerPaneTab(stubPaneTab("plainview"));

const block = (meta: Record<string, unknown>) => ({ oid: "b1", meta }) as unknown as Block;

describe("splitBlockDefFor", () => {
    it("a view that declares splitBlockDef gets it, and the fallback isn't built", () => {
        const fallback = vi.fn(() => ({ meta: { view: "term" } }));
        expect(splitBlockDefFor(block({ view: "pickerview", agentId: "lark" }), fallback)).toBe(PICKER);
        expect(fallback).not.toHaveBeenCalled();
    });

    it("any other view gets the caller's fallback", () => {
        const term = { meta: { view: "term", controller: "shell" } };
        expect(splitBlockDefFor(block({ view: "plainview" }), () => term)).toBe(term);
    });

    it("the view's splitBlockDef is given the pane being split", () => {
        const seen: Block[] = [];
        registerPaneTab(stubPaneTab("settingsview", { capabilities: { splitBlockDef: (src) => (seen.push(src), { meta: { view: "settingsview" } }) } }));
        const source = block({ view: "settingsview", "x:setting": 1 });
        splitBlockDefFor(source, () => ({ meta: { view: "term" } }));
        expect(seen).toEqual([source]);
    });

    it("no source block (nothing focused) gets the fallback", () => {
        const term = { meta: { view: "term" } };
        expect(splitBlockDefFor(undefined, () => term)).toBe(term);
    });
});

describe("shortcutsSplitBlockDef", () => {
    const fallback = () => ({ meta: { view: "shortcuts" } });

    it("is a Shortcuts pane naming the pane that was split, whatever its view declares", () => {
        for (const view of ["pickerview", "plainview", "term"]) {
            expect(shortcutsSplitBlockDef(block({ view }), fallback)).toEqual({ meta: { view: "shortcuts", "shortcuts:from": "b1" } });
        }
    });

    it("splitting a Shortcuts pane passes on the pane it came from, under either name of the view", () => {
        for (const view of ["shortcuts", "launcher"]) {
            expect(shortcutsSplitBlockDef(block({ view, "shortcuts:from": "origin" }), fallback)).toEqual({
                meta: { view: "shortcuts", "shortcuts:from": "origin" },
            });
        }
        // One opened by app:defaultnewblock has no source of its own: the Shortcuts pane is the source.
        expect(shortcutsSplitBlockDef(block({ view: "shortcuts" }), fallback)).toEqual({ meta: { view: "shortcuts", "shortcuts:from": "b1" } });
    });

    it("nothing to split gets the caller's fallback", () => {
        const make = vi.fn(fallback);
        expect(shortcutsSplitBlockDef(undefined, make)).toEqual({ meta: { view: "shortcuts" } });
        expect(make).toHaveBeenCalledOnce();
    });
});
