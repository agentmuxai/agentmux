// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, test, expect, beforeEach } from "vitest";
import {
    createAgentAtoms,
    type AgentAtoms,
} from "./state";

let atoms: AgentAtoms;

beforeEach(() => {
    atoms = createAgentAtoms();
});

// A6 of issue #1549: `createAgentAtoms` used to build a 19-signal mirror of
// the two agent-pane stores (turnPhase, document, failure, …). Those are
// read from `model.state` / `model.document` now, and the reactive-view
// contract is tested where it lives (agent-pane-state-store.test.ts,
// agent-pane-registration.test.ts, agent-pane-model.test.ts). What is left
// here is the genuinely view-local document UI state.
describe("createAgentAtoms (view-local state only)", () => {
    test("documentStateAtom has correct default filter", () => {
        const [getState] = atoms.documentStateAtom;
        const state = getState();
        expect(state.filter.showThinking).toBe(false);
        expect(state.filter.showSuccessfulTools).toBe(true);
        expect(state.filter.showFailedTools).toBe(true);
        expect(state.filter.showIncoming).toBe(true);
        expect(state.filter.showOutgoing).toBe(true);
    });

    test("documentStateAtom starts with empty collapse/pin/expanded sets and no selection", () => {
        const [getState] = atoms.documentStateAtom;
        const state = getState();
        expect(state.collapsedNodes.size).toBe(0);
        expect(state.pinnedNodes.size).toBe(0);
        expect(state.expandedTools.size).toBe(0);
        expect(state.scrollPosition).toBe(0);
        expect(state.selectedNode).toBeNull();
    });

    test("separate instances have independent state", () => {
        const atoms2 = createAgentAtoms();
        const [getState1, setState1] = atoms.documentStateAtom;
        const [getState2] = atoms2.documentStateAtom;

        setState1((prev) => ({ ...prev, scrollPosition: 42 }));
        expect(getState1().scrollPosition).toBe(42);
        expect(getState2().scrollPosition).toBe(0);
    });

    test("exposes no reducer-owned fields — those live on the pane model", () => {
        // Guard against the mirror creeping back: the only key is the
        // view-local document UI state.
        expect(Object.keys(atoms)).toEqual(["documentStateAtom"]);
    });
});
