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
        const [getPhase] = atoms.turnPhaseAtom;

        expect(getDoc()).toEqual([]);
        expect(getStats()).toBeNull();
        // PR G: the working signal is `turnPhase` only — Idle by default
        // (the legacy `turnActiveAtom` was dropped).
        expect(getPhase().kind).toBe("Idle");
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
        const [getPhase1, setPhase1] = atoms.turnPhaseAtom;
        const [getPhase2] = atoms2.turnPhaseAtom;

        setPhase1({ kind: "Submitting", submittedAt: 1, pendingContent: "" });
        expect(getPhase1().kind).toBe("Submitting");
        expect(getPhase2().kind).toBe("Idle");
    });

    // ── Turn-phase view binding ─────────────────────────────────────────
    // The view's working animation and "Stopping…" label both bind to
    // `turnPhaseAtom`. Verifies the SoT is wired correctly — these are
    // the only working/stopping signals after PR G dropped the legacy
    // `turnActiveAtom` / `stoppingAtom`.
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

    test("Interrupting phase drives the 'Stopping…' label", () => {
        const [getPhase, setPhase] = atoms.turnPhaseAtom;
        // The view's `stopping` prop on AgentStatusLine reads
        // `turnPhaseAtom[0]().kind === "Interrupting"`. Verify the
        // predicate flips correctly.
        expect(getPhase().kind === "Interrupting").toBe(false);

        setPhase({ kind: "Interrupting", reason: "user", sigintSentAt: 1 });
        expect(getPhase().kind === "Interrupting").toBe(true);

        setPhase({ kind: "Done", outcome: "stopped", finishedAt: 2 });
        expect(getPhase().kind === "Interrupting").toBe(false);
    });
});
