// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { workingFromPhase, type TurnPhase } from "@/app/store/agent-pane-state/types";
import { paneBusyForInput } from "./working-indicator";

const phase = (kind: TurnPhase["kind"], extra: Record<string, unknown> = {}): TurnPhase =>
    ({ kind, ...extra }) as TurnPhase;

// Reproduces pre-§2.3a behavior: no attached background work, so the
// Streaming carve-out never engages.
const NOT_BACKGROUNDED = { hasAttachedBackgroundWork: false, hasBlockingForegroundToolCall: false };

describe("working indicator — mirrors the send gate", () => {
    // The invariant that matters: agent-view's send path decides whether to
    // queue with workingFromPhase() on the pane snapshot. If this predicate
    // ever disagrees with that for a bare phase, an indicator is lying.
    const ALL_PHASES: TurnPhase["kind"][] = [
        "Idle",
        "Submitting",
        "Streaming",
        "Interrupting",
        "Done",
        "Disconnected",
    ];

    it("agrees with workingFromPhase for every phase when not launching and not backgrounded", () => {
        for (const kind of ALL_PHASES) {
            const p = phase(kind);
            expect(
                paneBusyForInput({ showingLaunchActivity: false, turnPhase: p, compacting: null, reconnecting: null, ...NOT_BACKGROUNDED }),
            ).toBe(workingFromPhase(p));
        }
    });

    it("is busy while launching even though no turn is in flight", () => {
        // A message sent mid-launch is not answered immediately either.
        expect(
            paneBusyForInput({ showingLaunchActivity: true, turnPhase: phase("Idle"), compacting: null, reconnecting: null, ...NOT_BACKGROUNDED }),
        ).toBe(true);
    });

    it("is idle only when nothing is launching and no turn is running", () => {
        expect(
            paneBusyForInput({ showingLaunchActivity: false, turnPhase: phase("Idle"), compacting: null, reconnecting: null, ...NOT_BACKGROUNDED }),
        ).toBe(false);
        expect(
            paneBusyForInput({ showingLaunchActivity: false, turnPhase: phase("Done"), compacting: null, reconnecting: null, ...NOT_BACKGROUNDED }),
        ).toBe(false);
    });
});

describe("working indicator — Streaming with no attached background work", () => {
    it("stays busy for an ordinary Streaming turn (nothing backgrounded)", () => {
        expect(
            paneBusyForInput({
                showingLaunchActivity: false,
                compacting: null,
                reconnecting: null,
                turnPhase: phase("Streaming"),
                ...NOT_BACKGROUNDED,
            }),
        ).toBe(true);
    });

    it("stays busy for a rate-limited Streaming turn", () => {
        expect(
            paneBusyForInput({
                showingLaunchActivity: false,
                compacting: null,
                reconnecting: null,
                turnPhase: phase("Streaming", { waitingReason: "rate_limit" }),
                ...NOT_BACKGROUNDED,
            }),
        ).toBe(true);
    });

    it("stays busy while interrupting", () => {
        expect(
            paneBusyForInput({
                showingLaunchActivity: false,
                compacting: null,
                reconnecting: null,
                turnPhase: phase("Interrupting"),
                ...NOT_BACKGROUNDED,
            }),
        ).toBe(true);
    });
});

describe("working indicator — §2.3a: backgrounding releases the gate, but only when nothing else blocks", () => {
    // Policy reversal, 2026-09-17 — see
    // docs/reports/REPORT_AGENT_PANE_PROGRESS_INDICATORS_CONSOLIDATION_2026_09_09.md
    // §2.3a. The whole point of promoting work to the dock is to free the
    // pane up; these cases are what actually proves that now.

    it("is NOT busy: Streaming, backgrounded, and nothing else blocking", () => {
        expect(
            paneBusyForInput({
                showingLaunchActivity: false,
                compacting: null,
                reconnecting: null,
                turnPhase: phase("Streaming"),
                hasAttachedBackgroundWork: true,
                hasBlockingForegroundToolCall: false,
            }),
        ).toBe(false);
    });

    it("stays busy: Streaming, backgrounded, but a genuine second tool call is still running", () => {
        // e.g. a run_in_background Bash launch was accepted (freeing the
        // harness's loop) AND a second, ordinary tool call is concurrently
        // in flight — that second call still genuinely blocks the turn.
        expect(
            paneBusyForInput({
                showingLaunchActivity: false,
                compacting: null,
                reconnecting: null,
                turnPhase: phase("Streaming"),
                hasAttachedBackgroundWork: true,
                hasBlockingForegroundToolCall: true,
            }),
        ).toBe(true);
    });

    it("stays busy: Submitting with attached background work (carve-out is Streaming-only)", () => {
        // Submitting has no tool-call bookkeeping to consult yet — a leftover
        // attachedTask from a just-finished prior turn must not leak into a
        // brand-new turn's Submitting phase.
        expect(
            paneBusyForInput({
                showingLaunchActivity: false,
                compacting: null,
                reconnecting: null,
                turnPhase: phase("Submitting"),
                hasAttachedBackgroundWork: true,
                hasBlockingForegroundToolCall: false,
            }),
        ).toBe(true);
    });

    it("stays busy: Interrupting with attached background work (carve-out is Streaming-only)", () => {
        // Already mid-stop — steering a new message in here would race the
        // interrupt.
        expect(
            paneBusyForInput({
                showingLaunchActivity: false,
                compacting: null,
                reconnecting: null,
                turnPhase: phase("Interrupting"),
                hasAttachedBackgroundWork: true,
                hasBlockingForegroundToolCall: false,
            }),
        ).toBe(true);
    });
});

describe("working indicator — busy without a turn in flight", () => {
    // Divergence B from the report (§3.2). These two states can be set while
    // turnPhase is Idle. The working row read them; the progress bar and the
    // composer strip did not — so during a reconnect the row said busy and the
    // bar said idle. Both are in the predicate now, so all three agree.
    const idle = { showingLaunchActivity: false, turnPhase: phase("Idle"), ...NOT_BACKGROUNDED };

    it("is busy while reconnecting, even with no turn running", () => {
        // The least ambiguous case in the whole predicate: `reconnecting` is
        // set ONLY after the process has already crashed or exited, so there is
        // literally nothing alive to answer a message typed now.
        expect(
            paneBusyForInput({ ...idle, compacting: null, reconnecting: { attempt: 1 } }),
        ).toBe(true);
    });

    it("is busy while compacting, even with no turn running", () => {
        expect(
            paneBusyForInput({ ...idle, compacting: { trigger: "auto" }, reconnecting: null }),
        ).toBe(true);
    });

    it("treats only null as not-busy, not merely falsy", () => {
        // These are opaque state objects; a caller passing through a falsy-but-
        // present value must not read as idle.
        expect(paneBusyForInput({ ...idle, compacting: 0, reconnecting: null })).toBe(true);
        expect(paneBusyForInput({ ...idle, compacting: null, reconnecting: "" })).toBe(true);
        expect(
            paneBusyForInput({ ...idle, compacting: undefined, reconnecting: undefined }),
        ).toBe(false);
    });
});
