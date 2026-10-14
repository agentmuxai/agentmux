// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import {
    CATCH_UP_TURN_PLACEHOLDER_CONTENT,
    createMemoryReinjectionController,
    type ClaimAnswer,
    type ComposedDelivery,
} from "./memory-reinjection-controller";
import type { CatchUpReason } from "./memory-reinjection";
import type { ContextDeliveryNode } from "./types";

// The catch-up turn: SPEC_RESUME_PHASE_CATCH_UP_TURN_2026_10_13.md §3.3.

const NOW = Date.parse("2026-10-14T10:00:00.000Z");
const RECENT = "2026-10-14T09:59:50.000Z";

const card = { type: "context_delivery", id: "memory-injected-fb-1" } as unknown as ContextDeliveryNode;

function make(
    over: {
        working?: () => boolean;
        claim?: (() => Promise<boolean | ClaimAnswer>) | null;
        composeCatchUp?: (reason: CatchUpReason, memory: boolean) => Promise<ComposedDelivery | null>;
        sendRpc?: (message: string, deliveryId?: string) => Promise<void>;
        entries?: { label: string; source: "global" | "personal"; body: string; sizeBytes: number }[];
    } = {},
) {
    const working = vi.fn(over.working ?? (() => false));
    const claimFn = over.claim === null ? undefined : vi.fn(over.claim ?? (async () => true));
    const composeCatchUp = vi.fn(
        over.composeCatchUp ??
            (async (_reason: CatchUpReason, memory: boolean): Promise<ComposedDelivery> =>
                memory ? { deliveryId: "fb-1", text: "memory + catch-up", node: card } : { text: "catch-up only" }),
    );
    const sendRpc = vi.fn(over.sendRpc ?? (async () => undefined));
    const dispatchTurnStart = vi.fn<(content: string, hidden: boolean) => void>();
    const dispatchTurnStartFailed = vi.fn<() => void>();
    const controller = createMemoryReinjectionController({
        contextWindow: () => 10_000,
        now: () => NOW,
        isPaneWorking: working,
        fetchEntries: async () => over.entries ?? [],
        sendRpc,
        composeCatchUp,
        ...(claimFn ? { claimFallback: claimFn } : {}),
        dispatchTurnStart,
        dispatchTurnStartFailed,
    });
    return { controller, working, claimFn, composeCatchUp, sendRpc, dispatchTurnStart, dispatchTurnStartFailed };
}

describe("catchUp: a fresh session on a quiet pane", () => {
    it("claims the memory event, composes with memory, sends it hidden-input and hides nothing", async () => {
        const t = make();
        await t.controller.catchUp(RECENT, "fresh_session");

        expect(t.claimFn).toHaveBeenCalledTimes(1);
        expect(t.composeCatchUp).toHaveBeenCalledWith("fresh_session", true);
        // Hidden input, but a distinct marker: the reply is shown and a suggestion follows it.
        expect(t.dispatchTurnStart).toHaveBeenCalledWith(CATCH_UP_TURN_PLACEHOLDER_CONTENT, true);
        expect(t.sendRpc).toHaveBeenCalledWith("memory + catch-up", "fb-1");
        // The point of a catch-up over a reinjection: the reply is shown.
        expect(t.controller.isHiding()).toBe(false);
    });

    it("leaves the memory out once the hook has delivered it", async () => {
        const t = make({ claim: async () => false });
        await t.controller.catchUp(RECENT, "fresh_session");

        expect(t.composeCatchUp).toHaveBeenCalledWith("fresh_session", false);
        expect(t.sendRpc).toHaveBeenCalledWith("catch-up only", undefined);
    });

    it("still sends when the claim fails: a duplicate beats no memory", async () => {
        const t = make({
            claim: async () => {
                throw new Error("rpc down");
            },
        });
        await t.controller.catchUp(RECENT, "fresh_session");

        expect(t.composeCatchUp).toHaveBeenCalledWith("fresh_session", true);
        expect(t.sendRpc).toHaveBeenCalledTimes(1);
    });

    it("does nothing on a busy pane: the user is already talking to the agent", async () => {
        const t = make({ working: () => true });
        await t.controller.catchUp(RECENT, "fresh_session");

        expect(t.claimFn).not.toHaveBeenCalled();
        expect(t.composeCatchUp).not.toHaveBeenCalled();
        expect(t.sendRpc).not.toHaveBeenCalled();
        expect(t.dispatchTurnStart).not.toHaveBeenCalled();
    });

    it("ignores an outcome that is not recent, and the same outcome seen twice", async () => {
        const stale = make();
        await stale.controller.catchUp("2026-10-14T09:50:00.000Z", "fresh_session");
        expect(stale.sendRpc).not.toHaveBeenCalled();

        const twice = make();
        await twice.controller.catchUp(RECENT, "fresh_session");
        // Its own turn ends; a re-read of the same outcome must not start another.
        twice.controller.onSessionEnd();
        await twice.controller.catchUp(RECENT, "fresh_session");
        expect(twice.sendRpc).toHaveBeenCalledTimes(1);
    });
});

