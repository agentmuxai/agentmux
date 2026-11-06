// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import type { ContextDeliveryNode, MemoryReinjectionNode } from "../../agent-model/types";
import type { CatchUpReason, MemoryEntryInput } from "./memory-reinjection";
import {
    createMemoryReinjectionController,
    type ComposedDelivery,
    type MemoryReinjectionControllerOpts,
} from "./memory-reinjection-controller";

// The paths the other two controller test files leave open: races between two
// triggers, flags that must be released on every exit, and the card's fields.

const NOW = Date.parse("2026-10-14T10:00:00.000Z");
const RECENT = "2026-10-14T09:59:50.000Z";
const ENTRY: MemoryEntryInput = { label: "g1", source: "global", body: "global body", sizeBytes: 11 };
const card = { type: "context_delivery", id: "memory-injected-fb-1" } as unknown as ContextDeliveryNode;

/** Lets every pending continuation run, so a "nothing happened" check is not just early. */
const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

function gate<T>() {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>((r) => {
        resolve = r;
    });
    return { promise, resolve };
}

const defaultCompose = async (_reason: CatchUpReason, memory: boolean): Promise<ComposedDelivery> =>
    memory ? { deliveryId: "fb-1", text: "memory + catch-up", node: card } : { text: "catch-up only" };

function build(over: Partial<MemoryReinjectionControllerOpts> = {}, state = { busy: false }) {
    const fetchEntries = vi.fn<() => Promise<MemoryEntryInput[]>>(over.fetchEntries ?? (async () => [ENTRY]));
    const sendRpc = vi.fn<(message: string, deliveryId?: string) => Promise<void>>(
        over.sendRpc ?? (async () => undefined)
    );
    const dispatchTurnStart = vi.fn<(content: string, hidden: boolean) => void>(over.dispatchTurnStart ?? (() => {}));
    const dispatchTurnStartFailed = vi.fn<() => void>();
    const claim = over.claimFallback ? vi.fn(over.claimFallback) : undefined;
    const composeCatchUp = over.composeCatchUp ? vi.fn(over.composeCatchUp) : undefined;
    const controller = createMemoryReinjectionController({
        contextWindow: () => 10_000,
        now: () => NOW,
        isPaneWorking: () => state.busy,
        ...over,
        fetchEntries,
        sendRpc,
        dispatchTurnStart,
        dispatchTurnStartFailed,
        ...(claim ? { claimFallback: claim } : {}),
        ...(composeCatchUp ? { composeCatchUp } : {}),
    });
    return {
        controller,
        state,
        fetchEntries,
        sendRpc,
        dispatchTurnStart,
        dispatchTurnStartFailed,
        claim,
        composeCatchUp,
    };
}

const memoryNode = (node: unknown) => node as MemoryReinjectionNode;

describe("boundaries a controller remembers", () => {
    it("keeps the last 64 and forgets older ones", async () => {
        const { controller, fetchEntries } = build({ fetchEntries: async () => [] });
        for (let i = 1; i <= 65; i++) await controller.trigger(null, "compaction", `u${i}`);
        expect(fetchEntries).toHaveBeenCalledTimes(65);

        // u2 and u65 are still remembered: seen again, ignored.
        await controller.trigger(null, "compaction", "u2");
        await controller.trigger(null, "compaction", "u65");
        expect(fetchEntries).toHaveBeenCalledTimes(65);

        // u1 was pushed out by the 65th: a new sighting.
        await controller.trigger(null, "compaction", "u1");
        expect(fetchEntries).toHaveBeenCalledTimes(66);
    });
});

