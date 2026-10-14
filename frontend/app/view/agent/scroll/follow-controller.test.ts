// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import {
    DETACHED,
    FOLLOWING,
    NEAR_TOP_PX,
    REATTACH_PX,
    followReducer,
    followStateLabel,
    intentOf,
    resolveFollowController,
    suspended,
    type FollowEffect,
    type FollowEvent,
    type FollowState,
    type ScrollGeometry,
} from "./follow-controller";

/** A 300 px viewport over 1000 px of content, `gap` px above the true bottom. */
const at = (gap: number, scrollHeight = 1000, clientHeight = 300): ScrollGeometry => ({
    scrollTop: scrollHeight - clientHeight - gap,
    scrollHeight,
    clientHeight,
});
const BOTTOM = at(0);

const scroll = (
    prev: ScrollGeometry,
    geo: ScrollGeometry,
    over: Partial<Extract<FollowEvent, { type: "Scroll" }>> = {}
): FollowEvent => ({
    type: "Scroll",
    user: true,
    prev,
    geo,
    hasOlderHistory: false,
    loadingOlder: false,
    ...over,
});

const types = (effects: FollowEffect[]): string[] => effects.map((e) => e.type);
const transition = (effects: FollowEffect[]) =>
    effects.find((e) => e.type === "Transition") as Extract<FollowEffect, { type: "Transition" }> | undefined;
const pinCause = (effects: FollowEffect[]) =>
    (effects.find((e) => e.type === "Pin") as { cause: string } | undefined)?.cause;

/** Reduce a sequence, collecting every effect. */
function run(state: FollowState, events: FollowEvent[]): { state: FollowState; effects: FollowEffect[] } {
    const effects: FollowEffect[] = [];
    for (const e of events) {
        const r = followReducer(state, e);
        state = r.state;
        effects.push(...r.effects);
    }
    return { state, effects };
}

const ALL_STATES: FollowState[] = [FOLLOWING, DETACHED, suspended("FOLLOWING"), suspended("DETACHED")];

