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

import { AgentComposerStrip, computeBalancedLeftKeys } from "./AgentComposerStrip";

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

/**
 * Coverage for Rev 6's real-width zone-balancing search (see this file's
 * own Rev 6 header comment and
 * docs/specs/SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md) — pure,
 * so these run without needing a real layout engine to produce widths
 * (unlike the component itself, which falls back to the fixed `side`
 * pairing under JSDOM's always-zero widths — see AgentComposerStrip.tsx's
 * `zones` memo).
 */
describe("computeBalancedLeftKeys", () => {
    it("returns an empty set when there are no movable slots", () => {
        expect(computeBalancedLeftKeys([], 90)).toEqual(new Set());
    });

    it("puts the sole movable slot on the left rather than leaving it empty", () => {
        // Every other subset is either empty (skipped — the caller's own
        // "never a dead zone" override handles the resulting all-right
        // case) or this one; nothing else to compare against.
        const result = computeBalancedLeftKeys([{ key: "a", width: 10 }], 100);
        expect(result).toEqual(new Set(["a"]));
    });

    it("picks the smaller-diff single-item split over grouping both together, first-found wins a tie", () => {
        // a=100, b=10, fixedRight=0. {a} alone: diff=|100-10|=90. {b}
        // alone: diff=|10-100|=90 (a tie with {a} alone). {a,b} together:
        // diff=|110-0|=110 (worse). Ties resolve to whichever subset the
        // brute force reaches first (ascending bitmask order) — pinning
        // that here makes the tie-break behavior explicit and regression-
        // tested, not incidental.
        const result = computeBalancedLeftKeys(
            [
                { key: "a", width: 100 },
                { key: "b", width: 10 },
            ],
            0,
        );
        expect(result).toEqual(new Set(["a"]));
    });

    it("finds the true minimum-diff split across more than 2 movable slots, even when it splits an otherwise-plausible pairing", () => {
        // Modeling the real reported shape (runtime trigger + ctx group +
        // auth tag, hostShell fixed right) with representative widths.
        // runtime=130, ctx=170, auth=55, hostShell(fixed right)=90.
        // Every 2-way split of {runtime, ctx, auth}:
        //   {runtime}       -> left=130, right=90+225=315, diff=185
        //   {ctx}           -> left=170, right=90+185=275, diff=105
        //   {runtime,ctx}   -> left=300, right=90+55 =145, diff=155
        //   {auth}          -> left=55,  right=90+300=390, diff=335
        //   {runtime,auth}  -> left=185, right=90+170=260, diff=75
        //   {ctx,auth}      -> left=225, right=90+130=220, diff=5   <- best
        //   {runtime,ctx,auth} -> left=355, right=90,       diff=265
        // The true optimum (ctx+auth) is NOT the "runtime+ctx together"
        // pairing Rev 5's fixed semantic grouping used — real widths, not
        // a semantic label, decide the split, which is the entire point
        // of this revision.
        const result = computeBalancedLeftKeys(
            [
                { key: "runtime", width: 130 },
                { key: "ctx", width: 170 },
                { key: "auth", width: 55 },
            ],
            90,
        );
        expect(result).toEqual(new Set(["ctx", "auth"]));
    });
});