describe("the reinjection card", () => {
    it("is timed by the frame that triggered it, and keyed on it when there is no boundary", async () => {
        const { controller } = build();
        await controller.trigger("2026-09-22T10:00:00.000Z", "compaction");
        const node = memoryNode(controller.onSessionEnd());
        expect(node.at).toBe(Date.parse("2026-09-22T10:00:00.000Z"));
        expect(node.id).toBe("memory-reinjected-2026-09-22T10:00:00.000Z");
    });

    it("is timed by the clock when the frame has no readable time", async () => {
        const none = build();
        await none.controller.trigger(null, "compaction");
        expect(memoryNode(none.controller.onSessionEnd()).at).toBe(NOW);

        const garbled = build();
        await garbled.controller.trigger("not a time", "compaction");
        expect(memoryNode(garbled.controller.onSessionEnd()).at).toBe(NOW);
    });

    it("bands Personal memory against the pane's own context window", async () => {
        // 1200 characters is 300 tokens: 30% of a 10_000-token window's 1000-token budget.
        const { controller } = build({
            fetchEntries: async () => [{ label: "p1", source: "personal", body: "x".repeat(1200), sizeBytes: 1200 }],
        });
        await controller.trigger(null, "compaction");
        expect(memoryNode(controller.onSessionEnd()).sizeBand).toBe("low");
    });
});

describe("a trigger that finds another one ahead of it", () => {
    it("keeps the first deferred trigger whole, not the second's reason or time", async () => {
        const { controller, state, sendRpc } = build();
        state.busy = true;
        await controller.trigger("2026-09-22T10:00:00.000Z", "compaction");
        await controller.trigger("2026-09-22T11:00:00.000Z", "fresh_session");

        state.busy = false;
        controller.maybeFireDeferred();
        await vi.waitFor(() => expect(sendRpc).toHaveBeenCalledTimes(1));
        expect(sendRpc.mock.calls[0][0]).toMatch(/compacted into a summary/);
        expect(memoryNode(controller.onSessionEnd()).at).toBe(Date.parse("2026-09-22T10:00:00.000Z"));
    });

    it("fires a deferred trigger once: a second maybeFireDeferred has nothing left", async () => {
        const { controller, state, sendRpc } = build();
        state.busy = true;
        await controller.trigger(null, "compaction");
        state.busy = false;
        controller.maybeFireDeferred();
        await vi.waitFor(() => expect(sendRpc).toHaveBeenCalledTimes(1));

        controller.onSessionEnd();
        controller.maybeFireDeferred();
        await flush();
        expect(sendRpc).toHaveBeenCalledTimes(1);
    });

    it("does not stack a second hidden turn on one in flight when maybeFireDeferred is called early", async () => {
        const fetches: ReturnType<typeof gate<MemoryEntryInput[]>>[] = [];
        const state = { busy: false };
        const { controller, fetchEntries, sendRpc } = build(
            {
                fetchEntries: () => {
                    const g = gate<MemoryEntryInput[]>();
                    fetches.push(g);
                    return g.promise;
                },
                dispatchTurnStart: () => {
                    state.busy = true;
                },
            },
            state
        );
        // Two triggers both start while the pane is quiet; the first one's turn wins.
        const first = controller.trigger("2026-09-22T10:00:00.000Z", "fresh_session");
        const second = controller.trigger("2026-09-22T11:00:00.000Z", "fresh_session");
        fetches[0].resolve([ENTRY]);
        await first;
        fetches[1].resolve([ENTRY]);
        await second;
        expect(controller.isHiding()).toBe(true);
        expect(sendRpc).toHaveBeenCalledTimes(1);

        // The second waits behind the first's turn; firing before it ends starts nothing.
        controller.maybeFireDeferred();
        await flush();
        expect(fetchEntries).toHaveBeenCalledTimes(2);

        state.busy = false;
        controller.onSessionEnd();
        controller.maybeFireDeferred();
        fetches[2].resolve([ENTRY]);
        await vi.waitFor(() => expect(sendRpc).toHaveBeenCalledTimes(2));
    });

    it("is a no-op while a catch-up turn is in flight", async () => {
        const { controller, fetchEntries, dispatchTurnStart } = build({ composeCatchUp: defaultCompose });
        await controller.catchUp(null, "resumed");
        await controller.trigger(null, "compaction");

        expect(fetchEntries).not.toHaveBeenCalled();
        expect(dispatchTurnStart).toHaveBeenCalledTimes(1);
    });
});

