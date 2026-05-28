// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, test, expect, beforeEach } from "vitest";
import {
    createAgentAtoms,
    type AgentAtoms,
} from "./state";
import { isInterruptibleTurn, workingFromPhase } from "@/app/store/agent-pane-state/types";

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

    // ── isInterruptibleTurn — "Send now" affordance gating ──────────────
    // The pending-queue "Send now" button is meaningful only when a CLI
    // process is in flight that SIGINT can interrupt — i.e. Streaming or
    // Interrupting. `Submitting` is excluded because the message itself
    // is the would-be turn, still waiting for `agent-message-accepted`.
    // Gating on `workingFromPhase` caused a brief flash on every send.
    // Spec: docs/analysis/ANALYSIS_SEND_NOW_FLASH_2026_05_28.md.
    test("isInterruptibleTurn = false for Idle / Submitting / Done / Disconnected", () => {
        const [getPhase, setPhase] = atoms.turnPhaseAtom;

        expect(getPhase().kind).toBe("Idle");
        expect(isInterruptibleTurn(getPhase())).toBe(false);

        setPhase({ kind: "Submitting", submittedAt: 1, pendingContent: "" });
        expect(isInterruptibleTurn(getPhase())).toBe(false);

        setPhase({ kind: "Done", outcome: "completed", finishedAt: 1 });
        expect(isInterruptibleTurn(getPhase())).toBe(false);

        setPhase({
            kind: "Disconnected",
            lastKind: "Streaming",
            lastConnectedAt: 1,
            reason: "stream-unsubscribed",
        });
        expect(isInterruptibleTurn(getPhase())).toBe(false);
    });

    test("isInterruptibleTurn = true for Streaming / Interrupting", () => {
        const [getPhase, setPhase] = atoms.turnPhaseAtom;

        setPhase({
            kind: "Streaming",
            bufferSize: 0,
            toolsActive: 0,
            lastEventMs: 1,
        });
        expect(isInterruptibleTurn(getPhase())).toBe(true);

        setPhase({ kind: "Interrupting", reason: "user", sigintSentAt: 1 });
        expect(isInterruptibleTurn(getPhase())).toBe(true);
    });

    test("isInterruptibleTurn excludes the Submitting case that workingFromPhase includes", () => {
        const submitting = { kind: "Submitting", submittedAt: 1, pendingContent: "" } as const;
        // The exact divergence point: this is why the Send-now flash
        // existed and why a separate predicate was needed.
        expect(workingFromPhase(submitting)).toBe(true);
        expect(isInterruptibleTurn(submitting)).toBe(false);
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
