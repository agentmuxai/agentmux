// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * splitBlockDefFor — one rule for every split path.
 * SPEC_AGENT_PANE_SPLIT_OPENS_PICKER_2026_09_30.md.
 */

import { describe, expect, it, vi } from "vitest";
import { registerPaneTab } from "./pane-tab-registry";
import { stubPaneTab } from "./pane-tab-test-utils";
import { splitBlockDefFor } from "./split-block-def";

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

    it("no source block (nothing focused) gets the fallback", () => {
        const term = { meta: { view: "term" } };
        expect(splitBlockDefFor(undefined, () => term)).toBe(term);
    });
});