describe("a fresh session's claim round trip", () => {
    it("defers with the claim kept when a turn starts during it, and does not claim again", async () => {
        const state = { busy: false };
        const { controller, claim, sendRpc } = build(
            {
                claimFallback: async () => {
                    state.busy = true; // the user sent something while srv was answering
                    return true;
                },
            },
            state
        );
        await controller.trigger(null, "fresh_session");
        expect(sendRpc).not.toHaveBeenCalled();
        expect(controller.isHiding()).toBe(false);

        state.busy = false;
        controller.maybeFireDeferred();
        await vi.waitFor(() => expect(sendRpc).toHaveBeenCalledTimes(1));
        expect(claim).toHaveBeenCalledTimes(1);
    });
});

describe("two compactions claimed at once", () => {
    function pendingClaims() {
        const pending: ReturnType<typeof gate<boolean>>[] = [];
        const claimFallback = () => {
            const g = gate<boolean>();
            pending.push(g);
            return g.promise;
        };
        return { pending, claimFallback };
    }

    it("keeps the first one deferred behind a turn, and drops the second", async () => {
        const { pending, claimFallback } = pendingClaims();
        const { controller, state, sendRpc } = build({ claimFallback });
        state.busy = true;
        const first = controller.trigger(null, "compaction", "u1");
        const second = controller.trigger(null, "compaction", "u2");
        pending[0].resolve(true);
        await first;
        pending[1].resolve(true);
        await second;

        state.busy = false;
        controller.maybeFireDeferred();
        await vi.waitFor(() => expect(sendRpc).toHaveBeenCalledTimes(1));
        expect(controller.onSessionEnd()?.id).toBe("memory-reinjected-u1");
    });

    it("drops the second when the first has already started its hidden turn", async () => {
        const { pending, claimFallback } = pendingClaims();
        const state = { busy: false };
        const { controller, sendRpc } = build(
            {
                claimFallback,
                dispatchTurnStart: () => {
                    state.busy = true;
                },
            },
            state
        );
        const first = controller.trigger(null, "compaction", "u1");
        const second = controller.trigger(null, "compaction", "u2");
        pending[0].resolve(true);
        await first;
        expect(controller.isHiding()).toBe(true);
        pending[1].resolve(true);
        await second;

        state.busy = false;
        controller.onSessionEnd();
        controller.maybeFireDeferred();
        await flush();
        expect(sendRpc).toHaveBeenCalledTimes(1);
    });
});

