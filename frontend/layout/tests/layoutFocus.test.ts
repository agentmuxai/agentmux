// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A tab's first focus skips the agent picker (docs/specs/PLAN_SHORTCUT_KINKS_2026_10_10.md, D3).

import { describe, expect, it, vi } from "vitest";

const blocks = vi.hoisted(() => new Map<string, { meta: Record<string, unknown> }>());
vi.mock("@/app/store/mos", () => ({
    makeORef: (otype: string, oid: string) => `${otype}:${oid}`,
    getObjectValue: (oref: string) => blocks.get(oref.replace(/^block:/, "")),
}));

import { defaultFocusLeaf } from "../lib/layoutFocus";

const leaf = (blockid: string) => ({ nodeid: `n-${blockid}`, blockid });

describe("defaultFocusLeaf", () => {
    it("skips an agent picker for the next pane", () => {
        blocks.set("picker", { meta: { view: "agent" } });
        blocks.set("cpu", { meta: { view: "sysinfo" } });
        expect(defaultFocusLeaf([leaf("picker"), leaf("cpu")])?.blockid).toBe("cpu");
    });

    it("keeps a running agent pane, which isn't a picker", () => {
        blocks.set("agent", { meta: { view: "agent", agentId: "claude" } });
        expect(defaultFocusLeaf([leaf("agent"), leaf("cpu")])?.blockid).toBe("agent");
    });

    it("falls back to the picker when it's the only pane", () => {
        expect(defaultFocusLeaf([leaf("picker")])?.blockid).toBe("picker");
        expect(defaultFocusLeaf([])).toBeUndefined();
    });
});
