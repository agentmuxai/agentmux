// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

const blocks: Record<string, unknown> = {};
vi.mock("@/store/mos", () => ({
    makeORef: (type: string, id: string) => `${type}:${id}`,
    getMuxObjectAtom: (oref: string) => () => blocks[oref],
}));

import { blockAgentName } from "./block-agent-name";

describe("blockAgentName", () => {
    it("reads the name from the block's meta when asked, so a name that arrives later is found", () => {
        const name = blockAgentName("b1");
        expect(name()).toBeUndefined();
        blocks["block:b1"] = { meta: { agentName: "Agent2" } };
        expect(name()).toBe("Agent2");
    });

    it("says nothing without a block id", () => {
        expect(blockAgentName(undefined)()).toBeUndefined();
    });
});