describe("catchUp: when it is allowed to start", () => {
    it("does nothing, and does not fail, when the pane has no catch-up composer", async () => {
        const { controller, sendRpc, dispatchTurnStart } = build();
        await expect(controller.catchUp(RECENT, "fresh_session")).resolves.toBeUndefined();
        expect(sendRpc).not.toHaveBeenCalled();
        expect(dispatchTurnStart).not.toHaveBeenCalled();
    });

    it("waits for a quiet pane before it claims or composes, and keeps waiting while the pane stays busy", async () => {
        const { controller, state, claim, composeCatchUp, sendRpc } = build({
            claimFallback: async () => true,
            composeCatchUp: defaultCompose,
        });
        state.busy = true;
        await controller.catchUp(RECENT, "fresh_session", { deferIfBusy: true });
        controller.maybeFireDeferred(); // still busy: it waits again
        await flush();
        expect(claim).not.toHaveBeenCalled();
        expect(composeCatchUp).not.toHaveBeenCalled();

        state.busy = false;
        controller.maybeFireDeferred();
        await vi.waitFor(() => expect(sendRpc).toHaveBeenCalledTimes(1));
        expect(claim).toHaveBeenCalledTimes(1);
    });

    it("does not even claim a compaction on a busy pane when it may not wait", async () => {
        const { controller, state, claim, composeCatchUp } = build({
            claimFallback: async () => true,
            composeCatchUp: defaultCompose,
        });
        state.busy = true;
        await controller.catchUp(RECENT, "compaction", { boundaryUuid: "u1" });
        expect(claim).not.toHaveBeenCalled();
        expect(composeCatchUp).not.toHaveBeenCalled();
    });

    it("catches up a compaction however old its frame is: only a fresh session's outcome can be stale", async () => {
        const { controller, sendRpc } = build({ composeCatchUp: defaultCompose });
        await controller.catchUp("2026-10-14T09:00:00.000Z", "compaction", { boundaryUuid: "u1" });
        expect(sendRpc).toHaveBeenCalledTimes(1);
    });

    it("counts a fresh-session outcome exactly a minute old as recent, and one a millisecond older as stale", async () => {
        const edge = build({ composeCatchUp: defaultCompose });
        await edge.controller.catchUp(new Date(NOW - 60_000).toISOString(), "fresh_session");
        expect(edge.sendRpc).toHaveBeenCalledTimes(1);

        const past = build({ composeCatchUp: defaultCompose });
        await past.controller.catchUp(new Date(NOW - 60_001).toISOString(), "fresh_session");
        expect(past.sendRpc).not.toHaveBeenCalled();
    });

    it("claims nothing and still sends for a compaction when the pane has no claim hook", async () => {
        const { controller, composeCatchUp, sendRpc } = build({ composeCatchUp: defaultCompose });
        await controller.catchUp(RECENT, "compaction", { boundaryUuid: "u1" });
        expect(composeCatchUp).toHaveBeenCalledWith("compaction", true);
        expect(sendRpc).toHaveBeenCalledTimes(1);
    });

    it("leaves the memory out of a compaction's catch-up once the hook has delivered it", async () => {
        const { controller, composeCatchUp, sendRpc } = build({
            claimFallback: async () => false,
            composeCatchUp: defaultCompose,
        });
        await controller.catchUp(RECENT, "compaction", { boundaryUuid: "u1" });
        expect(composeCatchUp).toHaveBeenCalledWith("compaction", false);
        expect(sendRpc).toHaveBeenCalledWith("catch-up only", undefined);
    });

    it("does nothing for a fresh session srv says another block's CLI wrote", async () => {
        const { controller, composeCatchUp, sendRpc } = build({
            claimFallback: async () => ({ deliver: false, foreign: true }),
            composeCatchUp: defaultCompose,
        });
        await controller.catchUp(RECENT, "fresh_session");
        expect(composeCatchUp).not.toHaveBeenCalled();
        expect(sendRpc).not.toHaveBeenCalled();
    });

    it("sees two compactions with the same frame time as two events", async () => {
        const { controller, sendRpc } = build({ composeCatchUp: defaultCompose });
        await controller.catchUp(RECENT, "compaction", { boundaryUuid: "u1" });
        controller.onSessionEnd();
        await controller.catchUp(RECENT, "compaction", { boundaryUuid: "u2" });
        expect(sendRpc).toHaveBeenCalledTimes(2);
    });

    it("does not dedupe fresh sessions whose outcome has no time", async () => {
        const { controller, sendRpc } = build({ composeCatchUp: defaultCompose });
        await controller.catchUp(null, "fresh_session");
        controller.onSessionEnd();
        await controller.catchUp(null, "fresh_session");
        expect(sendRpc).toHaveBeenCalledTimes(2);
    });

    it("does nothing while a hidden reinjection is in flight", async () => {
        const { controller, composeCatchUp } = build({ composeCatchUp: defaultCompose });
        await controller.trigger(null, "compaction");
        expect(controller.isHiding()).toBe(true);
        await controller.catchUp(null, "resumed");
        expect(composeCatchUp).not.toHaveBeenCalled();
    });

    it("does not compose a second catch-up while one is in flight", async () => {
        const { controller, composeCatchUp } = build({ composeCatchUp: defaultCompose });
        await controller.catchUp(null, "resumed");
        await controller.catchUp(null, "resumed");
        expect(composeCatchUp).toHaveBeenCalledTimes(1);
    });

    it("leaves a reinjection that is waiting for the pane to go first", async () => {
        const { controller, state, composeCatchUp, sendRpc } = build({ composeCatchUp: defaultCompose });
        state.busy = true;
        await controller.trigger(null, "compaction");
        state.busy = false;

        await controller.catchUp(null, "resumed");
        expect(composeCatchUp).not.toHaveBeenCalled();
        expect(sendRpc).not.toHaveBeenCalled();

        controller.maybeFireDeferred();
        await vi.waitFor(() => expect(sendRpc).toHaveBeenCalledTimes(1));
        expect(sendRpc.mock.calls[0][0]).toContain("global body");
    });
});