describe("followReducer — §5.3, row by row", () => {
    describe("ContentResized / ViewportResized", () => {
        for (const type of ["ContentResized", "ViewportResized"] as const) {
            const cause = type === "ContentResized" ? "content" : "viewport";

            it(`${type} while FOLLOWING pins and stays FOLLOWING`, () => {
                const r = followReducer(FOLLOWING, { type, geo: at(400) });
                expect(r.state).toBe(FOLLOWING);
                expect(r.effects).toEqual([{ type: "Pin", cause }]);
            });

            it(`${type} while DETACHED doesn't move (content grew below the reader)`, () => {
                const r = followReducer(DETACHED, { type, geo: at(900, 2000) });
                expect(r.state).toBe(DETACHED);
                expect(r.effects).toEqual([]);
            });

            it(`${type} while DETACHED with nothing left to scroll follows again`, () => {
                const r = followReducer(DETACHED, {
                    type,
                    geo: { scrollTop: 0, scrollHeight: 280, clientHeight: 300 },
                });
                expect(r.state).toBe(FOLLOWING);
                expect(types(r.effects)).toEqual(["Transition", "Pin"]);
                expect(transition(r.effects)!.cause).toBe("no-overflow");
            });

            it(`${type} while SUSPENDED is ignored`, () => {
                for (const s of [suspended("FOLLOWING"), suspended("DETACHED")]) {
                    const r = followReducer(s, { type, geo: at(10) });
                    expect(r.state).toBe(s);
                    expect(r.effects).toEqual([]);
                }
            });
        }
    });

    describe("UserEscape", () => {
        for (const cause of ["wheel-up", "key", "selection"] as const) {
            it(`${cause} detaches a following pane at once`, () => {
                const r = followReducer(FOLLOWING, { type: "UserEscape", cause, geo: BOTTOM });
                expect(r.state).toBe(DETACHED);
                expect(transition(r.effects)).toMatchObject({ cause: `user-escape:${cause}`, detail: "gap=0px" });
                expect(pinCause(r.effects)).toBeUndefined();
            });
        }

        it("does nothing at the top (nothing overflows yet: a new pane's first wheel)", () => {
            const r = followReducer(FOLLOWING, {
                type: "UserEscape",
                cause: "wheel-up",
                geo: { scrollTop: 0, scrollHeight: 280, clientHeight: 300 },
            });
            expect(r.state).toBe(FOLLOWING);
            expect(r.effects).toEqual([]);
        });

        it("does nothing when not FOLLOWING", () => {
            for (const s of [DETACHED, suspended("FOLLOWING"), suspended("DETACHED")]) {
                expect(followReducer(s, { type: "UserEscape", cause: "key", geo: BOTTOM })).toEqual({
                    state: s,
                    effects: [],
                });
            }
        });
    });

    describe("Scroll while FOLLOWING", () => {
        it("a user scroll up (same scrollHeight) detaches, with the gap", () => {
            const r = followReducer(FOLLOWING, scroll(BOTTOM, at(1 + 1)));
            expect(r.state).toBe(DETACHED);
            expect(transition(r.effects)).toMatchObject({ cause: "user-scroll", detail: "gap=2px" });
        });

        it("a scrollbar drag up says so", () => {
            const r = followReducer(FOLLOWING, scroll(BOTTOM, at(300), { scrollbar: true }));
            expect(transition(r.effects)!.cause).toBe("user-scroll:scrollbar");
        });

        it("a move of 1 px or less is fractional-zoom noise", () => {
            expect(followReducer(FOLLOWING, scroll(BOTTOM, at(1))).state).toBe(FOLLOWING);
        });

        it("a user scroll down never detaches, even short of the bottom", () => {
            const r = followReducer(FOLLOWING, scroll(at(500), at(300)));
            expect(r).toEqual({ state: FOLLOWING, effects: [] });
        });

        it("B2: a clamp (top and scrollHeight both shrink) during a gesture doesn't detach", () => {
            const r = followReducer(FOLLOWING, scroll(at(0, 1000), at(0, 800)));
            expect(r).toEqual({ state: FOLLOWING, effects: [] });
        });

        it("B2: a scroll with no user-intent window is ignored, whatever its geometry", () => {
            for (const geo of [at(600), { scrollTop: 0, scrollHeight: 1000, clientHeight: 300 }, at(0, 400)]) {
                expect(followReducer(FOLLOWING, scroll(BOTTOM, geo, { user: false, hasOlderHistory: true }))).toEqual({
                    state: FOLLOWING,
                    effects: [],
                });
            }
        });

        it("A1: our own small first-overflow pin near the top neither detaches nor pages", () => {
            // ContentResized lands the pin at scrollTop 12, then its scroll batch (no window).
            const pinned = { scrollTop: 12, scrollHeight: 312, clientHeight: 300 };
            const r = run(FOLLOWING, [
                { type: "ContentResized", geo: { scrollTop: 0, scrollHeight: 312, clientHeight: 300 } },
                scroll({ scrollTop: 0, scrollHeight: 312, clientHeight: 300 }, pinned, {
                    user: false,
                    hasOlderHistory: true,
                }),
            ]);
            expect(r.state).toBe(FOLLOWING);
            expect(types(r.effects)).toEqual(["Pin"]);
        });

        it("a user scroll up to the top detaches and pages in older history in the same batch", () => {
            const r = followReducer(
                FOLLOWING,
                scroll(BOTTOM, { scrollTop: 10, scrollHeight: 1000, clientHeight: 300 }, { hasOlderHistory: true })
            );
            expect(r.state).toBe(DETACHED);
            expect(types(r.effects)).toEqual(["Transition", "LoadOlder"]);
        });

        it("never pages while FOLLOWING (I2)", () => {
            // A short range: the bottom is also near the top. Down, so no detach.
            const r = followReducer(
                FOLLOWING,
                scroll(
                    { scrollTop: 5, scrollHeight: 320, clientHeight: 300 },
                    { scrollTop: 15, scrollHeight: 320, clientHeight: 300 },
                    { hasOlderHistory: true }
                )
            );
            expect(r).toEqual({ state: FOLLOWING, effects: [] });
        });
    });

    describe("Scroll while DETACHED", () => {
        it(`a user scroll that ends within REATTACH_PX (${REATTACH_PX}) of the bottom follows again and pins`, () => {
            const r = followReducer(DETACHED, scroll(at(400), at(REATTACH_PX)));
            expect(r.state).toBe(FOLLOWING);
            expect(transition(r.effects)).toMatchObject({
                cause: "user-scroll-to-bottom",
                detail: `gap=${REATTACH_PX}px`,
            });
            expect(pinCause(r.effects)).toBe("user-scroll-to-bottom");
        });

        it("just outside REATTACH_PX stays detached (no 200 px zone, no halving)", () => {
            expect(followReducer(DETACHED, scroll(at(400), at(REATTACH_PX + 1))).state).toBe(DETACHED);
            // A short range: the old rule made half of it "near bottom".
            const short = { scrollHeight: 400, clientHeight: 300 };
            expect(followReducer(DETACHED, scroll({ ...short, scrollTop: 0 }, { ...short, scrollTop: 60 })).state).toBe(
                DETACHED
            );
        });

        it("a small wheel-up near the bottom is not pulled back (moving up never re-attaches)", () => {
            const r = run(FOLLOWING, [{ type: "UserEscape", cause: "wheel-up", geo: BOTTOM }, scroll(BOTTOM, at(10))]);
            expect(r.state).toBe(DETACHED);
        });

        it("a scroll with no user-intent window never re-attaches, even at the bottom", () => {
            expect(followReducer(DETACHED, scroll(at(400), at(0), { user: false }))).toEqual({
                state: DETACHED,
                effects: [],
            });
        });

        it("a user scroll near the top pages when older history exists, staying DETACHED (I2)", () => {
            const top = { scrollTop: NEAR_TOP_PX - 1, scrollHeight: 1000, clientHeight: 300 };
            const r = followReducer(DETACHED, scroll(at(400), top, { hasOlderHistory: true }));
            expect(r).toEqual({ state: DETACHED, effects: [{ type: "LoadOlder" }] });
        });

        it("doesn't page without older history, while a page is loading, or outside NEAR_TOP_PX", () => {
            const top = { scrollTop: 10, scrollHeight: 1000, clientHeight: 300 };
            expect(followReducer(DETACHED, scroll(at(400), top)).effects).toEqual([]);
            expect(
                followReducer(DETACHED, scroll(at(400), top, { hasOlderHistory: true, loadingOlder: true })).effects
            ).toEqual([]);
            const notTop = { scrollTop: NEAR_TOP_PX, scrollHeight: 1000, clientHeight: 300 };
            expect(followReducer(DETACHED, scroll(at(400), notTop, { hasOlderHistory: true })).effects).toEqual([]);
        });

        it("A: a browser-made scroll near the top never pages, even with older history", () => {
            const top = { scrollTop: 5, scrollHeight: 1000, clientHeight: 300 };
            expect(
                followReducer(DETACHED, scroll(at(400), top, { user: false, hasOlderHistory: true })).effects
            ).toEqual([]);
        });
    });

    describe("Scroll while SUSPENDED", () => {
        it("is ignored, user or not", () => {
            for (const s of [suspended("FOLLOWING"), suspended("DETACHED")]) {
                expect(followReducer(s, scroll(BOTTOM, at(600), { hasOlderHistory: true }))).toEqual({
                    state: s,
                    effects: [],
                });
                expect(followReducer(s, scroll(at(600), BOTTOM))).toEqual({ state: s, effects: [] });
            }
        });
    });

    describe("JumpToBottom", () => {
        it("from DETACHED follows and pins, with the reason as the cause", () => {
            const r = followReducer(DETACHED, { type: "JumpToBottom", reason: "sent" });
            expect(r.state).toBe(FOLLOWING);
            expect(r.effects).toEqual([
                { type: "Transition", from: DETACHED, to: FOLLOWING, cause: "jump-to-bottom:sent", detail: undefined },
                { type: "Pin", cause: "jump-to-bottom:sent" },
            ]);
        });

        it("while already FOLLOWING still pins (typing), with no transition", () => {
            const r = followReducer(FOLLOWING, { type: "JumpToBottom", reason: "typing" });
            expect(r).toEqual({ state: FOLLOWING, effects: [{ type: "Pin", cause: "jump-to-bottom:typing" }] });
        });

        it("while hidden sets the intent and pins once shown", () => {
            const r = run(suspended("DETACHED"), [{ type: "JumpToBottom", reason: "queued-turn" }]);
            expect(r.state).toEqual(suspended("FOLLOWING"));
            expect(pinCause(r.effects)).toBeUndefined();
            expect(intentOf(r.state)).toBe("FOLLOWING");
            const shown = followReducer(r.state, { type: "BecameSized" });
            expect(shown.state).toBe(FOLLOWING);
            expect(pinCause(shown.effects)).toBe("sized");
        });
    });

    describe("JumpToNode", () => {
        it("detaches from any state, keeping a hidden pane hidden", () => {
            const r = followReducer(FOLLOWING, { type: "JumpToNode", nodeId: "node-1234567890" });
            expect(r.state).toBe(DETACHED);
            expect(transition(r.effects)).toMatchObject({ cause: "jump-to-node", detail: "node=node-123" });
            expect(followReducer(DETACHED, { type: "JumpToNode", nodeId: "x" })).toEqual({
                state: DETACHED,
                effects: [],
            });
            expect(followReducer(suspended("FOLLOWING"), { type: "JumpToNode", nodeId: "x" }).state).toEqual(
                suspended("DETACHED")
            );
        });

        it("a pin afterwards can't pull it back: content growth leaves it put", () => {
            const r = run(FOLLOWING, [
                { type: "JumpToNode", nodeId: "x" },
                { type: "ContentResized", geo: at(500, 2000) },
            ]);
            expect(r.state).toBe(DETACHED);
            expect(pinCause(r.effects)).toBeUndefined();
        });
    });

    describe("BecameZeroSize / BecameSized", () => {
        it("hidden suspends with the previous state; shown restores it", () => {
            for (const s of [FOLLOWING, DETACHED]) {
                const hidden = followReducer(s, { type: "BecameZeroSize" });
                expect(hidden.state).toEqual(suspended(s.kind as "FOLLOWING" | "DETACHED"));
                expect(transition(hidden.effects)).toMatchObject({ cause: "zero-size" });
                const shown = followReducer(hidden.state, { type: "BecameSized" });
                expect(shown.state).toBe(s);
                expect(transition(shown.effects)).toMatchObject({ cause: "sized" });
                // FOLLOWING pins; DETACHED leaves the reader's scrollTop alone.
                expect(pinCause(shown.effects)).toBe(s.kind === "FOLLOWING" ? "sized" : undefined);
            }
        });

        it("hidden twice is one suspension; sized while not suspended is nothing", () => {
            expect(followReducer(suspended("FOLLOWING"), { type: "BecameZeroSize" }).effects).toEqual([]);
            expect(followReducer(FOLLOWING, { type: "BecameSized" })).toEqual({ state: FOLLOWING, effects: [] });
            expect(followReducer(DETACHED, { type: "BecameSized" })).toEqual({ state: DETACHED, effects: [] });
        });

        it("a clamp while hidden (scrollTop reset to 0) changes nothing", () => {
            const r = run(FOLLOWING, [
                { type: "BecameZeroSize" },
                scroll(BOTTOM, { scrollTop: 0, scrollHeight: 1000, clientHeight: 300 }, { hasOlderHistory: true }),
                { type: "BecameSized" },
            ]);
            expect(r.state).toBe(FOLLOWING);
            expect(types(r.effects)).toEqual(["Transition", "Transition", "Pin"]);
        });
    });

    describe("Reset", () => {
        it("/clear follows again from any state, without a pin (nothing to pin to)", () => {
            expect(followReducer(DETACHED, { type: "Reset" })).toMatchObject({
                state: FOLLOWING,
                effects: [{ type: "Transition", cause: "reset" }],
            });
            expect(followReducer(FOLLOWING, { type: "Reset" })).toEqual({ state: FOLLOWING, effects: [] });
            expect(followReducer(suspended("DETACHED"), { type: "Reset" }).state).toEqual(suspended("FOLLOWING"));
        });
    });
});

