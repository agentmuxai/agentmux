// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "N held" on a Swarm row: the messages typed to an agent while its model API was out
 * (SPEC_OFFLINE_RESILIENCE_2026_10_20.md §4.4a), on a live agent's row and on a stopped agent's.
 */

import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const blocks = vi.hoisted(() => ({ meta: {} as Record<string, any> }));
vi.mock("@/app/store/global", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/store/global")>()),
    MOS: { makeORef: (type: string, id: string) => `${type}:${id}`, getMuxObjectAtom: () => () => ({ meta: blocks.meta }) },
}));

import { StoppedAgents } from "./swarm-stopped-agents";
import { HeldNote, heldCount } from "./swarm-row-notes";

afterEach(cleanup);

const held = (agentId: string, n: number) => ({
    agentId,
    messages: Array.from({ length: n }, (_, i) => ({ id: `m${i}`, text: `text ${i}`, heldAt: i })),
});

describe("heldCount", () => {
    it("counts the messages typed to the agent the block runs", () => {
        expect(heldCount({ agentId: "ann", "agent:held_input": held("ann", 3) })).toBe(3);
    });

    it("counts nothing for another agent's messages, an empty list or no meta", () => {
        expect(heldCount({ agentId: "ann", "agent:held_input": held("bob", 2) })).toBe(0);
        expect(heldCount({ agentId: "ann", "agent:held_input": held("ann", 0) })).toBe(0);
        expect(heldCount({ agentId: "ann", "agent:held_input": { agentId: "ann" } })).toBe(0);
        expect(heldCount({ agentId: "ann" })).toBe(0);
        expect(heldCount(undefined)).toBe(0);
    });
});

describe("HeldNote", () => {
    it("says how many are held, and nothing when none are", () => {
        const { container, unmount } = render(() => <HeldNote meta={{ agentId: "ann", "agent:held_input": held("ann", 2) }} />);
        expect(container.querySelector(".swarm-held")!.textContent).toBe("2 held");
        unmount();
        const none = render(() => <HeldNote meta={{ agentId: "ann" }} />);
        expect(none.container.querySelector(".swarm-held")).toBeNull();
    });
});

describe("a stopped agent's row", () => {
    const agent: AgentRuntime = {
        agent_id: "ann",
        state: "stopped",
        desired: "stopped",
        block_id: "blk",
        name: "Ann",
        provider: "claude",
        icon: "",
    };

    it("shows the messages held for it, which go out when it starts", () => {
        blocks.meta = { agentId: "ann", "agent:held_input": held("ann", 2) };
        const { container } = render(() => <StoppedAgents agents={() => [agent]} hasLive={() => false} onOpen={() => {}} onStart={() => {}} />);
        expect(container.querySelector(".swarm-stopped__row .swarm-held")!.textContent).toBe("2 held");
    });

    it("shows nothing for one with nothing held or no block yet", () => {
        blocks.meta = {};
        const { container } = render(() => (
            <StoppedAgents agents={() => [agent, { ...agent, agent_id: "bob", block_id: undefined }]} hasLive={() => false} onOpen={() => {}} onStart={() => {}} />
        ));
        expect(container.querySelector(".swarm-held")).toBeNull();
    });
});
