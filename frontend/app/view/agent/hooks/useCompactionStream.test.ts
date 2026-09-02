// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CompactionState } from "@/app/store/agent-pane-state/types";

// Codex P1 on PR #2378: `compaction_started` is a persisted WPS event
// (persist: 1) with no completion tombstone — a late/reconnecting
// subscriber replays it verbatim even long after the matching
// compaction finished. These tests cover the staleness guard added to
// close that gap.

const hub = vi.hoisted(() => ({
    handlers: new Map<string, (e: unknown) => void>(),
}));

vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn((sub: { eventType: string; handler: (e: unknown) => void }) => {
        hub.handlers.set(sub.eventType, sub.handler);
        return () => hub.handlers.delete(sub.eventType);
    }),
}));

import {
    compactionStartedNodeId,
    resolveCompactionStart,
    useCompactionStream,
} from "./useCompactionStream";

describe("resolveCompactionStart", () => {
    const NOW = 1_800_000_000_000; // arbitrary fixed epoch ms

    function payload(overrides: Record<string, unknown> = {}) {
        return {
            trigger: "manual",
            sessionId: "sess-1",
            startedAt: new Date(NOW).toISOString(),
            ...overrides,
        };
    }

    it("accepts a fresh manual start", () => {
        const resolved = resolveCompactionStart(payload({ trigger: "manual" }), NOW);
        expect(resolved).toEqual({ trigger: "manual", startedAt: NOW });
    });

    it("accepts a fresh auto start", () => {
        const resolved = resolveCompactionStart(payload({ trigger: "auto" }), NOW);
        expect(resolved?.trigger).toBe("auto");
    });

    it("accepts a start still well within the plausible-duration window", () => {
        const fiveMinutesAgo = new Date(NOW - 5 * 60 * 1000).toISOString();
        const resolved = resolveCompactionStart(payload({ startedAt: fiveMinutesAgo }), NOW);
        expect(resolved).not.toBeNull();
    });

    it("rejects a stale replay older than the plausible-duration window", () => {
        // The exact bug this guard exists for: a compaction that
        // finished 20 minutes ago replays on reconnect and must not
        // resurrect a "Compacting…" state.
        const twentyMinutesAgo = new Date(NOW - 20 * 60 * 1000).toISOString();
        expect(resolveCompactionStart(payload({ startedAt: twentyMinutesAgo }), NOW)).toBeNull();
    });

    it("rejects right at the boundary consistently (just over the max is stale)", () => {
        const justOver = new Date(NOW - (10 * 60 * 1000 + 1)).toISOString();
        expect(resolveCompactionStart(payload({ startedAt: justOver }), NOW)).toBeNull();
    });

    it("accepts a startedAt slightly in the future within clock-skew tolerance, clamped to now", () => {
        const thirtySecondsFuture = new Date(NOW + 30 * 1000).toISOString();
        const resolved = resolveCompactionStart(payload({ startedAt: thirtySecondsFuture }), NOW);
        expect(resolved?.startedAt).toBe(NOW);
    });

    it("rejects a startedAt far in the future beyond clock-skew tolerance", () => {
        const fiveMinutesFuture = new Date(NOW + 5 * 60 * 1000).toISOString();
        expect(resolveCompactionStart(payload({ startedAt: fiveMinutesFuture }), NOW)).toBeNull();
    });

    it("rejects a missing startedAt (fail closed, not treated as fresh)", () => {
        const { startedAt, ...rest } = payload();
        expect(resolveCompactionStart(rest, NOW)).toBeNull();
    });

    it("rejects an unparseable startedAt", () => {
        expect(resolveCompactionStart(payload({ startedAt: "not-a-date" }), NOW)).toBeNull();
    });

    it("rejects an unrecognized trigger value", () => {
        expect(resolveCompactionStart(payload({ trigger: "sometimes" }), NOW)).toBeNull();
    });

    it("rejects a missing trigger", () => {
        const { trigger, ...rest } = payload();
        expect(resolveCompactionStart(rest, NOW)).toBeNull();
    });

    it("rejects a non-object payload", () => {
        expect(resolveCompactionStart(null, NOW)).toBeNull();
        expect(resolveCompactionStart(undefined, NOW)).toBeNull();
        expect(resolveCompactionStart("not-an-object", NOW)).toBeNull();
    });
});

