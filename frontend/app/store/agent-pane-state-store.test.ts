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
    __resetListeners,
    addEventListener,
    type AgentPaneEvent,
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
        sessionTotals: () => {},
        currentTool: () => {},
        turnTokens: () => {},
        pending: () => {},
        initPhase: () => {},
    };
}

describe("agent-pane-state-store (cascade contracts)", () => {
    afterEach(() => {
        __resetAllSlots();
        __resetListeners();
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
            dispatch("blockD", { type: "InitReady", at: 100 });
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

    // ────────────────────────────────────────────────────────────────
    // Multicast event listeners — sound-notifications subsystem path.
    // See docs/specs/SPEC_SOUND_NOTIFICATIONS_2026_06_05.md §4.4 Path B.
    // ────────────────────────────────────────────────────────────────
    describe("addEventListener (multicast)", () => {
        it("delivers every emitted event to subscribers in addition to the single sink", () => {
            const singleSink = vi.fn();
            const listener = vi.fn();
            setEventSink(singleSink);
            addEventListener(listener);

            registerPane("blockX", "agentX", noopProj());
            dispatch("blockX", { type: "InitReady", at: 100 });

            // The single sink and the listener both saw the same event.
            expect(singleSink).toHaveBeenCalledTimes(1);
            expect(listener).toHaveBeenCalledTimes(1);
            expect(singleSink.mock.calls[0][0]).toBe("blockX");
            expect(listener.mock.calls[0][0]).toBe("blockX");
            expect(listener.mock.calls[0][1]).toMatchObject({ type: "init-ready" });
        });

        it("unsubscribe stops further delivery to the listener but not the sink", () => {
            const sink = vi.fn();
            const listener = vi.fn();
            setEventSink(sink);
            const unsub = addEventListener(listener);

            registerPane("blockY", "agentY", noopProj());
            dispatch("blockY", { type: "InitReady", at: 100 });
            unsub();
            dispatch("blockY", { type: "StreamSubscribe", at: 110 });

            expect(listener).toHaveBeenCalledTimes(1); // only the first
            expect(sink).toHaveBeenCalledTimes(2); // both
        });

        it("a throwing listener does not poison the sink or other listeners", () => {
            const sink = vi.fn();
            const thrower = vi.fn(() => {
                throw new Error("listener boom");
            });
            const good = vi.fn();
            setEventSink(sink);
            addEventListener(thrower);
            addEventListener(good);

            const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
            registerPane("blockZ", "agentZ", noopProj());
            dispatch("blockZ", { type: "InitReady", at: 100 });

            expect(sink).toHaveBeenCalledTimes(1);
            expect(thrower).toHaveBeenCalledTimes(1);
            expect(good).toHaveBeenCalledTimes(1);
            expect(warn).toHaveBeenCalled();
            warn.mockRestore();
        });

        it("turn-ended event carries the outcome for sound-notifications consumers", () => {
            const seen: AgentPaneEvent[] = [];
            setEventSink(() => {});
            addEventListener((_blockId, ev) => {
                seen.push(ev);
            });

            registerPane("blockTE", "agentTE", noopProj());
            dispatch("blockTE", { type: "InitReady", at: 100 });
            dispatch("blockTE", { type: "StreamSubscribe", at: 100 });
            dispatch("blockTE", { type: "TurnStart", at: 110 });
            dispatch("blockTE", { type: "StreamFlushObserved", addedCount: 1, at: 120 });
            dispatch("blockTE", { type: "TurnEnd", stats: null });

            const turnEnded = seen.find((e) => e.type === "turn-ended");
            expect(turnEnded).toBeDefined();
            expect(turnEnded).toMatchObject({
                type: "turn-ended",
                // Completed because no RequestStop happened.
                outcome: "completed",
            });
        });
    });

    // ────────────────────────────────────────────────────────────────
    // SPEC_AGENT_TURN_PHASE_TIMELINE_LOGGING_2026_08_18.md — [wave-turn]
    // transition logging must NOT auto-classify a StreamFlushObserved
    // re-promotion as anomalous. An earlier version of this line tagged
    // any promotion from a non-Submitting phase as `(stray)`; reagent P1
    // on PR #2653 caught that Idle/Disconnected (stream drop + resubscribe)
    // and Done.completed (a normal multi-round tool continuation —
    // session_end fires after every model API round) are BOTH documented,
    // legitimate cases in reducer.ts's own StreamFlushObserved arm, not
    // anomalies — so the blanket heuristic mislabeled the common healthy
    // path. Removed; these tests lock down that no such mislabeling
    // returns, covering every promotion source the reducer allows,
    // including the specific Done.completed case that had zero coverage
    // before (reagent's own callout).
    // ────────────────────────────────────────────────────────────────
    describe("[wave-turn] StreamFlushObserved re-promotion — no anomaly auto-tagging", () => {
        it.each([
            ["Idle", (id: string) => {
                dispatch(id, { type: "InitReady", at: 100 });
                dispatch(id, { type: "StreamSubscribe", at: 100 }); // sets lastEventMs, stays Idle
            }],
            ["Submitting", (id: string) => {
                dispatch(id, { type: "InitReady", at: 100 });
                dispatch(id, { type: "StreamSubscribe", at: 100 });
                dispatch(id, { type: "TurnStart", at: 110 });
            }],
            ["Done", (id: string) => {
                // Reaches Done with outcome "completed" — no RequestStop.
                dispatch(id, { type: "InitReady", at: 100 });
                dispatch(id, { type: "StreamSubscribe", at: 100 });
                dispatch(id, { type: "TurnStart", at: 110 });
                dispatch(id, { type: "StreamFlushObserved", addedCount: 1, at: 120 });
                dispatch(id, { type: "TurnEnd", stats: null });
            }],
        ])("promoting from %s never appends an anomaly label to the transition line", (fromKind, setup) => {
            const info = vi.spyOn(console, "info").mockImplementation(() => {});
            try {
                const blockId = `block-${fromKind}`;
                registerPane(blockId, "agentA", noopProj());
                setup(blockId);
                info.mockClear();

                dispatch(blockId, { type: "StreamFlushObserved", addedCount: 1, at: 999 });

                const line = info.mock.calls.find((c) => c[0] === "[wave-turn]");
                expect(line).toBeDefined();
                expect(line?.join(" ")).toContain(`${fromKind} → Streaming`);
                // No parenthetical anomaly tag of any kind — the raw
                // transition is all this line ever claims.
                expect(line?.join(" ")).not.toMatch(/\(\w+\)/);
            } finally {
                info.mockRestore();
            }
        });
    });

    describe("[wave-turn] watchdog tick heartbeat", () => {
        it("logs a heartbeat only every 12th StreamWatchdogTick dispatch", () => {
            const info = vi.spyOn(console, "info").mockImplementation(() => {});
            try {
                registerPane("blockTick", "agentA", noopProj());
                info.mockClear();

                const heartbeatCalls = () =>
                    info.mock.calls.filter((c) => c.join(" ").includes("watchdog: tick #"));

                for (let i = 1; i <= 11; i++) {
                    dispatch("blockTick", { type: "StreamWatchdogTick", nowMs: i * 5000 });
                }
                expect(heartbeatCalls()).toHaveLength(0);

                dispatch("blockTick", { type: "StreamWatchdogTick", nowMs: 12 * 5000 });
                expect(heartbeatCalls()).toHaveLength(1);
                expect(heartbeatCalls()[0].join(" ")).toContain("watchdog: tick #12");

                // Counts independently of whether the reducer itself no-ops
                // (unsubscribed pane here — lastEventMs is still null) —
                // this heartbeat is proof the INTERVAL dispatched the
                // command at all, not proof the reducer found work to do.
                for (let i = 13; i <= 23; i++) {
                    dispatch("blockTick", { type: "StreamWatchdogTick", nowMs: i * 5000 });
                }
                expect(heartbeatCalls()).toHaveLength(1);
                dispatch("blockTick", { type: "StreamWatchdogTick", nowMs: 24 * 5000 });
                expect(heartbeatCalls()).toHaveLength(2);
            } finally {
                info.mockRestore();
            }
        });
    });
});
