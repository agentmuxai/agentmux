// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { createMemoryReinjectionController, HIDDEN_TURN_PLACEHOLDER_CONTENT } from "./memory-reinjection-controller";
import type { MemoryEntryInput } from "./memory-reinjection";

function globalEntry(label: string, body: string): MemoryEntryInput {
    return { label, source: "global", body, sizeBytes: body.length };
}
function personalEntry(label: string, body: string): MemoryEntryInput {
    return { label, source: "personal", body, sizeBytes: body.length };
}

function makeController(overrides: {
    contextWindow?: () => number;
    now?: () => number;
    isPaneWorking?: ReturnType<typeof vi.fn<() => boolean>>;
    dispatchTurnStart?: ReturnType<typeof vi.fn<(content: string, hidden: boolean) => void>>;
    dispatchTurnReset?: ReturnType<typeof vi.fn<() => void>>;
    sendRpc?: ReturnType<typeof vi.fn<(message: string) => Promise<void>>>;
    fetchEntries?: ReturnType<typeof vi.fn<() => Promise<MemoryEntryInput[]>>>;
} = {}) {
    // Each mock is resolved to "override if given, else default" explicitly
    // per field (rather than a generic object spread/merge) so the returned
    // value keeps its concrete Mock type instead of widening to the plain
    // function-shaped interface type — that widening was the root cause of
    // an earlier bug here: an override was correctly wired into the
    // controller, but the test then asserted against a same-named value
    // whose type no longer exposed `.mock`, so the two silently diverged.
    const contextWindow = overrides.contextWindow ?? (() => 10_000);
    const now = overrides.now ?? (() => 1_758_534_000_000);
    // Default: pane idle. Most tests care about the idle-fires-immediately
    // path; the busy-deferral describe block below overrides this explicitly.
    const isPaneWorking = overrides.isPaneWorking ?? vi.fn<() => boolean>().mockReturnValue(false);
    const dispatchTurnStart = overrides.dispatchTurnStart ?? vi.fn<(content: string, hidden: boolean) => void>();
    const dispatchTurnReset = overrides.dispatchTurnReset ?? vi.fn<() => void>();
    const sendRpc = overrides.sendRpc ?? vi.fn<(message: string) => Promise<void>>().mockResolvedValue(undefined);
    const fetchEntries =
        overrides.fetchEntries ??
        vi.fn<() => Promise<MemoryEntryInput[]>>().mockResolvedValue([globalEntry("g1", "global body")]);

    const controller = createMemoryReinjectionController({
        contextWindow,
        now,
        isPaneWorking,
        dispatchTurnStart,
        dispatchTurnReset,
        sendRpc,
        fetchEntries,
    });
    return { controller, isPaneWorking, dispatchTurnStart, dispatchTurnReset, sendRpc, fetchEntries };
}