describe("useCompactionStream — transcript node driven by the `compacting` signal (reagent P1 on PR #2928)", () => {
    // The reducer can set `state.compacting` from THREE different dispatch
    // sites: this hook's own live CompactionStarted (accepted immediately),
    // or a buffered `pendingCompactionPing` promoted later by
    // ReconcileTurnActive (agent-view.tsx) or StreamFlushObserved
    // (stream-flush-queue.ts) — see
    // SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02.md §4. Neither
    // of the latter two call sites has access to this hook's document queue,
    // so the node-push logic reacts to the `compacting` SIGNAL rather than
    // any one dispatch's return value — these tests simulate all three by
    // just setting the fake `compacting` accessor directly, exactly as
    // `registerAgentPane`'s generic projection would after any of them.

    function makeFakeQueue() {
        return { pushNewNode: vi.fn(), scheduleFlush: vi.fn() } as any;
    }

    function fireCompactionStarted(startedAtIso: string, trigger: "manual" | "auto" = "auto") {
        const handler = [...hub.handlers.values()][0];
        if (!handler) throw new Error("no compaction_started handler registered — useCompactionStream did not mount");
        handler({ data: { trigger, startedAt: startedAtIso } });
    }

    beforeEach(() => {
        hub.handlers.clear();
        vi.useFakeTimers();
        vi.setSystemTime(1_800_000_000_000);
    });
    afterEach(() => vi.useRealTimers());

    it("pushes the transcript node when `compacting` transitions from null to set via THIS hook's own live dispatch", async () => {
        await createRoot(async (dispose) => {
            const [compacting, setCompacting] = createSignal<CompactionState | null>(null);
            const dispatchPane = vi.fn((command: { type: string; trigger: "manual" | "auto"; at: number }) => {
                if (command.type === "CompactionStarted") {
                    setCompacting({ trigger: command.trigger, startedAt: command.at });
                }
                return [];
            });
            const model = { blockId: "b", disposed: false, dispatchPane, dispatchDoc: vi.fn(() => []) } as any;
            const queue = makeFakeQueue();
            useCompactionStream({ blockId: "b", model, queue, hasNodeId: () => false, addNodeId: () => {}, compacting });
            await Promise.resolve();

            fireCompactionStarted(new Date().toISOString(), "manual");
            await Promise.resolve();

            expect(dispatchPane).toHaveBeenCalledWith(
                expect.objectContaining({ type: "CompactionStarted", trigger: "manual" }),
            );
            expect(queue.pushNewNode).toHaveBeenCalledTimes(1);
            expect(queue.pushNewNode).toHaveBeenCalledWith(
                expect.objectContaining({ type: "compaction_started", trigger: "manual" }),
            );

            dispose();
        });
    });

    it("pushes the transcript node when `compacting` is set by a DIFFERENT dispatch entirely — the promoted-ping path", async () => {
        // Exactly reagent's failure scenario: a pendingCompactionPing gets
        // promoted by ReconcileTurnActive or StreamFlushObserved, neither of
        // which is this hook's own WPS-triggered dispatch. Simulated here by
        // setting `compacting` directly, with no `fireCompactionStarted` call
        // at all — proving the push doesn't depend on this hook having
        // dispatched anything itself.
        await createRoot(async (dispose) => {
            const [compacting, setCompacting] = createSignal<CompactionState | null>(null);
            const model = { blockId: "b", disposed: false, dispatchPane: vi.fn(() => []), dispatchDoc: vi.fn(() => []) } as any;
            const queue = makeFakeQueue();
            useCompactionStream({ blockId: "b", model, queue, hasNodeId: () => false, addNodeId: () => {}, compacting });
            await Promise.resolve();

            setCompacting({ trigger: "manual", startedAt: Date.now() });
            await Promise.resolve();

            expect(queue.pushNewNode).toHaveBeenCalledTimes(1);
            expect(queue.pushNewNode).toHaveBeenCalledWith(
                expect.objectContaining({ type: "compaction_started", trigger: "manual" }),
            );

            dispose();
        });
    });

    it("does not push a node while `compacting` stays null (a buffered-but-not-yet-promoted ping)", async () => {
        await createRoot(async (dispose) => {
            const [compacting] = createSignal<CompactionState | null>(null);
            const model = { blockId: "b", disposed: false, dispatchPane: vi.fn(() => []), dispatchDoc: vi.fn(() => []) } as any;
            const queue = makeFakeQueue();
            useCompactionStream({ blockId: "b", model, queue, hasNodeId: () => false, addNodeId: () => {}, compacting });
            await Promise.resolve();

            fireCompactionStarted(new Date().toISOString());
            await Promise.resolve();

            expect(queue.pushNewNode).not.toHaveBeenCalled();
            dispose();
        });
    });

    it("dedups via hasNodeId — does not push twice for the same startedAt", async () => {
        await createRoot(async (dispose) => {
            const [compacting, setCompacting] = createSignal<CompactionState | null>(null);
            const seen = new Set<string>();
            const model = { blockId: "b", disposed: false, dispatchPane: vi.fn(() => []), dispatchDoc: vi.fn(() => []) } as any;
            const queue = makeFakeQueue();
            useCompactionStream({
                blockId: "b",
                model,
                queue,
                hasNodeId: (id) => seen.has(id),
                addNodeId: (id) => { seen.add(id); },
                compacting,
            });
            await Promise.resolve();

            const startedAt = Date.now();
            setCompacting({ trigger: "auto", startedAt });
            await Promise.resolve();
            // A no-op re-set to the identical value (e.g. an unrelated
            // reactive re-run) must not push a second node.
            setCompacting({ trigger: "auto", startedAt });
            await Promise.resolve();

            expect(queue.pushNewNode).toHaveBeenCalledTimes(1);
            dispose();
        });
    });
});

describe("compactionStartedNodeId", () => {
    it("is keyed by startedAt", () => {
        expect(compactionStartedNodeId(1234)).toBe("compaction-started-1234");
    });
});
