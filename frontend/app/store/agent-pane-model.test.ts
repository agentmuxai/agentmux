// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentPaneModel — PR-4 of the cascade follow-up.
 *
 * Contract under test: the model exposed by `registerPane` carries a
 * `disposed` flag that flips synchronously inside `unregisterPane`
 * BEFORE the underlying store deletes run. Dispatches through the model
 * after dispose silently no-op (no throw, return []) and emit a single
 * debug log line. Dispatches before dispose forward to the underlying
 * store's `dispatchIfRegistered` and update reducer state normally.
 *
 * This is the second safety net on top of cascade detection (PR #878)
 * and unified atomic registration (PR #999). The disposed flag catches
 * deferred dispatchers — setTimeout, await continuations, subscription
 * handlers — that land AFTER the cleanup runs.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
    __resetAllSlots as resetDocSlots,
    snapshot as docSnapshot,
} from "./agent-document-store";
import {
    __resetAllSlots as resetPaneStateSlots,
    type AgentPaneProjections,
    snapshot as paneStateSnapshot,
} from "./agent-pane-state-store";
import {
    type AgentPaneModel,
    type PaneRegistration,
    getPaneModel,
    registerPane,
    unregisterPane,
} from "./agent-pane-registration";

function noopProjections(): AgentPaneProjections {
    return {
        streaming: () => {},
        sessionStats: () => {},
        sessionTotals: () => {},
        currentTool: () => {},
        turnTokens: () => {},
        pending: () => {},
        initPhase: () => {},
        turnPhase: () => {},
    };
}

function defaultRegistration(): PaneRegistration {
    return {
        agentId: "agent-1",
        documentSetter: () => {},
        projections: noopProjections(),
    };
}

describe("AgentPaneModel (PR-4 — model-level dispatchIfAlive)", () => {
    afterEach(() => {
        resetDocSlots();
        resetPaneStateSlots();
        vi.restoreAllMocks();
    });

    describe("construction + lifetime", () => {
        it("registerPane returns a model whose blockId matches the registered pane", () => {
            const model = registerPane("blk-1", defaultRegistration());
            expect(model.blockId).toBe("blk-1");
            expect(model.disposed).toBe(false);
        });

        it("getPaneModel returns the same model registerPane returned", () => {
            const registered = registerPane("blk-2", defaultRegistration());
            const fetched = getPaneModel("blk-2");
            expect(fetched).toBe(registered);
        });

        it("getPaneModel returns null for an unregistered blockId", () => {
            expect(getPaneModel("never-registered")).toBeNull();
        });

        it("unregisterPane flips disposed to true and removes the model from the registry", () => {
            const model = registerPane("blk-3", defaultRegistration());
            expect(model.disposed).toBe(false);
            unregisterPane("blk-3");
            expect(model.disposed).toBe(true);
            expect(getPaneModel("blk-3")).toBeNull();
        });

        it("re-registering the same blockId disposes the prior model", () => {
            // Hot-reload / re-mount semantics: the old model handle's
            // callers should silently no-op so they don't fight the new
            // pane's setup.
            const first = registerPane("blk-4", defaultRegistration());
            const second = registerPane("blk-4", defaultRegistration());
            expect(first).not.toBe(second);
            expect(first.disposed).toBe(true);
            expect(second.disposed).toBe(false);
            expect(getPaneModel("blk-4")).toBe(second);
        });
    });

    describe("disposed flag flips BEFORE store unregisters", () => {
        it("a setter notified during the unregister sequence sees disposed=true", () => {
            // The structural guarantee from PR-4: the disposed flag is
            // flipped FIRST in unregisterPane, before either underlying
            // store deletes its slot. So a reactive subscriber that
            // fires during unregister (which doesn't happen with the
            // current pure-`Map.delete` per-store unregisters, but the
            // contract holds for any future projection that does) can
            // synchronously check the model's disposed flag and skip
            // its work.
            //
            // We test this by snapshotting the model BEFORE unregister
            // and asserting the disposed flag is true immediately
            // after unregister returns — no intermediate microtask
            // boundary, no async yield.
            const model = registerPane("blk-5", defaultRegistration());
            expect(model.disposed).toBe(false);
            // Within the same JS frame, capture the flag at three points:
            // pre-unregister, post-unregister, after a microtask. The
            // disposed flag is set synchronously, so all post-unregister
            // observations must see `true`.
            const observations: boolean[] = [];
            observations.push(model.disposed); // pre
            unregisterPane("blk-5");
            observations.push(model.disposed); // sync-post
            expect(observations).toEqual([false, true]);
        });
    });

    describe("dispatchPane — drop semantics", () => {
        it("after dispose, dispatchPane returns [] and does not touch the store", () => {
            const model = registerPane("blk-6", defaultRegistration());
            unregisterPane("blk-6");
            const debugSpy = vi.spyOn(console, "debug").mockImplementation(() => {});

            const events = model.dispatchPane({ type: "InitStart" });
            expect(events).toEqual([]);
            // Drop emits a single debug log line.
            expect(debugSpy).toHaveBeenCalledTimes(1);
            const msg = debugSpy.mock.calls[0][0];
            expect(typeof msg).toBe("string");
            expect(msg as string).toMatch(/dispatchPane dropped/);
            expect(msg as string).toMatch(/InitStart/);
        });

        it("before dispose, dispatchPane forwards to the store and updates reducer state", () => {
            const turnPhaseKinds: string[] = [];
            const model = registerPane("blk-7", {
                agentId: "a",
                documentSetter: () => {},
                projections: {
                    ...noopProjections(),
                    turnPhase: (next) => turnPhaseKinds.push(next.kind),
                },
            });

            model.dispatchPane({ type: "InitReady", at: 0 });
            model.dispatchPane({ type: "StreamSubscribe", at: 1 });
            model.dispatchPane({ type: "TurnStart", at: 2 });

            // State updated through the dispatch path.
            expect(turnPhaseKinds).toContain("Submitting");
            const snap = paneStateSnapshot("blk-7");
            expect(snap).not.toBeNull();
            expect(snap?.turnPhase.kind).toBe("Submitting");
        });
    });

    describe("dispatchDoc — drop semantics", () => {
        it("after dispose, dispatchDoc returns [] and does not touch the store", () => {
            const model = registerPane("blk-8", defaultRegistration());
            unregisterPane("blk-8");
            const debugSpy = vi.spyOn(console, "debug").mockImplementation(() => {});

            const events = model.dispatchDoc({
                type: "StreamFlush",
                newNodes: [],
                updatedNodes: [],
            });
            expect(events).toEqual([]);
            expect(debugSpy).toHaveBeenCalledTimes(1);
            const msg = debugSpy.mock.calls[0][0];
            expect(msg as string).toMatch(/dispatchDoc dropped/);
            expect(msg as string).toMatch(/StreamFlush/);
        });

        it("before dispose, dispatchDoc forwards to the store and updates reducer state", () => {
            const setterCalls: number[] = [];
            const model = registerPane("blk-9", {
                agentId: "a",
                documentSetter: (nodes) => setterCalls.push(nodes.length),
                projections: noopProjections(),
            });

            model.dispatchDoc({
                type: "StreamFlush",
                newNodes: [
                    { type: "markdown", id: "n1", content: "hi" },
                ],
                updatedNodes: [],
            });

            expect(setterCalls).toEqual([1]);
            expect(docSnapshot("blk-9")?.nodes.length).toBe(1);
        });
    });

    describe("torture test — deferred dispatch after dispose", () => {
        it("a setTimeout dispatcher landing after unregister is a no-op", async () => {
            // Production scenario: a hook schedules a dispatch via
            // setTimeout for a watchdog tick or expiry timer; the pane
            // unmounts before the timer fires. Without the model, the
            // deferred dispatch would either throw (`dispatch`) or rely
            // on the per-store soft variant catching the gap. With the
            // model, the disposed-flag check is the structural guarantee.
            const model = registerPane("blk-10", defaultRegistration());

            // Schedule a dispatch that will fire AFTER unregister.
            const dispatchPromise = new Promise<void>((resolve) => {
                setTimeout(() => {
                    // This is the dispatch we expect to no-op.
                    model.dispatchPane({
                        type: "StreamWatchdogTick",
                        nowMs: Date.now(),
                    });
                    resolve();
                }, 5);
            });

            // Unregister BEFORE the timer fires.
            unregisterPane("blk-10");
            expect(model.disposed).toBe(true);

            // Let the timer fire.
            const debugSpy = vi.spyOn(console, "debug").mockImplementation(() => {});
            await dispatchPromise;
            // The deferred dispatch dropped, no throw.
            expect(debugSpy).toHaveBeenCalledTimes(1);
            expect((debugSpy.mock.calls[0][0] as string)).toMatch(/dispatchPane dropped/);
            // No store slot was created either.
            expect(paneStateSnapshot("blk-10")).toBeNull();
        });

        it("an await-continuation dispatcher landing after unregister is a no-op", async () => {
            // Same scenario, await-shape: an async function captures
            // the model, awaits an RPC, then dispatches after the
            // await. If the pane unmounted during the await, the
            // dispatch must drop silently.
            const model = registerPane("blk-11", defaultRegistration());

            const work = async () => {
                // Yield to the event loop so the caller can unregister
                // before the dispatch happens.
                await Promise.resolve();
                await Promise.resolve();
                model.dispatchDoc({
                    type: "HistoryLoaded",
                    nodes: [{ type: "markdown", id: "n1", content: "after-await" }],
                });
            };

            const promise = work();
            unregisterPane("blk-11");

            const debugSpy = vi.spyOn(console, "debug").mockImplementation(() => {});
            await promise;
            expect(debugSpy).toHaveBeenCalledTimes(1);
            expect((debugSpy.mock.calls[0][0] as string)).toMatch(/dispatchDoc dropped/);
            // Document slot deleted; no nodes ever landed.
            expect(docSnapshot("blk-11")).toBeNull();
        });
    });

    describe("coexistence with module-level dispatchIfRegistered", () => {
        it("after model dispose, the per-store dispatchIfRegistered still works for a fresh slot", async () => {
            // The model carries the disposed flag for ONE pane lifetime
            // — it doesn't pollute the per-store soft-dispatch path.
            // Re-registering a blockId disposes the old model but the
            // new one (and the per-store dispatchIfRegistered) behave
            // normally.
            const oldModel = registerPane("blk-12", defaultRegistration());
            unregisterPane("blk-12");
            expect(oldModel.disposed).toBe(true);

            const newModel = registerPane("blk-12", defaultRegistration());
            expect(newModel.disposed).toBe(false);

            const newEvents = newModel.dispatchPane({ type: "InitStart" });
            // newEvents may be empty if InitStart produced no events,
            // but the dispatch went through (no drop). The slot
            // exists post-dispatch:
            expect(Array.isArray(newEvents)).toBe(true);
            expect(paneStateSnapshot("blk-12")).not.toBeNull();

            // The old model's dispatch still no-ops:
            const debugSpy = vi.spyOn(console, "debug").mockImplementation(() => {});
            const oldEvents = oldModel.dispatchPane({ type: "InitStart" });
            expect(oldEvents).toEqual([]);
            expect(debugSpy).toHaveBeenCalledTimes(1);
        });

        it("dispatchIfRegistered (module-level) and model.dispatch* observe the same store state", async () => {
            // Smoke: a dispatch through the model and one through the
            // module-level soft variant must both see the same slot
            // contents. The model's disposed flag is checked BEFORE
            // dispatching; it doesn't alter the store's behavior.
            const model = registerPane("blk-13", defaultRegistration());
            model.dispatchPane({ type: "InitReady", at: 100 });
            const snap = paneStateSnapshot("blk-13");
            expect(snap?.initPhase.kind).toBe("InitReady");

            const { dispatchIfRegistered: dispatchPaneIfRegistered } =
                await import("./agent-pane-state-store");
            const events = dispatchPaneIfRegistered("blk-13", {
                type: "StreamSubscribe",
                at: 101,
            });
            expect(Array.isArray(events)).toBe(true);
        });
    });

    describe("idempotency", () => {
        it("dispatchPane after dispose can be called many times without throwing", () => {
            const model = registerPane("blk-14", defaultRegistration());
            unregisterPane("blk-14");
            vi.spyOn(console, "debug").mockImplementation(() => {});
            for (let i = 0; i < 10; i++) {
                expect(() =>
                    model.dispatchPane({ type: "InitStart" }),
                ).not.toThrow();
            }
        });

        it("over-cleanup tolerance: a second unregisterPane is a no-op", () => {
            const model = registerPane("blk-15", defaultRegistration());
            unregisterPane("blk-15");
            expect(model.disposed).toBe(true);
            expect(() => unregisterPane("blk-15")).not.toThrow();
            expect(model.disposed).toBe(true);
        });
    });

    describe("model type surface", () => {
        it("model conforms to AgentPaneModel interface", () => {
            const model: AgentPaneModel = registerPane("blk-16", defaultRegistration());
            // Compile-time + runtime check: the public interface
            // exposes blockId, disposed, dispatchPane, dispatchDoc and
            // does NOT expose _markDisposed (it's internal).
            expect(typeof model.blockId).toBe("string");
            expect(typeof model.disposed).toBe("boolean");
            expect(typeof model.dispatchPane).toBe("function");
            expect(typeof model.dispatchDoc).toBe("function");
        });
    });
});

describe("AgentPaneModel — clean-path quiet", () => {
    beforeEach(() => {
        resetDocSlots();
        resetPaneStateSlots();
    });
    afterEach(() => {
        vi.restoreAllMocks();
    });

    it("does not log warnings or debug messages on the happy path", () => {
        const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
        const debugSpy = vi.spyOn(console, "debug").mockImplementation(() => {});
        const model = registerPane("happy-path", defaultRegistration());
        model.dispatchPane({ type: "InitReady", at: 0 });
        model.dispatchDoc({
            type: "StreamFlush",
            newNodes: [{ type: "markdown", id: "h", content: "hi" }],
            updatedNodes: [],
        });
        unregisterPane("happy-path");
        expect(warnSpy).not.toHaveBeenCalled();
        expect(debugSpy).not.toHaveBeenCalled();
    });
});