describe("createMemoryReinjectionController — idle pane (fires immediately)", () => {
    it("starts NOT hiding", () => {
        const { controller } = makeController();
        expect(controller.isHiding()).toBe(false);
    });

    it("on trigger: fetches entries, dispatches TurnStart with a PLACEHOLDER (never the real message), sends the real message via RPC, and enters hiding", async () => {
        const { controller, dispatchTurnStart, sendRpc, fetchEntries } = makeController();

        await controller.trigger("2026-09-22T10:00:00.000Z", "compaction");

        expect(fetchEntries).toHaveBeenCalledTimes(1);
        expect(dispatchTurnStart).toHaveBeenCalledTimes(1);
        // Fix 2 (reagentx P0, PR #3502): TurnStart's content must NEVER be
        // the real composed message — any consumer watching turnPhase
        // (e.g. useAgentActivitySummary.ts) would otherwise see it.
        expect(dispatchTurnStart).toHaveBeenCalledWith(HIDDEN_TURN_PLACEHOLDER_CONTENT, true);
        const dispatchedContent = dispatchTurnStart.mock.calls[0][0] as string;
        expect(dispatchedContent).not.toContain("global body");

        // The REAL content only ever travels through sendRpc.
        expect(sendRpc).toHaveBeenCalledTimes(1);
        const sentMessage = sendRpc.mock.calls[0][0] as string;
        expect(sentMessage).toContain("<system-reminder>");
        expect(sentMessage).toContain("global body");

        expect(controller.isHiding()).toBe(true);
    });

    it("threads reason through to the composed message — fresh_session gets fresh_session wording, not compaction's", async () => {
        // §3.3 "fresh session" addendum: a persistent identity whose prior
        // session could not be resumed. trigger()'s second argument must
        // reach composeReinjectionMessage unchanged.
        const { controller, sendRpc } = makeController();

        await controller.trigger("2026-09-22T10:00:00.000Z", "fresh_session");

        expect(sendRpc).toHaveBeenCalledTimes(1);
        const sentMessage = sendRpc.mock.calls[0][0] as string;
        expect(sentMessage).toMatch(/fresh one was started/);
        expect(sentMessage).not.toMatch(/compacted into a summary/);
    });

    it("does NOT dispatch or send when there are no entries — §3.1 suppression, at the trigger layer too", async () => {
        const { controller, dispatchTurnStart, sendRpc, fetchEntries } = makeController({
            fetchEntries: vi.fn().mockResolvedValue([]),
        });

        await controller.trigger("2026-09-22T10:00:00.000Z", "compaction");

        expect(fetchEntries).toHaveBeenCalledTimes(1);
        expect(dispatchTurnStart).not.toHaveBeenCalled();
        expect(sendRpc).not.toHaveBeenCalled();
        expect(controller.isHiding()).toBe(false);
    });

    it("ignores a trigger call while already hiding — re-entrancy guard, never stacks a second hidden turn", async () => {
        const { controller, dispatchTurnStart, fetchEntries } = makeController();

        await controller.trigger("2026-09-22T10:00:00.000Z", "compaction");
        expect(controller.isHiding()).toBe(true);

        await controller.trigger("2026-09-22T10:05:00.000Z", "compaction");
        // Still only the first trigger's calls — the second was a no-op.
        expect(fetchEntries).toHaveBeenCalledTimes(1);
        expect(dispatchTurnStart).toHaveBeenCalledTimes(1);
    });

    it("on RPC send failure: clears hiding, calls dispatchTurnReset, never leaves the pane stuck", async () => {
        const { controller, dispatchTurnReset } = makeController({
            sendRpc: vi.fn().mockRejectedValue(new Error("network down")),
        });

        await controller.trigger("2026-09-22T10:00:00.000Z", "compaction");

        expect(dispatchTurnReset).toHaveBeenCalledTimes(1);
        expect(controller.isHiding()).toBe(false);
    });

    it("on fetchEntries failure: never dispatches TurnStart, never hides — nothing was sent, so nothing needs resetting", async () => {
        const { controller, dispatchTurnStart, dispatchTurnReset, sendRpc } = makeController({
            fetchEntries: vi.fn().mockRejectedValue(new Error("rpc down")),
        });

        await controller.trigger("2026-09-22T10:00:00.000Z", "compaction");

        expect(dispatchTurnStart).not.toHaveBeenCalled();
        expect(sendRpc).not.toHaveBeenCalled();
        expect(dispatchTurnReset).not.toHaveBeenCalled();
        expect(controller.isHiding()).toBe(false);
    });

    it("onSessionEnd returns null and stays not-hiding when no hidden turn is in flight — a normal turn's session_end must be a no-op here", () => {
        const { controller } = makeController();
        expect(controller.onSessionEnd()).toBeNull();
        expect(controller.isHiding()).toBe(false);
    });

    it("onSessionEnd returns the built node and clears hiding, once, after a successful hidden turn", async () => {
        const { controller } = makeController();
        await controller.trigger("2026-09-22T10:00:00.000Z", "compaction");
        expect(controller.isHiding()).toBe(true);

        const node = controller.onSessionEnd();
        expect(node).not.toBeNull();
        expect(node?.type).toBe("memory_reinjection");
        expect(node?.type === "memory_reinjection" && node.globalMemoryCount).toBe(1);
        expect(controller.isHiding()).toBe(false);

        // A second call (e.g. a stray extra session_end) must not resurrect
        // a stale node or leave hiding toggled back on.
        expect(controller.onSessionEnd()).toBeNull();
    });

    it("builds the node's sizeBand from Personal entries only, threading contextWindow through from the controller's own opts", async () => {
        // "x".repeat(3800) -> estimateTokenCount = 950 -> critical against a
        // 10_000-token window at the default 0.10 fraction (threshold 1000).
        const { controller } = makeController({
            fetchEntries: vi.fn().mockResolvedValue([personalEntry("p1", "x".repeat(3800))]),
        });
        await controller.trigger("2026-09-22T10:00:00.000Z", "compaction");
        const node = controller.onSessionEnd();
        expect(node?.sizeBand).toBe("critical");
    });
});

