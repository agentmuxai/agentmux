// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    DEADLINES_MS,
    IDLE,
    liveIntent,
    transition,
    type Intent,
    type SelectionEffect,
    type SelectionEvent,
    type SelectionState,
    type SelectionStateKind,
} from "./selection-machine";

const intent = (id: number, blockId = "blk"): Intent => ({ id, blockId, source: "swarm", focusCaret: true, localOnly: false, quiet: false });

/** Feed events in order; the state after, and every effect they produced. */
function run(events: SelectionEvent[], from: SelectionState = IDLE) {
    let state = from;
    const effects: SelectionEffect[] = [];
    for (const e of events) {
        const t = transition(state, e);
        state = t.state;
        effects.push(...t.effects);
    }
    return { state, effects };
}

const types = (effects: SelectionEffect[]) => effects.map((e) => e.type);
const finished = (effects: SelectionEffect[]) =>
    effects.flatMap((e) => (e.type === "finish" ? [{ id: e.intentId, result: e.result }] : []));

const select = (id: number, blockId?: string): SelectionEvent => ({ type: "select", intent: intent(id, blockId) });
const here = (id: number, active = false): SelectionEvent => ({ type: "resolved", intentId: id, where: "here", tabId: "t1", active });
const ev = <T extends SelectionEvent["type"]>(type: T, intentId: number): SelectionEvent =>
    ({ type, intentId }) as unknown as SelectionEvent;
const verified = (id: number, ok: boolean): SelectionEvent =>
    ok ? { type: "verified", intentId: id, ok: true } : { type: "verified", intentId: id, ok: false, detail: "the pane is not the focused one" };

/** The events that carry intent `id` up to `verifying` on window tab t1 (not the active one). */
const toVerifying = (id: number): SelectionEvent[] => [
    select(id),
    here(id),
    ev("tabSwitched", id),
    ev("layoutReady", id),
    ev("shown", id),
    ev("focused", id),
];