describe("catchUp: a resumed conversation", () => {
    it("claims nothing and composes without memory", async () => {
        const t = make();
        await t.controller.catchUp(null, "resumed");

        expect(t.claimFn).not.toHaveBeenCalled();
        expect(t.composeCatchUp).toHaveBeenCalledWith("resumed", false);
        expect(t.sendRpc).toHaveBeenCalledWith("catch-up only", undefined);
    });
});

describe("catchUp: a manual compaction", () => {
    it("claims at the boundary, even while its own turn is still running, and sends when that turn ends", async () => {
        let busy = true;
        const t = make({ working: () => busy });
        await t.controller.catchUp(RECENT, "compaction", { boundaryUuid: "u1", deferIfBusy: true });

        // Claimed now (the hook delivered moments ago), sent later.
        expect(t.claimFn).toHaveBeenCalledTimes(1);
        expect(t.sendRpc).not.toHaveBeenCalled();

        busy = false;
        t.controller.maybeFireDeferred();
        await vi.waitFor(() => expect(t.sendRpc).toHaveBeenCalledTimes(1));
        // The claim was kept, not asked again.
        expect(t.claimFn).toHaveBeenCalledTimes(1);
        expect(t.composeCatchUp).toHaveBeenCalledWith("compaction", true);
    });

    it("does nothing for a compaction another block's CLI wrote", async () => {
        const t = make({ claim: async () => ({ deliver: false, foreign: true }) });
        await t.controller.catchUp(RECENT, "compaction", { boundaryUuid: "u1", deferIfBusy: true });

        expect(t.composeCatchUp).not.toHaveBeenCalled();
        expect(t.sendRpc).not.toHaveBeenCalled();
    });

    it("catches up once per boundary", async () => {
        const t = make();
        await t.controller.catchUp(RECENT, "compaction", { boundaryUuid: "u1", deferIfBusy: true });
        t.controller.onSessionEnd();
        await t.controller.catchUp(RECENT, "compaction", { boundaryUuid: "u1", deferIfBusy: true });

        expect(t.sendRpc).toHaveBeenCalledTimes(1);
    });
});

