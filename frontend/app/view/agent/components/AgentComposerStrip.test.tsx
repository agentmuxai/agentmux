// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Coverage for the Tier 3 predictive-countdown addition
 * (docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md §7) —
 * explicit "~Nk to auto-compact" text, surfaced inline once the fill level
 * reaches the mid band, escalating visually at the critical band. Not a
 * full component smoke test — AgentComposerStrip's other zones (controls,
 * turn/session stats, Shell toggle) are exercised elsewhere; this file is
 * scoped to the countdown behavior this change actually adds.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";

import { AgentComposerStrip } from "./AgentComposerStrip";

afterEach(() => {
    cleanup();
});

const baseProps = {
    logOpen: false,
    onToggleLog: () => {},
    // The countdown is Claude-only (compactionThreshold() hard-codes
    // Claude Code's own auto-compact buffer) — most tests below want it
    // enabled; the provider-gating test overrides this explicitly.
    providerId: "claude",
};

describe("AgentComposerStrip — Tier 3 predictive countdown", () => {
    it("renders no countdown text when the window is unknown", () => {
        render(() => (
            <AgentComposerStrip {...baseProps} contextTokens={10_000} contextWindow={undefined} />
        ));
        expect(screen.queryByText(/to auto-compact/)).toBeNull();
    });

    it("renders no countdown text in the low band", () => {
        // compactionThreshold(200_000) = 167_000; 10_000 / 167_000 ≈ 6% — low band.
        render(() => (
            <AgentComposerStrip {...baseProps} contextTokens={10_000} contextWindow={200_000} />
        ));
        expect(screen.queryByText(/to auto-compact/)).toBeNull();
    });

    it("renders explicit countdown text once the mid band is reached", () => {
        // threshold = 167_000; 50% of that ≈ 83_500 — mid band.
        render(() => (
            <AgentComposerStrip {...baseProps} contextTokens={90_000} contextWindow={200_000} />
        ));
        expect(screen.getByText(/~77k to auto-compact/)).toBeInTheDocument();
    });

    it("does not apply the critical escalation class in the mid band", () => {
        const { container } = render(() => (
            <AgentComposerStrip {...baseProps} contextTokens={90_000} contextWindow={200_000} />
        ));
        expect(container.querySelector(".agent-composer-strip-ctx-countdown--critical")).toBeNull();
    });

    it("applies the critical escalation class once the critical band is crossed", () => {
        // threshold = 167_000; 90% of that ≈ 150_300 — critical band.
        const { container } = render(() => (
            <AgentComposerStrip {...baseProps} contextTokens={155_000} contextWindow={200_000} />
        ));
        expect(screen.getByText(/to auto-compact/)).toBeInTheDocument();
        expect(container.querySelector(".agent-composer-strip-ctx-countdown--critical")).not.toBeNull();
    });

    it("clamps the countdown at zero once past the threshold, never going negative", () => {
        render(() => (
            <AgentComposerStrip {...baseProps} contextTokens={200_000} contextWindow={200_000} />
        ));
        expect(screen.getByText(/~0 to auto-compact/)).toBeInTheDocument();
    });

    it("the tooltip states this predicts auto-compaction only, not a manual /compact", () => {
        const { container } = render(() => (
            <AgentComposerStrip {...baseProps} contextTokens={90_000} contextWindow={200_000} />
        ));
        const countdownEl = container.querySelector(".agent-composer-strip-ctx-countdown");
        expect(countdownEl?.getAttribute("title")).toMatch(/manual \/compact can happen at any fill level/);
    });

    // Regression for Codex P2 on PR #2729: compactionThreshold() hard-codes
    // Claude Code's own ~33K auto-compact buffer. A non-Claude provider
    // that happens to report Claude-shaped message_start usage (e.g. a
    // muxcode-catalog entry) must not get an invented countdown against a
    // threshold that was never verified for it.
    it("suppresses the countdown for a non-Claude provider even in the critical band", () => {
        render(() => (
            <AgentComposerStrip
                {...baseProps}
                providerId="codex"
                contextTokens={155_000}
                contextWindow={200_000}
            />
        ));
        expect(screen.queryByText(/to auto-compact/)).toBeNull();
    });
});