describe("followReducer — invariants", () => {
    // Every event the binding can produce, from every state.
    const EVENTS: FollowEvent[] = [
        { type: "ContentResized", geo: at(300) },
        { type: "ContentResized", geo: { scrollTop: 0, scrollHeight: 100, clientHeight: 300 } },
        { type: "ViewportResized", geo: at(50) },
        { type: "UserEscape", cause: "wheel-up", geo: BOTTOM },
        { type: "UserEscape", cause: "selection", geo: { scrollTop: 0, scrollHeight: 100, clientHeight: 300 } },
        scroll(BOTTOM, at(600), { user: false, hasOlderHistory: true }),
        scroll(BOTTOM, { scrollTop: 3, scrollHeight: 1000, clientHeight: 300 }, { hasOlderHistory: true }),
        scroll(at(600), BOTTOM),
        scroll(at(0, 1000), at(0, 700)),
        { type: "JumpToBottom", reason: "typing" },
        { type: "JumpToNode", nodeId: "n" },
        { type: "BecameZeroSize" },
        { type: "BecameSized" },
        { type: "Reset" },
    ];

    it("I1: only a user gesture leaves FOLLOWING", () => {
        const gestures = new Set(["UserEscape", "JumpToNode"]);
        for (const e of EVENTS) {
            const r = followReducer(FOLLOWING, e);
            if (intentOf(r.state) === "DETACHED") {
                const userScroll = e.type === "Scroll" && e.user;
                expect(gestures.has(e.type) || userScroll, `${e.type} detached`).toBe(true);
            }
        }
    });

    it("I2: LoadOlder only from a user scroll, never from FOLLOWING's own state, and never changes intent by itself", () => {
        for (const s of ALL_STATES) {
            for (const e of EVENTS) {
                const r = followReducer(s, e);
                if (!r.effects.some((x) => x.type === "LoadOlder")) continue;
                expect(e.type === "Scroll" && e.user).toBe(true);
                expect(r.state).toBe(DETACHED);
            }
        }
    });

    it("I4: a state change emits exactly one Transition, and no change emits none", () => {
        for (const s of ALL_STATES) {
            for (const e of EVENTS) {
                const r = followReducer(s, e);
                const n = r.effects.filter((x) => x.type === "Transition").length;
                const changed = followStateLabel(r.state) !== followStateLabel(s) || intentOf(r.state) !== intentOf(s);
                expect(n, `${followStateLabel(s)} + ${e.type}`).toBe(changed ? 1 : 0);
                const t = transition(r.effects);
                if (t) {
                    expect(t.from).toEqual(s);
                    expect(t.to).toEqual(r.state);
                    expect(t.cause.length).toBeGreaterThan(0);
                }
            }
        }
    });

    it("never pins unless the result is FOLLOWING", () => {
        for (const s of ALL_STATES) {
            for (const e of EVENTS) {
                const r = followReducer(s, e);
                if (pinCause(r.effects) !== undefined) expect(r.state).toBe(FOLLOWING);
            }
        }
    });

    it("B1: a gesture long ago is not this scroll's: the binding passes user=false and nothing changes", () => {
        // The window is the binding's (follow-scroll.ts); the reducer only sees its verdict.
        const r = followReducer(FOLLOWING, scroll(BOTTOM, at(400, 1400), { user: false }));
        expect(r).toEqual({ state: FOLLOWING, effects: [] });
    });
});

describe("followStateLabel / intentOf", () => {
    it("label every state, and a suspended pane's intent is what it goes back to", () => {
        expect(ALL_STATES.map(followStateLabel)).toEqual(["following", "detached", "suspended", "suspended"]);
        expect(ALL_STATES.map(intentOf)).toEqual(["FOLLOWING", "DETACHED", "FOLLOWING", "DETACHED"]);
    });
});

describe("resolveFollowController (agent:followcontroller)", () => {
    it("an explicit setting wins; unset follows the build", () => {
        expect(resolveFollowController(true, () => false)).toBe(true);
        expect(resolveFollowController(false, () => true)).toBe(false);
        expect(resolveFollowController(undefined, () => true)).toBe(true);
        expect(resolveFollowController(undefined, () => false)).toBe(false);
        expect(resolveFollowController("yes", () => false)).toBe(false);
    });

    it("no host API means the release default", () => {
        expect(
            resolveFollowController(undefined, () => {
                throw new Error("no api");
            })
        ).toBe(false);
    });
});
