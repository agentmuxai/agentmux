// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
import { beforeEach, describe, expect, it, vi } from "vitest";

const blocks = vi.hoisted(() => new Map<string, { meta?: Record<string, unknown> }>());
const failing = vi.hoisted(() => new Set<string>());
const reveal = vi.hoisted(() => vi.fn());
const openAgent = vi.hoisted(() => vi.fn());
const push = vi.hoisted(() => vi.fn());

vi.mock("@/app/store/global", () => ({
    MOS: {
        makeORef: (t: string, id: string) => `${t}:${id}`,
        getObjectValue: (oref: string) => blocks.get(oref.split(":")[1]),
        reloadMuxObject: async (oref: string) => {
            if (failing.has(oref.split(":")[1])) throw new Error("no such block");
            return blocks.get(oref.split(":")[1]);
        },
    },
    pushNotification: push,
}));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: reveal }));
vi.mock("@/app/store/agent-runtime-live", () => ({ openAgentInActiveTab: openAgent }));

import { goToAttention, isParkedItem } from "./attention-go";

beforeEach(() => {
    blocks.clear();
    failing.clear();
    reveal.mockReset();
    openAgent.mockReset();
    push.mockReset();
});

// SPEC_ATTENTION_TOAST_WITH_LINK_TO_AGENT_2026_11_02.md §3.4.
describe("goToAttention", () => {
    it("reveals the pane of an agent that has one", async () => {
        blocks.set("b1", { meta: {} });
        await goToAttention({ blockId: "b1", name: "Korp", agentId: "korp" });
        expect(reveal).toHaveBeenCalledWith("b1", { source: "attention" });
        expect(openAgent).not.toHaveBeenCalled();
    });

    it("opens an agent that runs with no pane, which has nothing to reveal", async () => {
        blocks.set("b1", { meta: { "agent:parked": true } });
        openAgent.mockResolvedValue(undefined);
        await goToAttention({ blockId: "b1", name: "Korp", agentId: "korp" });
        expect(openAgent).toHaveBeenCalledWith({ agent_id: "korp", name: "Korp" });
        expect(reveal).not.toHaveBeenCalled();
        expect(push).not.toHaveBeenCalled();
    });

    it("says why when opening is refused", async () => {
        blocks.set("b1", { meta: { "agent:parked": true } });
        openAgent.mockResolvedValue("Korp is open in another window");
        await goToAttention({ blockId: "b1", name: "Korp", agentId: "korp" });
        expect(push).toHaveBeenCalledWith(
            expect.objectContaining({ type: "error", title: "Couldn't open Korp", message: "Korp is open in another window" })
        );
    });

    it("reveals a pane it has no agent for, whatever the block says", async () => {
        blocks.set("b1", { meta: { "agent:parked": true } });
        await goToAttention({ blockId: "b1", name: "An agent" });
        expect(reveal).toHaveBeenCalledWith("b1", { source: "attention" });
    });

    it("reveals when the block cannot be loaded, instead of failing the click", async () => {
        failing.add("gone");
        await goToAttention({ blockId: "gone", name: "Korp", agentId: "korp" });
        expect(reveal).toHaveBeenCalledWith("gone", { source: "attention" });
        expect(openAgent).not.toHaveBeenCalled();
    });

    it("labels a parked agent from what this window already has, and only when the click would open it", () => {
        blocks.set("b1", { meta: { "agent:parked": true } });
        blocks.set("b2", { meta: {} });
        expect(isParkedItem({ blockId: "b1", agentId: "korp" })).toBe(true);
        expect(isParkedItem({ blockId: "b1" })).toBe(false);
        expect(isParkedItem({ blockId: "b2", agentId: "korp" })).toBe(false);
        expect(isParkedItem({ blockId: "unknown", agentId: "korp" })).toBe(false);
    });
});