describe("catchUp: the turn's lifetime", () => {
    it("does not stack a second catch-up while one is running, and hands back its card at the end", async () => {
        const t = make();
        await t.controller.catchUp(RECENT, "fresh_session");
        await t.controller.catchUp("2026-10-14T09:59:55.000Z", "fresh_session");
        expect(t.sendRpc).toHaveBeenCalledTimes(1);

        expect(t.controller.onSessionEnd()).toBe(card);
        // Over: the next session_end is an ordinary turn's.
        expect(t.controller.onSessionEnd()).toBeNull();
    });

    it("sends nothing and marks no turn when srv cannot compose it", async () => {
        const t = make({
            composeCatchUp: async () => {
                throw new Error("older srv");
            },
        });
        await t.controller.catchUp(RECENT, "fresh_session");

        expect(t.dispatchTurnStart).not.toHaveBeenCalled();
        expect(t.sendRpc).not.toHaveBeenCalled();
    });

    it("reverts the turn when the send fails, and can catch up again", async () => {
        let fail = true;
        const t = make({
            sendRpc: async () => {
                if (fail) throw new Error("send failed");
            },
        });
        await t.controller.catchUp(RECENT, "resumed");

        expect(t.dispatchTurnStartFailed).toHaveBeenCalledTimes(1);
        expect(t.controller.onSessionEnd()).toBeNull();

        fail = false;
        await t.controller.catchUp(null, "resumed");
        expect(t.sendRpc).toHaveBeenCalledTimes(2);
    });

    it("after a failed send lets the same outcome try again, and still owes the memory it claimed", async () => {
        let fail = true;
        const t = make({
            entries: [{ label: "g1", source: "global", body: "global body", sizeBytes: 11 }],
            sendRpc: async () => {
                if (fail) throw new Error("send failed");
            },
        });
        await t.controller.catchUp(RECENT, "fresh_session");
        expect(t.dispatchTurnStartFailed).toHaveBeenCalledTimes(1);

        // The hook stood down for the claim, so the memory goes out as a plain
        // reinjection at the next quiet.
        fail = false;
        t.controller.maybeFireDeferred();
        await vi.waitFor(() => expect(t.sendRpc).toHaveBeenCalledTimes(2));
        expect(t.sendRpc.mock.calls[1][0]).toContain("global body");
        t.controller.onSessionEnd();

        // And the outcome itself is not spent: a second sighting catches up.
        await t.controller.catchUp(RECENT, "fresh_session");
        expect(t.composeCatchUp).toHaveBeenCalledTimes(2);
    });

    it("drops a fresh-session catch-up when a turn starts while srv is composing", async () => {
        let busy = false;
        const t = make({
            working: () => busy,
            composeCatchUp: async (_reason, memory) => {
                busy = true; // the user sent something during the round trip
                return memory ? { text: "x", deliveryId: "d" } : { text: "y" };
            },
        });
        await t.controller.catchUp(RECENT, "fresh_session");
        busy = false;
        t.controller.maybeFireDeferred();

        // The user is already talking to the agent: nothing to catch up on, now or later.
        expect(t.sendRpc).not.toHaveBeenCalled();
        expect(t.dispatchTurnStart).not.toHaveBeenCalled();
    });

    it("still delivers the memory it claimed, as a plain reinjection, when a turn starts during the claim", async () => {
        let busy = false;
        const t = make({
            working: () => busy,
            entries: [{ label: "g1", source: "global", body: "global body", sizeBytes: 11 }],
            claim: async () => {
                busy = true; // the user sent something while srv was answering
                return true;
            },
        });
        await t.controller.catchUp(RECENT, "fresh_session");
        // Not a catch-up: the user is already talking to the agent.
        expect(t.composeCatchUp).not.toHaveBeenCalled();
        expect(t.sendRpc).not.toHaveBeenCalled();

        // The claim made the hook stand down, so the memory goes out once the pane is quiet.
        busy = false;
        t.controller.maybeFireDeferred();
        await vi.waitFor(() => expect(t.sendRpc).toHaveBeenCalledTimes(1));
        expect(t.claimFn).toHaveBeenCalledTimes(1);
        expect(t.sendRpc.mock.calls[0][0]).toContain("global body");
        expect(t.controller.isHiding()).toBe(true);
    });

    it("keeps a manual compaction's catch-up for when the turn that started ends", async () => {
        let busy = false;
        let first = true;
        const t = make({
            working: () => busy,
            composeCatchUp: async (_reason, memory) => {
                if (first) {
                    first = false;
                    busy = true;
                }
                return memory ? { text: "x", deliveryId: "d" } : { text: "y" };
            },
        });
        await t.controller.catchUp(RECENT, "compaction", { boundaryUuid: "u1", deferIfBusy: true });
        expect(t.sendRpc).not.toHaveBeenCalled();

        busy = false;
        t.controller.maybeFireDeferred();
        await vi.waitFor(() => expect(t.sendRpc).toHaveBeenCalledTimes(1));
    });
});