describe("catchUp: two of them racing", () => {
    it("lets the one whose compose finishes first send, and holds the other until that turn ends", async () => {
        const composes: ReturnType<typeof gate<ComposedDelivery | null>>[] = [];
        const { controller, composeCatchUp, sendRpc } = build({
            composeCatchUp: () => {
                const g = gate<ComposedDelivery | null>();
                composes.push(g);
                return g.promise;
            },
        });
        const first = controller.catchUp(null, "resumed", { deferIfBusy: true });
        const second = controller.catchUp(null, "resumed", { deferIfBusy: true });
        composes[0].resolve({ text: "one" });
        await first;
        composes[1].resolve({ text: "two" });
        await second;
        expect(sendRpc).toHaveBeenCalledTimes(1);

        // The second was held, not lost: it is composed again at the next quiet.
        controller.onSessionEnd();
        controller.maybeFireDeferred();
        composes[2].resolve({ text: "three" });
        await vi.waitFor(() => expect(sendRpc).toHaveBeenCalledTimes(2));
        expect(composeCatchUp).toHaveBeenCalledTimes(3);
    });

    it("does not send a catch-up composed while a reinjection started", async () => {
        const compose = gate<ComposedDelivery | null>();
        const { controller, sendRpc, dispatchTurnStart } = build({ composeCatchUp: () => compose.promise });
        const catchUp = controller.catchUp(null, "resumed");
        await controller.trigger(null, "compaction");
        expect(controller.isHiding()).toBe(true);

        compose.resolve({ text: "catch-up" });
        await catchUp;
        expect(sendRpc).toHaveBeenCalledTimes(1);
        expect(sendRpc.mock.calls[0][0]).toContain("global body");
        expect(dispatchTurnStart).toHaveBeenCalledTimes(1);
    });

    function pendingClaims() {
        const pending: ReturnType<typeof gate<boolean>>[] = [];
        const claimFallback = () => {
            const g = gate<boolean>();
            pending.push(g);
            return g.promise;
        };
        return { pending, claimFallback };
    }

    it("drops a compaction whose claim comes back after a catch-up turn started", async () => {
        const { pending, claimFallback } = pendingClaims();
        const { controller, composeCatchUp, sendRpc } = build({ claimFallback, composeCatchUp: defaultCompose });
        const first = controller.catchUp(RECENT, "compaction", { boundaryUuid: "u1" });
        const second = controller.catchUp(RECENT, "compaction", { boundaryUuid: "u2" });
        pending[0].resolve(true);
        await first;
        pending[1].resolve(true);
        await second;
        expect(sendRpc).toHaveBeenCalledTimes(1);
        expect(composeCatchUp).toHaveBeenCalledTimes(1);
    });

    it("drops a compaction whose claim comes back after a reinjection started", async () => {
        const gateOne = gate<boolean>();
        let calls = 0;
        const { controller, composeCatchUp } = build({
            claimFallback: () => (calls++ === 0 ? gateOne.promise : Promise.resolve(true)),
            composeCatchUp: defaultCompose,
        });
        const catchUp = controller.catchUp(RECENT, "compaction", { boundaryUuid: "u1" });
        await controller.trigger(null, "fresh_session");
        expect(controller.isHiding()).toBe(true);

        gateOne.resolve(true);
        await catchUp;
        expect(composeCatchUp).not.toHaveBeenCalled();
    });

    it("keeps the first deferred compaction's claim, not the second's", async () => {
        const { pending, claimFallback } = pendingClaims();
        const { controller, state, composeCatchUp } = build({ claimFallback, composeCatchUp: defaultCompose });
        state.busy = true;
        const first = controller.catchUp(RECENT, "compaction", { boundaryUuid: "u1", deferIfBusy: true });
        const second = controller.catchUp(RECENT, "compaction", { boundaryUuid: "u2", deferIfBusy: true });
        pending[0].resolve(true);
        await first;
        pending[1].resolve(false);
        await second;

        state.busy = false;
        controller.maybeFireDeferred();
        await vi.waitFor(() => expect(composeCatchUp).toHaveBeenCalledTimes(1));
        expect(composeCatchUp).toHaveBeenCalledWith("compaction", true);
    });
});