/**
 * Fix 1, reagentx P0 on PR #3502: dispatching TurnStart while a real turn is
 * already in flight (the common auto-compaction case — compact_boundary
 * lands mid an ongoing, still-streaming turn) regresses turnPhase from
 * Streaming back to Submitting. The controller must defer instead of firing
 * immediately, and only actually fire once the busy turn's own session_end
 * has fully resolved.
 */
describe("createMemoryReinjectionController — busy pane (defers)", () => {
    it("does NOT fetch, dispatch, or send when the pane is busy — defers instead", async () => {
        const { controller, fetchEntries, dispatchTurnStart, sendRpc, isPaneWorking } = makeController({
            isPaneWorking: vi.fn().mockReturnValue(true),
        });

        await controller.trigger("2026-09-22T10:00:00.000Z", "compaction");

        expect(isPaneWorking).toHaveBeenCalled();
        expect(fetchEntries).not.toHaveBeenCalled();
        expect(dispatchTurnStart).not.toHaveBeenCalled();
        expect(sendRpc).not.toHaveBeenCalled();
        expect(controller.isHiding()).toBe(false);
    });

    it("maybeFireDeferred is a no-op when nothing was deferred", () => {
        const { controller, fetchEntries } = makeController();
        controller.maybeFireDeferred();
        expect(fetchEntries).not.toHaveBeenCalled();
    });

    it("preserves the reason across a defer — a fresh_session trigger deferred while busy still sends fresh_session wording once fired", async () => {
        const { controller, sendRpc, isPaneWorking } = makeController({
            isPaneWorking: vi.fn().mockReturnValue(true),
        });

        await controller.trigger("2026-09-22T10:00:00.000Z", "fresh_session");
        expect(sendRpc).not.toHaveBeenCalled();

        isPaneWorking.mockReturnValue(false);
        controller.maybeFireDeferred();
        await Promise.resolve();
        await Promise.resolve();

        expect(sendRpc).toHaveBeenCalledTimes(1);
        const sentMessage = sendRpc.mock.calls[0][0] as string;
        expect(sentMessage).toMatch(/fresh one was started/);
    });

    it("maybeFireDeferred fires the deferred trigger once called AND genuinely idle — fetch/dispatch/send now happen", async () => {
        const { controller, fetchEntries, dispatchTurnStart, sendRpc, isPaneWorking } = makeController({
            isPaneWorking: vi.fn().mockReturnValue(true),
        });

        await controller.trigger("2026-09-22T10:00:00.000Z", "compaction");
        expect(fetchEntries).not.toHaveBeenCalled();

        // doTrigger() re-checks isPaneWorking() itself right before
        // dispatching (P1 fix, second review round) — must actually be
        // idle by the time maybeFireDeferred() is called, not just called.
        isPaneWorking.mockReturnValue(false);
        controller.maybeFireDeferred();
        // doTrigger is async internally; flush microtasks.
        await Promise.resolve();
        await Promise.resolve();

        expect(fetchEntries).toHaveBeenCalledTimes(1);
        expect(dispatchTurnStart).toHaveBeenCalledWith(HIDDEN_TURN_PLACEHOLDER_CONTENT, true);
        expect(sendRpc).toHaveBeenCalledTimes(1);
        expect(controller.isHiding()).toBe(true);
    });

    it("a second trigger() call while one is already deferred does not stack a second deferred entry", async () => {
        const { controller, fetchEntries, isPaneWorking } = makeController({
            isPaneWorking: vi.fn().mockReturnValue(true),
        });

        await controller.trigger("2026-09-22T10:00:00.000Z", "compaction");
        await controller.trigger("2026-09-22T10:05:00.000Z", "compaction");

        controller.maybeFireDeferred();
        await Promise.resolve();
        await Promise.resolve();

        // Only ONE fetch — the second trigger() call while busy was a no-op,
        // not a second queued deferral.
        expect(fetchEntries).toHaveBeenCalledTimes(1);
        void isPaneWorking;
    });

    it("full lifecycle: deferred while busy, fires once idle, completes via onSessionEnd like any other hidden turn", async () => {
        const { controller, isPaneWorking } = makeController({
            isPaneWorking: vi.fn().mockReturnValue(true),
        });

        await controller.trigger("2026-09-22T10:00:00.000Z", "compaction");
        expect(controller.isHiding()).toBe(false); // still deferred, not yet hiding

        isPaneWorking.mockReturnValue(false); // now genuinely idle
        controller.maybeFireDeferred();
        await Promise.resolve();
        await Promise.resolve();
        expect(controller.isHiding()).toBe(true);

        const node = controller.onSessionEnd();
        expect(node).not.toBeNull();
        expect(controller.isHiding()).toBe(false);
        void isPaneWorking;
    });
});

