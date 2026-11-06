// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { paneToRevealFor } from "./attach-target";

describe("paneToRevealFor (the picker's Open it for an agent that runs, #6325)", () => {
    it("is the agent's block when it has a pane, even one in a workspace no window shows", () => {
        expect(paneToRevealFor({ block_id: "blk-1" }, { "agent:id": "a" })).toBe("blk-1");
        expect(paneToRevealFor({ block_id: "blk-1" }, undefined)).toBe("blk-1");
    });

    it("is nothing for a parked block: it has no pane to reveal, the agent opens in the active tab", () => {
        expect(paneToRevealFor({ block_id: "blk-1" }, { "agent:parked": true })).toBeNull();
    });

    it("is nothing for an agent that has never had a block", () => {
        expect(paneToRevealFor({}, undefined)).toBeNull();
        expect(paneToRevealFor(undefined, undefined)).toBeNull();
        expect(paneToRevealFor({ block_id: "" }, undefined)).toBeNull();
    });
});