describe("the selection machine", () => {
    it("starts resolving on a select, with its deadline armed", () => {
        const { state, effects } = run([select(1)]);
        expect(state.kind).toBe("resolving");
        expect(effects).toEqual([
            { type: "resolve", intent: intent(1) },
            { type: "arm", intentId: 1, step: 1, ms: DEADLINES_MS.resolving },
        ]);
    });

    it("goes tab, layout, show, focus, verify, settled for a block in a tab that is not active, and flashes", () => {
        const kinds: SelectionStateKind[] = [];
        let state: SelectionState = IDLE;
        const all: SelectionEffect[] = [];
        for (const e of [...toVerifying(1), verified(1, true)]) {
            const t = transition(state, e);
            state = t.state;
            all.push(...t.effects);
            kinds.push(state.kind);
        }
        expect(kinds).toEqual(["resolving", "switchingTab", "awaitingLayout", "showing", "focusing", "verifying", "settled"]);
        expect(types(all).filter((t) => t !== "arm")).toEqual([
            "resolve",
            "switchTab",
            "awaitLayout",
            "show",
            "focus",
            "verify",
            "flash",
            "finish",
        ]);
        expect(finished(all)).toEqual([{ id: 1, result: { ok: true } }]);
    });

    it("skips the tab switch when the holder tab is the active one", () => {
        const { state, effects } = run([select(1), here(1, true)]);
        expect(state.kind).toBe("awaitingLayout");
        expect(types(effects)).not.toContain("switchTab");
    });

    it("hands a block that is not in this window to srv, and settles on its answer without a flash of its own", () => {
        const found = run([select(1), { type: "resolved", intentId: 1, where: "other" }, { type: "remoteDone", intentId: 1, outcome: "revealed" }]);
        expect(found.state.kind).toBe("settled");
        expect(types(found.effects)).toContain("handoff");
        expect(types(found.effects)).not.toContain("flash");
        expect(finished(found.effects)).toEqual([{ id: 1, result: { ok: true } }]);
        const missing = run([select(1), { type: "resolved", intentId: 1, where: "other" }, { type: "remoteDone", intentId: 1, outcome: "not-found" }]);
        expect(missing.state).toMatchObject({ kind: "failed", reason: "not-found" });
        const handed = [select(1), { type: "resolved", intentId: 1, where: "other" } as SelectionEvent];
        expect(run([...handed, { type: "remoteDone", intentId: 1, outcome: "timeout" }]).state).toMatchObject({ kind: "failed", reason: "remote-timeout" });
        expect(run([...handed, { type: "remoteDone", intentId: 1, outcome: "failed", reason: "mismatch" }]).state).toMatchObject({
            kind: "failed",
            reason: "remote-failed",
            detail: "mismatch",
        });
    });

    it("fails with a notice when nothing holds the block", () => {
        const { state, effects } = run([select(1), { type: "resolved", intentId: 1, where: "none" }]);
        expect(state).toMatchObject({ kind: "failed", reason: "not-found" });
        expect(types(effects)).toEqual(expect.arrayContaining(["notify", "finish"]));
        expect(finished(effects)).toEqual([{ id: 1, result: { ok: false, reason: "not-found", detail: undefined } }]);
    });

    describe("the newest request wins", () => {
        const steps: [string, SelectionEvent[]][] = [
            ["resolving", [select(1)]],
            ["switchingTab", [select(1), here(1)]],
            ["awaitingLayout", [select(1), here(1, true)]],
            ["showing", [select(1), here(1, true), ev("layoutReady", 1)]],
            ["focusing", [select(1), here(1, true), ev("layoutReady", 1), ev("shown", 1)]],
            ["verifying", toVerifying(1)],
            ["handoff", [select(1), { type: "resolved", intentId: 1, where: "other" }]],
        ];
        for (const [kind, events] of steps) {
            it(`a select during ${kind} ends the first as superseded and starts the second`, () => {
                const before = run(events);
                expect(before.state.kind).toBe(kind);
                const t = transition(before.state, select(2, "other"));
                expect(t.state).toMatchObject({ kind: "resolving", intent: { id: 2, blockId: "other" } });
                expect(finished(t.effects)).toEqual([{ id: 1, result: { ok: false, reason: "superseded" } }]);
                expect(types(t.effects)).toContain("resolve");
            });
        }

        it("what an older request finds after a newer one started is dropped, whatever order it arrives in", () => {
            const { state } = run([select(1), select(2), here(1), ev("layoutReady", 1), ev("shown", 1), { type: "resolved", intentId: 2, where: "none" }]);
            expect(state).toMatchObject({ kind: "failed", reason: "not-found", intent: { id: 2 } });
        });

        it("selecting the same target again restarts it as a new request", () => {
            const { state, effects } = run([select(1), here(1, true), select(2)]);
            expect(state).toMatchObject({ kind: "resolving", intent: { id: 2 } });
            expect(finished(effects)).toEqual([{ id: 1, result: { ok: false, reason: "superseded" } }]);
        });
    });

    describe("deadlines", () => {
        const cases: [string, SelectionEvent[], string][] = [
            ["resolving", [select(1)], "resolve-timeout"],
            ["switchingTab", [select(1), here(1)], "tab-timeout"],
            ["awaitingLayout", [select(1), here(1, true)], "layout-timeout"],
            ["showing", [select(1), here(1, true), ev("layoutReady", 1)], "stuck"],
            ["focusing", [select(1), here(1, true), ev("layoutReady", 1), ev("shown", 1)], "stuck"],
            ["handoff", [select(1), { type: "resolved", intentId: 1, where: "other" }], "remote-timeout"],
        ];
        for (const [kind, events, reason] of cases) {
            it(`${kind} past its deadline fails with ${reason}, and says so`, () => {
                const { state } = run(events);
                const t = transition(state, { type: "deadline", intentId: 1, step: state.step });
                expect(t.state).toMatchObject({ kind: "failed", reason });
                expect(types(t.effects)).toEqual(expect.arrayContaining(["notify", "finish"]));
            });
        }

        it("a deadline armed for an earlier state does not end a later one", () => {
            const first = run([select(1)]);
            const later = run([here(1, true)], first.state);
            expect(later.state.kind).toBe("awaitingLayout");
            const t = transition(later.state, { type: "deadline", intentId: 1, step: first.state.step });
            expect(t.state).toBe(later.state);
            expect(t.effects).toEqual([]);
        });

        it("a deadline of a replaced request is dropped", () => {
            const first = run([select(1)]);
            const second = run([select(2)], first.state);
            const t = transition(second.state, { type: "deadline", intentId: 1, step: first.state.step });
            expect(t.state).toBe(second.state);
        });

        it("every state with a deadline has one", () => {
            expect(Object.keys(DEADLINES_MS).sort()).toEqual(
                ["awaitingLayout", "focusing", "handoff", "resolving", "showing", "switchingTab", "verifying"].sort()
            );
        });
    });

    describe("reading it back", () => {
        it("a mismatch shows it once more, and a second mismatch fails with what disagreed", () => {
            const first = run([...toVerifying(1), verified(1, false)]);
            expect(first.state).toMatchObject({ kind: "showing", retried: true });
            expect(types(first.effects)).toContain("show");
            const second = run([ev("shown", 1), ev("focused", 1), verified(1, false)], first.state);
            expect(second.state).toMatchObject({
                kind: "failed",
                reason: "mismatch",
                detail: "the pane is not the focused one",
            });
            expect(types(second.effects)).toContain("notify");
        });

        it("a retry that verifies settles, and flashes", () => {
            const { state, effects } = run([...toVerifying(1), verified(1, false), ev("shown", 1), ev("focused", 1), verified(1, true)]);
            expect(state.kind).toBe("settled");
            expect(types(effects)).toContain("flash");
        });

        it("a verification that never finishes counts as a mismatch, once", () => {
            const verifying = run(toVerifying(1)).state;
            const retried = transition(verifying, { type: "deadline", intentId: 1, step: verifying.step });
            expect(retried.state).toMatchObject({ kind: "showing", retried: true });
        });

        it("a block that left its layout fails as gone", () => {
            const { state } = run([...toVerifying(1), ev("blockGone", 1)]);
            expect(state).toMatchObject({ kind: "failed", reason: "gone" });
        });
    });

    describe("the user and failures", () => {
        it("the user selecting something else ends the request quietly, with no notice and no retry", () => {
            const t = transition(run(toVerifying(1)).state, { type: "userMoved" });
            expect(t.state.kind).toBe("idle");
            expect(finished(t.effects)).toEqual([{ id: 1, result: { ok: false, reason: "superseded" } }]);
            expect(types(t.effects)).not.toContain("notify");
        });

        it("a user move with nothing in flight, or after it ended, changes nothing", () => {
            expect(transition(IDLE, { type: "userMoved" }).effects).toEqual([]);
            const settled = run([...toVerifying(1), verified(1, true)]).state;
            expect(transition(settled, { type: "userMoved" }).state).toBe(settled);
        });

        it("an effect that throws fails the request as an error", () => {
            const { state } = run([select(1), { type: "effectFailed", intentId: 1, detail: "boom" }]);
            expect(state).toMatchObject({ kind: "failed", reason: "error", detail: "boom" });
        });

        it("events after the request ended, or out of order for its state, are ignored", () => {
            const failed = run([select(1), { type: "resolved", intentId: 1, where: "none" }]).state;
            expect(transition(failed, ev("layoutReady", 1)).state).toBe(failed);
            const resolving = run([select(1)]).state;
            for (const e of [ev("layoutReady", 1), ev("shown", 1), ev("focused", 1), ev("tabSwitched", 1), verified(1, true)]) {
                expect(transition(resolving, e).state).toBe(resolving);
            }
        });

        it("liveIntent is the request while it runs and none once it ended", () => {
            expect(liveIntent(IDLE)).toBeNull();
            expect(liveIntent(run([select(1)]).state)?.id).toBe(1);
            expect(liveIntent(run([select(1), { type: "resolved", intentId: 1, where: "none" }]).state)).toBeNull();
        });
    });
});
