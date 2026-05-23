// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, test, expect, beforeEach } from "vitest";
import {
    createAgentAtoms,
    type AgentAtoms,
} from "./state";
import { workingFromPhase } from "@/app/store/agent-pane-state/types";

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

    // ── PR B (turn-phase view migration) ────────────────────────────────
    // Verifies that the view's "working" animation binding can be driven
    // entirely from `turnPhaseAtom` via the `workingFromPhase` selector —
    // no read of the legacy `turnActiveAtom` / `stoppingAtom` is required.
    // The reducer still dual-writes the legacy fields (PR G drops them).
    // Spec: docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §7.
    test("turnPhaseAtom defaults to Idle and drives workingFromPhase = false", () => {
        const [getPhase] = atoms.turnPhaseAtom;
        expect(getPhase().kind).toBe("Idle");
        expect(workingFromPhase(getPhase())).toBe(false);
    });

    test("workingFromPhase(turnPhaseAtom) = true for Submitting/Streaming/Interrupting", () => {
        const [getPhase, setPhase] = atoms.turnPhaseAtom;

        setPhase({ kind: "Submitting", submittedAt: 1, pendingContent: "" });
        expect(workingFromPhase(getPhase())).toBe(true);

        setPhase({
            kind: "Streaming",
            bufferSize: 0,
            toolsActive: 0,
            lastEventMs: 1,
        });
        expect(workingFromPhase(getPhase())).toBe(true);

        setPhase({ kind: "Interrupting", reason: "user", sigintSentAt: 1 });
        expect(workingFromPhase(getPhase())).toBe(true);

        setPhase({ kind: "Done", outcome: "completed", finishedAt: 1 });
        expect(workingFromPhase(getPhase())).toBe(false);

        setPhase({
            kind: "Disconnected",
            lastKind: "Streaming",
            lastConnectedAt: 1,
            reason: "stream-unsubscribed",
        });
        expect(workingFromPhase(getPhase())).toBe(false);
    });

    test("Interrupting phase drives the 'Stopping…' label (replaces legacy stoppingAtom read)", () => {
        const [getPhase, setPhase] = atoms.turnPhaseAtom;
        // The view's `stopping` prop on AgentStatusLine reads
        // `turnPhaseAtom[0]().kind === "Interrupting"` — no read of the
        // legacy `stoppingAtom`. Verify the predicate flips correctly.
        expect(getPhase().kind === "Interrupting").toBe(false);

        setPhase({ kind: "Interrupting", reason: "user", sigintSentAt: 1 });
        expect(getPhase().kind === "Interrupting").toBe(true);

        setPhase({ kind: "Done", outcome: "stopped", finishedAt: 2 });
        expect(getPhase().kind === "Interrupting").toBe(false);
    });
});
