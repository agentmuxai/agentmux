// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * reagentx P0 on PR #2781 (round 4): a persisted-session agent block whose
 * mount-time history read comes back EMPTY (the backend's backfill scan
 * hasn't started broadcasting yet — not "nothing will ever happen") used
 * to resolve `settled=true` immediately. The real "started" then arrived
 * moments later, flipping `settled` back to `false` — which, wired into
 * `block.tsx`'s `<Show when={ready()}>`, unmounts/re-registers/rescans the
 * pane, repeating indefinitely on every reopen of any agent with a
 * persisted session. These tests drive that exact race directly against
 * the hook's state machine (not just the pure `resolveBackfillStatus`
 * resolver), mirroring `useControllerStatusEvents.test.ts`'s
 * `createRoot` + mocked-`waveEventSubscribe` harness.
 */

import { createRoot, createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    handler: null as ((e: unknown) => void) | null,
}));

vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn((sub: { eventType: string; handler: (e: unknown) => void }) => {
        hub.handler = sub.handler;
        return () => {
            hub.handler = null;
        };
    }),
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

const rpcHub = vi.hoisted(() => ({
    resolve: null as ((v: unknown) => void) | null,
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        EventReadHistoryCommand: vi.fn(
            () =>
                new Promise((resolve) => {
                    rpcHub.resolve = resolve;
                })
        ),
    },
}));

import { resolveBackfillStatus, useSubagentBackfillGate } from "./useSubagentBackfillGate";

afterEach(() => {
    hub.handler = null;
    rpcHub.resolve = null;
    vi.useRealTimers();
});

function mount(viewType: string | undefined, hasPersistedSession: boolean) {
    let dispose = () => {};
    let settled: () => boolean = () => false;
    createRoot((d) => {
        dispose = d;
        settled = useSubagentBackfillGate("block-1", () => viewType, () => hasPersistedSession);
    });
    return { settled: () => settled(), dispose };
}

describe("resolveBackfillStatus", () => {
    it("resolves a started status", () => {
        expect(resolveBackfillStatus({ status: "started" })).toBe("started");
    });

    it("resolves a done status", () => {
        expect(resolveBackfillStatus({ status: "done" })).toBe("done");
    });

    it("rejects an unrecognized status", () => {
        expect(resolveBackfillStatus({ status: "pending" })).toBeNull();
    });

    it("rejects a missing status", () => {
        expect(resolveBackfillStatus({})).toBeNull();
    });

    it("rejects a non-object payload", () => {
        expect(resolveBackfillStatus(null)).toBeNull();
        expect(resolveBackfillStatus(undefined)).toBeNull();
        expect(resolveBackfillStatus("not-an-object")).toBeNull();
    });
});

describe("useSubagentBackfillGate", () => {
    it("resolves immediately for a non-agent block, regardless of persisted session", () => {
        const { settled } = mount("term", true);
        expect(settled()).toBe(true);
    });

    it("resolves immediately for an agent block with NO persisted session (nothing will ever backfill)", () => {
        const { settled } = mount("agent", false);
        expect(settled()).toBe(true);
    });

    it("the P0 regression: a persisted-session agent block stays gated when the mount-time history read comes back empty", async () => {
        const { settled } = mount("agent", true);
        expect(settled()).toBe(false);

        // The mount-time history read resolves EMPTY -- the backend's scan
        // just hasn't started broadcasting yet, not "nothing pending."
        rpcHub.resolve?.([]);
        await Promise.resolve();
        await Promise.resolve();

        expect(settled()).toBe(false);
    });

    it("stays gated on a live 'started', settles (after the dock buffer) on a live 'done'", async () => {
        vi.useFakeTimers();
        const { settled } = mount("agent", true);
        rpcHub.resolve?.([]);
        await vi.advanceTimersByTimeAsync(0);
        expect(settled()).toBe(false);

        hub.handler?.({ data: { status: "started" } });
        expect(settled()).toBe(false);

        hub.handler?.({ data: { status: "done" } });
        expect(settled()).toBe(false); // buffer not elapsed yet

        await vi.advanceTimersByTimeAsync(250);
        expect(settled()).toBe(true);
    });

    it("the exact bug scenario: 'started' arriving AFTER an empty history read must not get stuck, and must not settle early", async () => {
        vi.useFakeTimers();
        const { settled } = mount("agent", true);

        // History read resolves empty first (the race this hook now
        // handles correctly).
        rpcHub.resolve?.([]);
        await vi.advanceTimersByTimeAsync(0);
        expect(settled()).toBe(false);

        // The real backend scan then actually starts.
        hub.handler?.({ data: { status: "started" } });
        expect(settled()).toBe(false);

        // ...and finishes.
        hub.handler?.({ data: { status: "done" } });
        await vi.advanceTimersByTimeAsync(250);
        expect(settled()).toBe(true);
    });

    it("safety net: reveals anyway if 'done' never arrives at all", async () => {
        vi.useFakeTimers();
        const { settled } = mount("agent", true);
        rpcHub.resolve?.([]);
        await vi.advanceTimersByTimeAsync(0);
        expect(settled()).toBe(false);

        await vi.advanceTimersByTimeAsync(20_000);
        expect(settled()).toBe(true);
    });

    it("a historical 'done' (already finished before mount) settles without waiting for a live event", async () => {
        vi.useFakeTimers();
        const { settled } = mount("agent", true);
        rpcHub.resolve?.([
            { data: { status: "started" } },
            { data: { status: "done" } },
        ]);
        await vi.advanceTimersByTimeAsync(0);
        await vi.advanceTimersByTimeAsync(250);
        expect(settled()).toBe(true);
    });

    // reagentx P2 (PR #2781, round 6): `wired` used to be a plain closure
    // flag that only ever flipped true, never reset on cleanup — so a
    // block whose view changes away from "agent" (e.g. "Replace With...")
    // and back again would never re-subscribe, leaving `settled` frozen at
    // whatever it last resolved to and silently disabling the gate for the
    // rest of that block's life.
    it("re-wires correctly after the view type changes away from agent and back", async () => {
        vi.useFakeTimers();
        const [viewType, setViewType] = createSignal<string | undefined>("agent");
        let settled: () => boolean = () => false;
        createRoot(() => {
            settled = useSubagentBackfillGate("block-1", viewType, () => true);
        });

        // First mount: stays gated (empty history, no live event yet).
        rpcHub.resolve?.([]);
        await vi.advanceTimersByTimeAsync(0);
        expect(settled()).toBe(false);

        // View changes away from "agent" — resolves immediately (this
        // block no longer has any backfill to wait on) and tears down the
        // subscription.
        setViewType("term");
        expect(settled()).toBe(true);
        expect(hub.handler).toBeNull();

        // View changes back to "agent" — must re-wire from scratch, not
        // silently stay settled=true forever.
        rpcHub.resolve = null;
        setViewType("agent");
        expect(settled()).toBe(false);
        expect(hub.handler).not.toBeNull();

        rpcHub.resolve?.([{ data: { status: "done" } }]);
        await vi.advanceTimersByTimeAsync(250);
        expect(settled()).toBe(true);
    });
});