describe("catchUp: a turn that started while it was being set up", () => {
    it("owes nothing when the hook already delivered the memory it claimed against", async () => {
        const state = { busy: false };
        const { controller, fetchEntries, sendRpc } = build(
            {
                claimFallback: async () => false,
                composeCatchUp: async () => {
                    state.busy = true;
                    return { text: "x" };
                },
            },
            state
        );
        await controller.catchUp(RECENT, "fresh_session");
        state.busy = false;
        controller.maybeFireDeferred();
        await flush();
        expect(fetchEntries).not.toHaveBeenCalled();
        expect(sendRpc).not.toHaveBeenCalled();
    });

    it("owes nothing when it claimed nothing", async () => {
        const state = { busy: false };
        const { controller, fetchEntries, sendRpc } = build(
            {
                composeCatchUp: async () => {
                    state.busy = true;
                    return { text: "x" };
                },
            },
            state
        );
        await controller.catchUp(RECENT, "fresh_session");
        state.busy = false;
        controller.maybeFireDeferred();
        await flush();
        expect(fetchEntries).not.toHaveBeenCalled();
        expect(sendRpc).not.toHaveBeenCalled();
    });
});

describe("catchUp: a send that fails", () => {
    const failOnce = () => {
        let fail = true;
        return async () => {
            if (fail) {
                fail = false;
                throw new Error("send failed");
            }
        };
    };

    it("leaves the pane free for a reinjection straight away", async () => {
        const { controller, sendRpc } = build({ composeCatchUp: defaultCompose, sendRpc: failOnce() });
        await controller.catchUp(null, "resumed");
        await controller.trigger(null, "compaction");
        expect(sendRpc).toHaveBeenCalledTimes(2);
        expect(controller.isHiding()).toBe(true);
    });

    it("leaves the pane free for the next catch-up straight away", async () => {
        const { controller, sendRpc } = build({ composeCatchUp: defaultCompose, sendRpc: failOnce() });
        await controller.catchUp(null, "resumed");
        await controller.catchUp(null, "resumed");
        expect(sendRpc).toHaveBeenCalledTimes(2);
    });

    it("lets the same compaction boundary try again", async () => {
        const { controller, sendRpc } = build({ composeCatchUp: defaultCompose, sendRpc: failOnce() });
        await controller.catchUp(RECENT, "compaction", { boundaryUuid: "u1" });
        await controller.catchUp(RECENT, "compaction", { boundaryUuid: "u1" });
        expect(sendRpc).toHaveBeenCalledTimes(2);
    });

    it("sends no plain reinjection when the hook delivered the memory", async () => {
        const { controller, fetchEntries, sendRpc } = build({
            claimFallback: async () => false,
            composeCatchUp: defaultCompose,
            sendRpc: failOnce(),
        });
        await controller.catchUp(RECENT, "fresh_session");
        await flush();
        expect(fetchEntries).not.toHaveBeenCalled();
        expect(sendRpc).toHaveBeenCalledTimes(1);
    });

    it("sends no plain reinjection when nothing was claimed", async () => {
        const { controller, fetchEntries, sendRpc } = build({ composeCatchUp: defaultCompose, sendRpc: failOnce() });
        await controller.catchUp(RECENT, "fresh_session");
        await flush();
        expect(fetchEntries).not.toHaveBeenCalled();
        expect(sendRpc).toHaveBeenCalledTimes(1);
    });
});

describe("catchUp: when its turn ends", () => {
    it("frees the pane for a reinjection and for the next catch-up", async () => {
        const { controller, sendRpc } = build({ composeCatchUp: defaultCompose });
        await controller.catchUp(null, "resumed");
        controller.onSessionEnd();

        await controller.trigger(null, "compaction");
        expect(sendRpc).toHaveBeenCalledTimes(2);
        expect(controller.isHiding()).toBe(true);

        controller.onSessionEnd();
        await controller.catchUp(null, "resumed");
        expect(sendRpc).toHaveBeenCalledTimes(3);
    });
});