/**
 * reagentx P1, second review round on PR #3502: checking isPaneWorking()
 * only once — before the async fetchEntries() call — left a real race. A
 * real turn can start DURING that await; the original fix still dispatched
 * TurnStart unconditionally once the fetch resolved. These tests simulate
 * exactly that: isPaneWorking() flips to busy WHILE fetchEntries() is
 * in-flight, not before trigger() is even called.
 */
describe("createMemoryReinjectionController — pane becomes busy DURING the fetch (race)", () => {
    function makeRacingController() {
        // isPaneWorking reads false on its FIRST call (trigger()'s own
        // pre-fetch check — still idle at that instant) and true on every
        // call after — simulating a real turn starting while
        // fetchEntries() is in flight.
        let calls = 0;
        const isPaneWorking = vi.fn<() => boolean>(() => {
            calls++;
            return calls > 1;
        });
        return makeController({ isPaneWorking });
    }

    it("does NOT dispatch TurnStart when the pane became busy during fetchEntries()", async () => {
        const { controller, dispatchTurnStart, sendRpc } = makeRacingController();

        await controller.trigger("2026-09-22T10:00:00.000Z", "compaction");

        expect(dispatchTurnStart).not.toHaveBeenCalled();
        expect(sendRpc).not.toHaveBeenCalled();
        expect(controller.isHiding()).toBe(false);
    });

    it("defers instead — a subsequent maybeFireDeferred() (once genuinely idle) fires it", async () => {
        const { controller, fetchEntries, dispatchTurnStart, isPaneWorking } = makeRacingController();

        await controller.trigger("2026-09-22T10:00:00.000Z", "compaction");
        expect(dispatchTurnStart).not.toHaveBeenCalled();
        expect(fetchEntries).toHaveBeenCalledTimes(1); // the fetch itself still happened — the race is caught AFTER it, not before

        // Now genuinely idle for every subsequent isPaneWorking() call.
        isPaneWorking.mockReturnValue(false);
        controller.maybeFireDeferred();
        await Promise.resolve();
        await Promise.resolve();

        expect(dispatchTurnStart).toHaveBeenCalledTimes(1);
        expect(controller.isHiding()).toBe(true);
    });

    it("maybeFireDeferred() itself re-checks busy-ness — does not fire blindly just because it was called", async () => {
        // Deferred once (busy). maybeFireDeferred() is called, but the pane
        // is STILL busy at that exact moment too (e.g. the same still-
        // running turn, or a new one) — must re-defer, not fire.
        const { controller, dispatchTurnStart, isPaneWorking } = makeController({
            isPaneWorking: vi.fn().mockReturnValue(true),
        });

        await controller.trigger("2026-09-22T10:00:00.000Z", "compaction");
        controller.maybeFireDeferred();
        await Promise.resolve();
        await Promise.resolve();

        expect(dispatchTurnStart).not.toHaveBeenCalled();
        expect(controller.isHiding()).toBe(false);

        // Now genuinely idle — a LATER maybeFireDeferred() (e.g. from the
        // NEXT session_end) finally fires it.
        isPaneWorking.mockReturnValue(false);
        controller.maybeFireDeferred();
        await Promise.resolve();
        await Promise.resolve();
        expect(dispatchTurnStart).toHaveBeenCalledTimes(1);
    });
});

// SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md §7 P2: once Claude Code's
// SessionStart hook delivers memory, this fallback asks before it fires.
describe("createMemoryReinjectionController — claimFallback (the SessionStart hook may have delivered)", () => {
    function withClaim(
        claimFallback: (reason: "compaction" | "fresh_session", boundaryUuid?: string) => Promise<boolean>,
        busy: () => boolean = () => false,
    ) {
        const dispatchTurnStart = vi.fn<(content: string, hidden: boolean) => void>();
        const sendRpc = vi.fn<(message: string) => Promise<void>>().mockResolvedValue(undefined);
        const isPaneWorking = vi.fn<() => boolean>().mockImplementation(busy);
        const claim = vi.fn(claimFallback);
        const controller = createMemoryReinjectionController({
            contextWindow: () => 10_000,
            now: () => 0,
            isPaneWorking,
            dispatchTurnStart,
            dispatchTurnReset: vi.fn(),
            sendRpc,
            fetchEntries: vi.fn<() => Promise<MemoryEntryInput[]>>().mockResolvedValue([globalEntry("g1", "body")]),
            claimFallback: claim,
        });
        return { controller, dispatchTurnStart, sendRpc, claim, isPaneWorking };
    }

    it("stands down when the hook already delivered: nothing hidden, dispatched or sent", async () => {
        const { controller, dispatchTurnStart, sendRpc, claim } = withClaim(async () => false);
        await controller.trigger("2026-09-27T10:00:00.000Z", "compaction");
        expect(claim.mock.calls[0][0]).toBe("compaction");
        expect(dispatchTurnStart).not.toHaveBeenCalled();
        expect(sendRpc).not.toHaveBeenCalled();
        expect(controller.isHiding()).toBe(false);
    });

    it("delivers when the claim says so", async () => {
        const { controller, dispatchTurnStart, sendRpc } = withClaim(async () => true);
        await controller.trigger(null, "fresh_session");
        expect(dispatchTurnStart).toHaveBeenCalledTimes(1);
        expect(sendRpc).toHaveBeenCalledTimes(1);
        expect(controller.isHiding()).toBe(true);
    });

    it("delivers when the claim fails: a duplicate beats no memory", async () => {
        const { controller, sendRpc } = withClaim(async () => {
            throw new Error("rpc down");
        });
        await controller.trigger(null, "compaction");
        expect(sendRpc).toHaveBeenCalledTimes(1);
    });

    it("a fresh session never claims while the pane is busy — it defers, and claims only when it really fires", async () => {
        let busy = true;
        const { controller, claim, sendRpc } = withClaim(async () => true, () => busy);
        await controller.trigger(null, "fresh_session");
        expect(claim).not.toHaveBeenCalled();
        busy = false;
        controller.maybeFireDeferred();
        await vi.waitFor(() => expect(sendRpc).toHaveBeenCalledTimes(1));
        expect(claim).toHaveBeenCalledTimes(1);
    });

    it("defers instead of firing if a real turn started during the claim round trip", async () => {
        let busy = false;
        const { controller, sendRpc } = withClaim(async () => {
            busy = true;
            return true;
        }, () => busy);
        await controller.trigger(null, "compaction");
        expect(sendRpc).not.toHaveBeenCalled();
        expect(controller.isHiding()).toBe(false);
    });
});

describe("createMemoryReinjectionController — srv composes (CD2b)", () => {
    const card = {
        type: "context_delivery" as const,
        id: "memory-injected-fb-1",
        reason: "compaction" as const,
        items: [{ kind: "global_memory" as const, name: "App API", tier: "system" as const, sizeBytes: 10, tokens: 3 }],
        timestamp: 1,
    };
    const make = (compose: () => Promise<unknown>) => {
        const sendRpc = vi.fn().mockResolvedValue(undefined);
        const fetchEntries = vi.fn().mockResolvedValue([
            { label: "notes.md", source: "personal", body: "local", sizeBytes: 5 },
        ] satisfies MemoryEntryInput[]);
        const controller = createMemoryReinjectionController({
            contextWindow: () => 200_000,
            now: () => 0,
            isPaneWorking: () => false,
            fetchEntries,
            sendRpc,
            compose: compose as never,
            dispatchTurnStart: vi.fn(),
            dispatchTurnReset: vi.fn(),
        });
        return { controller, sendRpc, fetchEntries };
    };

    it("sends srv's message with its delivery id, and shows srv's card", async () => {
        const { controller, sendRpc, fetchEntries } = make(() =>
            Promise.resolve({ deliveryId: "fb-1", text: "<system-reminder>srv</system-reminder>\n", node: card }),
        );
        await controller.trigger(null, "compaction");
        expect(sendRpc).toHaveBeenCalledWith("<system-reminder>srv</system-reminder>\n", "fb-1");
        expect(fetchEntries).not.toHaveBeenCalled();
        expect(controller.onSessionEnd()).toBe(card);
    });

    it("does nothing when srv has no memory to deliver", async () => {
        const { controller, sendRpc, fetchEntries } = make(() => Promise.resolve(null));
        await controller.trigger(null, "compaction");
        expect(sendRpc).not.toHaveBeenCalled();
        expect(fetchEntries).not.toHaveBeenCalled();
        expect(controller.isHiding()).toBe(false);
    });

    it("composes here, as before, when srv can't (an older srv)", async () => {
        const { controller, sendRpc, fetchEntries } = make(() => Promise.reject(new Error("unknown command")));
        await controller.trigger(null, "compaction");
        expect(fetchEntries).toHaveBeenCalledTimes(1);
        const [message, deliveryId] = sendRpc.mock.calls[0];
        expect(message).toContain("Your memory was reinjected because your working context was just reset.");
        expect(deliveryId).toBeUndefined();
        expect(controller.onSessionEnd()?.type).toBe("memory_reinjection");
    });
});

