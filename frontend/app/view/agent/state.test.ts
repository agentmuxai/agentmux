// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, test, expect, beforeEach } from "vitest";
import {
    createAgentAtoms,
    type AgentAtoms,
} from "./state";

let atoms: AgentAtoms;

beforeEach(() => {
    atoms = createAgentAtoms("test-block-1");
});

describe("createAgentAtoms", () => {
    test("creates signals with correct default values", () => {
        const [getDoc] = atoms.documentAtom;
        const [getStats] = atoms.sessionStatsAtom;
        const [getTurnActive] = atoms.turnActiveAtom;

        expect(getDoc()).toEqual([]);
        expect(getStats()).toBeNull();
        expect(getTurnActive()).toBe(false);
    });

    test("turnActiveAtom can be toggled", () => {
        const [getTurnActive, setTurnActive] = atoms.turnActiveAtom;
        setTurnActive(true);
        expect(getTurnActive()).toBe(true);
        setTurnActive(false);
        expect(getTurnActive()).toBe(false);
    });

    test("documentStateAtom has correct default filter", () => {
        const [getState] = atoms.documentStateAtom;
        const state = getState();
        expect(state.filter.showThinking).toBe(false);
        expect(state.filter.showSuccessfulTools).toBe(true);
        expect(state.filter.showFailedTools).toBe(true);
        expect(state.filter.showIncoming).toBe(true);
        expect(state.filter.showOutgoing).toBe(true);
    });

    test("separate instances have independent state", () => {
        const atoms2 = createAgentAtoms("test-block-2");
        const [, setActive1] = atoms.turnActiveAtom;
        const [getActive1] = atoms.turnActiveAtom;
        const [getActive2] = atoms2.turnActiveAtom;

        setActive1(true);
        expect(getActive1()).toBe(true);
        expect(getActive2()).toBe(false);
    });
});
