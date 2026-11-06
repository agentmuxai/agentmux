// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { attachRunningAgent, paneToRevealFor } from "./attach-target";

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

const h = vi.hoisted(() => ({
    reveal: vi.fn(),
    open: vi.fn(),
    notify: vi.fn(),
    meta: undefined as Record<string, unknown> | undefined,
}));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: h.reveal }));
vi.mock("@/app/store/agent-runtime-live", () => ({ openAgentInActiveTab: h.open }));
vi.mock("@/app/store/global", () => ({
    pushNotification: h.notify,
    MOS: { makeORef: (_t: string, id: string) => id, getMuxObjectAtom: () => () => ({ meta: h.meta }) },
}));

describe("attachRunningAgent", () => {
    const row = { definition_id: "def-1", instance_name: "Tessa", definition_name: "Goose" } as RecentSessionRow;
    beforeEach(() => {
        vi.clearAllMocks();
        h.meta = undefined;
        h.open.mockResolvedValue(undefined);
    });

    it("reveals the pane of an agent that has one, and opens nothing", async () => {
        h.reveal.mockResolvedValue({ ok: true });
        await attachRunningAgent(row, { block_id: "blk-1" });
        expect(h.reveal).toHaveBeenCalledWith("blk-1", { source: "picker" });
        expect(h.open).not.toHaveBeenCalled();
    });

    it("opens it in the active tab when its block is parked", async () => {
        h.meta = { "agent:parked": true };
        await attachRunningAgent(row, { block_id: "blk-1" });
        expect(h.reveal).not.toHaveBeenCalled();
        expect(h.open).toHaveBeenCalledWith({ agent_id: "def-1", name: "Tessa" });
    });

    it("opens it in the active tab when the reveal does not take", async () => {
        h.reveal.mockResolvedValue({ ok: false, reason: "not-found" });
        await attachRunningAgent(row, { block_id: "blk-1" });
        expect(h.reveal).toHaveBeenCalledOnce();
        expect(h.open).toHaveBeenCalledOnce();
    });

    it("says why when srv refuses to open it", async () => {
        h.open.mockResolvedValue("a limit is reached");
        await attachRunningAgent(row, undefined);
        expect(h.notify).toHaveBeenCalledWith(expect.objectContaining({ title: "Couldn't open Tessa", message: "a limit is reached" }));
    });
});