// SPEC_COMPACTION_ESTIMATED_PROGRESS_AND_STREAM_FRAMES_2026_10_01.md §8 D1:
// a compaction is claimed when its boundary arrives, once per boundary uuid.
describe("createMemoryReinjectionController — compaction claimed at the boundary", () => {
    const UUID = "8c1f4e2a-2b7d-4a51-9a0e-6f3c2d1b0a99";
    function make(answer: boolean, initiallyBusy: boolean) {
        let busy = initiallyBusy;
        const claim = vi.fn(async (_reason: "compaction" | "fresh_session", _uuid?: string) => answer);
        const sendRpc = vi.fn<(message: string) => Promise<void>>().mockResolvedValue(undefined);
        const fetchEntries = vi.fn<() => Promise<MemoryEntryInput[]>>().mockResolvedValue([globalEntry("g1", "body")]);
        const controller = createMemoryReinjectionController({
            contextWindow: () => 10_000,
            now: () => 0,
            isPaneWorking: () => busy,
            dispatchTurnStart: vi.fn(),
            dispatchTurnReset: vi.fn(),
            sendRpc,
            fetchEntries,
            claimFallback: claim,
        });
        const endTurn = () => {
            busy = false;
            controller.onSessionEnd();
            controller.maybeFireDeferred();
        };
        return { controller, claim, sendRpc, fetchEntries, endTurn };
    }

    it("claims when the boundary arrives mid-turn, with its uuid, not at turn end", async () => {
        const { controller, claim, sendRpc, endTurn } = make(true, true);
        await controller.trigger(null, "compaction", UUID);
        expect(claim).toHaveBeenCalledTimes(1);
        expect(claim).toHaveBeenCalledWith("compaction", UUID);
        expect(sendRpc).not.toHaveBeenCalled();
        endTurn();
        await vi.waitFor(() => expect(sendRpc).toHaveBeenCalledTimes(1));
        expect(claim).toHaveBeenCalledTimes(1);
    });

    it("a skip decision sends nothing at turn end", async () => {
        const { controller, claim, sendRpc, fetchEntries, endTurn } = make(false, true);
        await controller.trigger(null, "compaction", UUID);
        endTurn();
        await Promise.resolve();
        await Promise.resolve();
        expect(claim).toHaveBeenCalledTimes(1);
        expect(fetchEntries).not.toHaveBeenCalled();
        expect(sendRpc).not.toHaveBeenCalled();
    });

    it("a send decision sends exactly once", async () => {
        const { controller, sendRpc, endTurn } = make(true, true);
        await controller.trigger(null, "compaction", UUID);
        endTurn();
        await vi.waitFor(() => expect(sendRpc).toHaveBeenCalledTimes(1));
        controller.onSessionEnd(); // the hidden turn ends
        controller.maybeFireDeferred();
        await Promise.resolve();
        expect(sendRpc).toHaveBeenCalledTimes(1);
    });

    it("the same boundary seen twice claims once, even after its reinjection finished", async () => {
        const { controller, claim, sendRpc } = make(true, false);
        await controller.trigger(null, "compaction", UUID);
        expect(sendRpc).toHaveBeenCalledTimes(1);
        controller.onSessionEnd();
        await controller.trigger(null, "compaction", UUID);
        expect(claim).toHaveBeenCalledTimes(1);
        expect(sendRpc).toHaveBeenCalledTimes(1);
    });

    it("a second compaction (new uuid) is claimed and sent again", async () => {
        const { controller, claim, sendRpc } = make(true, false);
        await controller.trigger(null, "compaction", UUID);
        controller.onSessionEnd();
        await controller.trigger(null, "compaction", "0f2e4d6c-8b0a-4c1e-9f3d-5a7b9c1d3e5f");
        expect(claim).toHaveBeenCalledTimes(2);
        expect(sendRpc).toHaveBeenCalledTimes(2);
    });

    it("keys the reinjection node on the boundary uuid", async () => {
        const { controller } = make(true, false);
        await controller.trigger(null, "compaction", UUID);
        expect(controller.onSessionEnd()?.id).toBe(`memory-reinjected-${UUID}`);
    });
});
