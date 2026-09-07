// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pins A6 of the architecture-refactor program (issue #1549): the agent
 * pane's reducer state is rendered from ONE reactive source — the
 * store-owned `AgentPaneView` — not from a view-owned mirror of hand-wired
 * signals. The acceptance criterion on the board was "adding a pane state
 * field touches one place; no `AgentAtoms ⇄ AgentPaneState` copy."
 *
 * The first block tests that property generically: it does not name any
 * field. Whatever `initialState()` declares is what the view exposes, and
 * whatever a dispatch changes is what a reactive reader sees — so a new
 * reducer field is covered here the day it is added, with no test edit.
 *
 * The second block is grep-shaped (same pattern as
 * agent-view-dispatch-via-pane-model.test.ts): the projection-setter
 * interface and the view-side mirror must not creep back.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createComputed, createRoot } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";

import { initialState } from "./agent-pane-state/types";
import {
    __resetAllSlots,
    dispatch,
    paneView,
    registerPane,
    snapshot,
} from "./agent-pane-state-store";

const BLOCK = "view-contract";

afterEach(() => {
    __resetAllSlots();
});

describe("AgentPaneView — one reactive signal per reducer field, generically", () => {
    it("exposes exactly the fields initialState() declares, with the same values", () => {
        registerPane(BLOCK, "agent-1");
        const view = paneView(BLOCK)!;
        const expected = initialState("agent-1");
        expect(Object.keys(view).sort()).toEqual(Object.keys(expected).sort());
        for (const key of Object.keys(expected) as (keyof typeof expected)[]) {
            expect(view[key]).toEqual(expected[key]);
        }
    });

    it("tracks the plain snapshot field-for-field after a run of dispatches", () => {
        registerPane(BLOCK, "agent-1");
        dispatch(BLOCK, { type: "InitReady", at: 100 });
        dispatch(BLOCK, { type: "StreamSubscribe", at: 100 });
        dispatch(BLOCK, { type: "TurnStart", at: 110 });
        dispatch(BLOCK, { type: "StreamFlushObserved", addedCount: 1, at: 120 });
        dispatch(BLOCK, { type: "DetailsExpand" });
        dispatch(BLOCK, { type: "AttachedTaskObserved", at: 500 });

        const view = paneView(BLOCK)!;
        const snap = snapshot(BLOCK)!;
        for (const key of Object.keys(snap) as (keyof typeof snap)[]) {
            // Identity, not just deep equality: the view hands out the very
            // object the reducer produced, so readers never see a stale copy.
            expect(view[key]).toBe(snap[key]);
        }
    });

    it("notifies a reactive reader only for the fields a dispatch actually changed", () => {
        registerPane(BLOCK, "agent-1");
        const view = paneView(BLOCK)!;
        const runs: Record<string, number> = {};
        const dispose = createRoot((d) => {
            for (const key of Object.keys(view) as (keyof typeof view)[]) {
                runs[key] = -1; // first run (subscription) is not a notification
                createComputed(() => {
                    view[key];
                    runs[key] += 1;
                });
            }
            return d;
        });
        try {
            // DetailsExpand touches exactly one field.
            dispatch(BLOCK, { type: "DetailsExpand" });
            const notified = Object.entries(runs).filter(([, n]) => n > 0).map(([k]) => k);
            expect(notified).toEqual(["detailsOpen"]);
            expect(view.detailsOpen).toBe(true);

            // A no-op dispatch (already expanded) notifies nobody.
            dispatch(BLOCK, { type: "DetailsExpand" });
            expect(runs.detailsOpen).toBe(1);
        } finally {
            dispose();
        }
    });

    it("keeps the last published state readable after unregister (model.state outlives the slot)", () => {
        registerPane(BLOCK, "agent-1");
        const view = paneView(BLOCK)!;
        dispatch(BLOCK, { type: "DetailsExpand" });
        __resetAllSlots();
        expect(paneView(BLOCK)).toBeNull();
        expect(view.detailsOpen).toBe(true);
    });
});

describe("the mirror must not creep back (A6 acceptance, grep-shaped)", () => {
    const store = readFileSync(join(__dirname, "agent-pane-state-store.ts"), "utf8");
    const registration = readFileSync(join(__dirname, "agent-pane-registration.ts"), "utf8");
    const viewState = readFileSync(join(__dirname, "..", "view", "agent", "state.ts"), "utf8");
    const agentView = readFileSync(join(__dirname, "..", "view", "agent", "agent-view.tsx"), "utf8");

    it("the store has no projection-setter interface", () => {
        // Declarations and typed fields, not mentions — both files explain
        // in comments what they replaced.
        expect(store).not.toMatch(/interface AgentPaneProjections|proj: AgentPaneProjections/);
        expect(registration).not.toMatch(/AgentPaneProjections,|documentSetter:|projections:/);
    });

    it("the view-side atoms hold only view-local UI state", () => {
        const atomFields = viewState.match(/^\s+(\w+Atom): SignalPair</gm) ?? [];
        expect(atomFields.map((m) => m.trim().split(":")[0])).toEqual(["documentStateAtom"]);
    });

    it("agent-view never writes a signal for a reducer-owned field", () => {
        // The only setter the view may touch is the view-local documentState.
        const writes: string[] = agentView.match(/agentAtoms\(\)\.(\w+)Atom\[1\]/g) ?? [];
        const offending = writes.filter((w) => !w.includes("documentStateAtom"));
        expect(offending).toEqual([]);
        // And it reads reducer state from the model, not from a mirror.
        expect(agentView).not.toMatch(/agentAtoms\(\)\.(?!documentState)\w+Atom\[0\]/);
    });
});
