// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Regression coverage for the cascade-dispose lifecycle vulnerability
 * documented in docs/analysis/LIFECYCLE_DISPATCH_LEAK_2026_05_15.md.
 *
 * The scenario: a projection setter's reactive subscriber synchronously
 * unmounts the pane (calls `unregisterPane`) inside the dispatch's
 * setter call. The setter returns, and the caller's NEXT dispatch on
 * the same blockId hits a gone slot. The two contracts that need to
 * hold:
 *
 *   1. `dispatch()` still throws if the slot is gone — registration
 *      ordering bugs must remain loud.
 *   2. `dispatchIfRegistered()` silently no-ops in the same scenario —
 *      gives async-context callers a way to dispatch defensively without
 *      catching their own throws.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import {
    __resetAllSlots,
    type AgentPaneProjections,
    dispatch,
    dispatchIfRegistered,
    registerPane,
    setEventSink,
    unregisterPane,
} from "./agent-pane-state-store";

function noopProj(): AgentPaneProjections {
    return {
        streaming: () => {},
        sessionStats: () => {},
        currentTool: () => {},
        turnTokens: () => {},
        turnActive: () => {},
        stopping: () => {},
        pending: () => {},
        initPhase: () => {},
    };
}

describe("agent-pane-state-store (cascade contracts)", () => {
    afterEach(() => {
        __resetAllSlots();
        setEventSink(() => {});
    });

    describe("dispatch (throwing variant)", () => {
        it("throws on unregistered blockId", () => {
            expect(() =>
                dispatch("blockA", { type: "StreamFlushObserved", addedCount: 0, at: 0 }),
            ).toThrowError(/dispatch for unregistered pane/);
        });

        it("still throws when a setter cascade-disposes the slot mid-dispatch", () => {
            // Setter that simulates a reactive subscriber unmounting the
            // pane: synchronously calls `unregisterPane` during its own
            // notification. The cascade-detection inside the store will
            // emit a CASCADE_DETECTED warning. The dispatch call itself
            // returns events normally — but the next dispatch throws.
            // `streaming` is the first setter the dispatch loop touches
            // when state.streaming changes — StreamSubscribe always
            // toggles it. Use that as the cascading site so the test
            // doesn't depend on internal projection-order details.
            const proj: AgentPaneProjections = {
                ...noopProj(),
                streaming: () => {
                    unregisterPane("blockA"); // cascade: subscriber disposed pane
                },
            };
            registerPane("blockA", "agentA", proj);

            const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
            try {
                // First dispatch: setter cascade-disposes the slot.
                dispatch("blockA", { type: "StreamSubscribe", at: 0 });
                // Cascade-detection log should fire.
                const cascadeWarns = warnSpy.mock.calls.filter(([msg]) =>
                    typeof msg === "string" && msg.includes("CASCADE_DETECTED"),
                );
                expect(cascadeWarns.length).toBeGreaterThan(0);
                expect(cascadeWarns[0][0]).toContain("'streaming'"); // names the cascading setter

                // Second dispatch: throws because slot is gone.
                expect(() =>
                    dispatch("blockA", { type: "StreamFlushObserved", addedCount: 0, at: 1 }),
                ).toThrowError(/dispatch for unregistered pane/);
            } finally {
                warnSpy.mockRestore();
            }
        });
    });

    describe("dispatchIfRegistered (soft variant)", () => {
        it("returns [] silently for an unregistered blockId", () => {
            const events = dispatchIfRegistered("ghostBlock", {
                type: "StreamFlushObserved",
                addedCount: 0,
                at: 0,
            });
            expect(events).toEqual([]);
        });

        it("dispatches normally when the slot exists", () => {
            registerPane("blockB", "agentA", noopProj());
            const events = dispatchIfRegistered("blockB", { type: "TurnStart", at: 0 });
            // TurnStart produces events; we don't care about content here,
            // just that the call went through the real reducer.
            expect(Array.isArray(events)).toBe(true);
        });

        it("no-ops after a setter cascade-disposes the slot — no throw", () => {
            // This is the production scenario from
            // LIFECYCLE_DISPATCH_LEAK_2026_05_15.md: a documentAtom
            // subscriber unmounted the pane during a StreamFlush; the
            // next call (in useAgentStream.flushPendingNodes) is
            // StreamFlushObserved against agent-pane-state. With the
            // soft variant, that next call returns [] silently.
            const proj: AgentPaneProjections = {
                ...noopProj(),
                streaming: () => {
                    unregisterPane("blockC"); // cascade
                },
            };
            registerPane("blockC", "agentA", proj);

            const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
            try {
                // Trigger the cascade via the throwing dispatch (the
                // path that does the actual projection work).
                dispatch("blockC", { type: "StreamSubscribe", at: 0 });
                // The simulated "next dispatch in the same caller frame":
                // with the soft variant, no throw, just an empty array.
                const events = dispatchIfRegistered("blockC", {
                    type: "StreamFlushObserved",
                    addedCount: 0,
                    at: 1,
                });
                expect(events).toEqual([]);
            } finally {
                warnSpy.mockRestore();
            }
        });
    });

    // ── PR B (turn-phase view migration) ────────────────────────────────
    // PR A added `turnPhase` to the reducer state with dual-write; PR B —
    // this PR — exposes a projection setter for it so the agent VIEW can
    // bind its "working" animation to `workingFromPhase(turnPhase)` via
    // a Solid signal. Verify the setter is invoked with the dual-written
    // phase value at each lifecycle transition.
    describe("turnPhase projection (PR B)", () => {
        it("calls the turnPhase setter with the dual-written phase value", () => {
            const phaseCalls: { kind: string }[] = [];
            const proj: AgentPaneProjections = {
                ...noopProj(),
                turnPhase: (next) => {
                    phaseCalls.push({ kind: next.kind });
                },
            };
            registerPane("blockD", "agentA", proj);

            // Sequence: InitReady → StreamSubscribe (Idle stays Idle)
            //         → TurnStart (Idle → Submitting)
            //         → StreamFlushObserved (Submitting → Streaming)
            //         → RequestStop (Streaming → Interrupting)
            //         → TurnEnd (Interrupting → Done.stopped)
            dispatch("blockD", { type: "InitReady" });
            dispatch("blockD", { type: "StreamSubscribe", at: 100 });
            dispatch("blockD", { type: "TurnStart", at: 110 });
            dispatch("blockD", { type: "StreamFlushObserved", addedCount: 1, at: 120 });
            dispatch("blockD", { type: "RequestStop", at: 130 });
            dispatch("blockD", { type: "TurnEnd", stats: null });

            // Only ACTUAL transitions hit the setter (reducer skips
            // setter calls when prev === next reference). Subscribing
            // from Idle keeps phase Idle (no spontaneous promotion), so
            // there's no setter call there.
            const kinds = phaseCalls.map((c) => c.kind);
            expect(kinds).toEqual([
                "Submitting", // TurnStart
                "Streaming", // StreamFlushObserved promotes from Submitting
                "Interrupting", // RequestStop while working
                "Done", // TurnEnd
            ]);
        });
    });
});
