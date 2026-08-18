// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentActivitySummary — pins the session-goal-title trigger behavior
 * introduced by docs/specs/SPEC_AMBIENT_PANE_TITLE_OVERALL_GOAL_TRACKING_2026_08_17.md:
 * fires on entering `Submitting` (not turn completion), sends the literal
 * `pendingContent` as `user_message`, and only writes the result if it's
 * still the most recent request when it resolves.
 */

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TurnPhase } from "@/app/store/agent-pane-state/types";

const hub = vi.hoisted(() => ({
    activitySummary: vi.fn(),
    updateMeta: vi.fn(),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentActivitySummaryCommand: (...args: unknown[]) => hub.activitySummary(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/wos", () => ({ makeORef: (type: string, id: string) => `${type}:${id}` }));
vi.mock("@/app/store/services", () => ({
    ObjectService: { UpdateObjectMeta: (...args: unknown[]) => hub.updateMeta(...args) },
}));
vi.mock("@/app/store/token-usage", () => ({ recordTurn: vi.fn() }));

import { useAgentActivitySummary } from "./useAgentActivitySummary";

const BLOCK_ID = "b";

beforeEach(() => {
    hub.activitySummary.mockReset();
    hub.updateMeta.mockReset().mockResolvedValue(undefined);
});
afterEach(() => {
    vi.clearAllMocks();
});

function setup(initialPhase: TurnPhase = { kind: "Idle" }) {
    let setPhase!: (p: TurnPhase) => void;
    let dispose: () => void = () => {};
    createRoot((d) => {
        dispose = d;
        const [phase, setP] = createSignal<TurnPhase>(initialPhase);
        setPhase = setP;
        useAgentActivitySummary({
            blockId: BLOCK_ID,
            turnPhase: phase,
            getRootWidth: () => 400,
        });
    });
    return { setPhase, dispose };
}

describe("useAgentActivitySummary — trigger", () => {
    it("fires on entering Submitting, sending pendingContent as user_message", async () => {
        hub.activitySummary.mockResolvedValue({ summary: "new title", tokens: null });
        const { setPhase, dispose } = setup();

        setPhase({ kind: "Submitting", submittedAt: 1, pendingContent: "invert user input styling" });
        await Promise.resolve();
        await Promise.resolve();

        expect(hub.activitySummary).toHaveBeenCalledTimes(1);
        const [, payload] = hub.activitySummary.mock.calls[0];
        expect(payload.block_id).toBe(BLOCK_ID);
        expect(payload.user_message).toBe("invert user input styling");
        expect(hub.updateMeta).toHaveBeenCalledWith("block:b", { "term:ambient_summary": "new title" });
        dispose();
    });

    it("does not fire at mount even if the initial phase is already Submitting (defer: true)", async () => {
        const { dispose } = setup({ kind: "Submitting", submittedAt: 1, pendingContent: "already in flight" });
        await Promise.resolve();
        await Promise.resolve();

        expect(hub.activitySummary).not.toHaveBeenCalled();
        dispose();
    });

    it.each(["Idle", "Streaming", "Interrupting", "Done", "Disconnected"] as const)(
        "does not fire when the phase transitions to %s",
        async (kind) => {
            const { setPhase, dispose } = setup();
            const phase =
                kind === "Streaming"
                    ? ({ kind, bufferSize: 0, toolsActive: 0, lastEventMs: 0 } as TurnPhase)
                    : kind === "Interrupting"
                      ? ({ kind, reason: "user", sigintSentAt: 0 } as TurnPhase)
                      : kind === "Done"
                        ? ({ kind, outcome: "completed", finishedAt: 0 } as TurnPhase)
                        : kind === "Disconnected"
                          ? ({ kind, lastKind: "Streaming", lastConnectedAt: 0, reason: "stream-unsubscribed" } as TurnPhase)
                          : ({ kind } as TurnPhase);
            setPhase(phase);
            await Promise.resolve();

            expect(hub.activitySummary).not.toHaveBeenCalled();
            dispose();
        },
    );

    it("discards a stale result superseded by a newer Submitting transition before it resolves", async () => {
        let resolveFirst!: (v: unknown) => void;
        hub.activitySummary
            .mockImplementationOnce(() => new Promise((res) => { resolveFirst = res; }))
            .mockResolvedValueOnce({ summary: "second title", tokens: null });
        const { setPhase, dispose } = setup();

        setPhase({ kind: "Submitting", submittedAt: 1, pendingContent: "first ask" });
        await Promise.resolve();
        setPhase({ kind: "Streaming", bufferSize: 0, toolsActive: 0, lastEventMs: 0 });
        setPhase({ kind: "Submitting", submittedAt: 2, pendingContent: "second ask" });
        await Promise.resolve();
        await Promise.resolve();

        // Second call already resolved and wrote its result.
        expect(hub.updateMeta).toHaveBeenCalledWith("block:b", { "term:ambient_summary": "second title" });
        hub.updateMeta.mockClear();

        // First (stale) call resolves late — must NOT overwrite the newer title.
        resolveFirst({ summary: "stale first title", tokens: null });
        await Promise.resolve();
        await Promise.resolve();

        expect(hub.updateMeta).not.toHaveBeenCalled();
        dispose();
    });
});
