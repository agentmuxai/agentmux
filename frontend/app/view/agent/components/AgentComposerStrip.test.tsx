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
import userEvent from "@testing-library/user-event";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { ComposerRow } from "./AgentComposerStrip";
import { AgentComposerStrip, computeBalancedLeftKeys, computeComposerRows, computeStatsInline, orderKeysForEdgePriority } from "./AgentComposerStrip";

// AgentRuntimeDropup (rendered via showControls()) imports this — same mock
// AgentRuntimeDropup.test.tsx uses; only exercised by the reagent P1
// regression test below, which opens that dropdown.
vi.mock("../runtime-apply", () => ({
    applyRuntimeChange: vi.fn().mockResolvedValue(undefined),
}));

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

describe("AgentComposerStrip — zone-assignment identity stability (reagent P1 on PR #2808)", () => {
    it("keeps the Mode/Model/Effort dropdown open when an unrelated slot's own state changes (e.g. a tracked process count ticking)", async () => {
        // Before this fix, <For> iterated the FRESH {key,side,render}
        // objects `slots()` allocates on every recompute — any unrelated
        // prop change (processCount here) gave every slot a brand-new
        // object identity, so <For> destroyed and recreated ALL of them,
        // including AgentRuntimeDropup (which owns its own `open` signal).
        // That silently closed this exact dropdown. `<For>` now iterates
        // stable string keys (see `slotByKey`'s doc comment in
        // AgentComposerStrip.tsx) — this test would fail against the old
        // behavior (aria-expanded resets to "false" after setProcessCount).
        const [processCount, setProcessCount] = createSignal(0);
        render(() => (
            <AgentComposerStrip
                {...baseProps}
                blockId="block-1"
                blockAtom={() => undefined}
                processCount={processCount()}
            />
        ));

        await userEvent.click(screen.getByRole("button", { name: /Runtime settings/i }));
        expect(screen.getByRole("button", { name: /Runtime settings/i }).getAttribute("aria-expanded")).toBe("true");

        setProcessCount(1);
        // Proves the update actually reached the DOM (the process badge is
        // gated on processCount > 0) before trusting the assertion below —
        // otherwise a no-op re-render would make this test pass for the
        // wrong reason regardless of whether the underlying bug is fixed.
        await screen.findByText("1");

        // Deliberately RE-QUERIES rather than reusing the button reference
        // from before setProcessCount: if the fix regresses and the trigger
        // gets destroyed/recreated, the OLD (now-detached) node would still
        // report its last "aria-expanded=true" forever, making a reused
        // reference pass for the wrong reason regardless of the real bug.
        expect(screen.getByRole("button", { name: /Runtime settings/i }).getAttribute("aria-expanded")).toBe("true");
    });

    it("still updates ctx text live in place when contextTokens changes, despite the untrack() around the one-time render() call", async () => {
        // The untrack() fix above deliberately stops the OUTER slot lookup
        // from re-invoking render() on unrelated changes — this test guards
        // the other direction: it must NOT also break live reactivity
        // WITHIN a slot's own already-rendered JSX (ctx text needs to keep
        // updating every turn without the whole slot remounting).
        const [contextTokens, setContextTokens] = createSignal(90_000);
        render(() => (
            <AgentComposerStrip {...baseProps} contextTokens={contextTokens()} contextWindow={200_000} />
        ));

        expect(screen.getByText(/90k/i)).toBeInTheDocument();

        setContextTokens(120_000);
        await screen.findByText(/120k/i);

        expect(screen.queryByText(/90k/i)).not.toBeInTheDocument();
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

/**
 * Coverage for Rev 7's row-building (see this file's own Rev 7 header
 * comment, `computeComposerRows`'s own doc comment, and
 * docs/specs/SPEC_COMPOSER_STRIP_ROW_BASED_LAYOUT_2026_08_26.md) — pure,
 * same rigor as `computeBalancedLeftKeys`'s own tests above. §7.2's
 * explicit invariant test (last in this block) is the one thing missing
 * from every prior revision's test suite — see
 * docs/retro/retro-composer-strip-one-sided-lines-misdiagnosis-2026-08-26.md
 * for why that gap let the one-sided-lines bug through six revisions.
 */
describe("computeComposerRows", () => {
    it("delegates to computeBalancedLeftKeys for a single row when everything fits", () => {
        const slots = [
            { key: "a", width: 50 },
            { key: "hostShell", width: 30 },
        ];
        const rows = computeComposerRows(slots, "hostShell", 200, 5);
        expect(rows).toEqual([{ left: ["a"], right: ["hostShell"] }]);
    });

    it("builds width-paired rows (widest-with-narrowest) when the pool needs more than one line, even slot count", () => {
        const slots = [
            { key: "w1", width: 100 },
            { key: "w2", width: 80 },
            { key: "w3", width: 60 },
            { key: "hostShell", width: 50 },
        ];
        // totalWidth = 290 + 3*5 = 305, past availableWidth=200 — forces
        // the multi-row path. Both generated pairs still fit within 200
        // (w1+hostShell+gap=155, w2+w3+gap=145), so parity holds cleanly
        // — a per-pair-capacity failure is exercised by its own dedicated
        // test below instead (Codex P1, PR #2812).
        const rows = computeComposerRows(slots, "hostShell", 200, 5);
        // Sorted descending [w1,w2,w3,hostShell]; two-pointer pairs
        // (w1,hostShell) and (w2,w3); hostShell's pair reoriented to the
        // END. Every row has both sides filled — the core invariant,
        // asserted directly (spec §1), not inferred from a total-width
        // comparison the way every prior revision's own acceptance
        // criteria did.
        expect(rows).toEqual([
            { left: ["w2"], right: ["w3"] },
            { left: ["w1"], right: ["hostShell"] },
        ]);
        for (const row of rows) {
            expect(row.left.length).toBeGreaterThan(0);
            expect(row.right.length).toBeGreaterThan(0);
        }
    });

    it("leaves exactly ONE row as the allowed singleton exception when the total slot count is odd", () => {
        const slots = [
            { key: "w1", width: 100 },
            { key: "w2", width: 60 },
            { key: "hostShell", width: 50 },
        ];
        // availableWidth=180: past the single-row total (210+10=220), but
        // still enough for the w1+hostShell pair (100+50+5=155) to fit.
        const rows = computeComposerRows(slots, "hostShell", 180, 5);
        // w1 pairs with hostShell (widest+narrowest among the 3), leaving
        // w2 as the odd one out — reoriented so hostShell's row is last.
        expect(rows).toEqual([
            { left: ["w2"], right: [] },
            { left: ["w1"], right: ["hostShell"] },
        ]);
        const oneSided = rows.filter((r) => r.left.length === 0 || r.right.length === 0);
        expect(oneSided).toHaveLength(1);
    });

    it("reorients hostShell to the right side even when it sorts as the WIDER element of its pair", () => {
        // hostShell(200) is the single widest slot here, so the two-pointer
        // walk initially pairs it as the "a" (left) position with w2 (the
        // narrowest) — this test exercises the swap branch specifically,
        // not just the already-oriented case the previous two tests cover.
        const slots = [
            { key: "hostShell", width: 200 },
            { key: "w1", width: 100 },
            { key: "w2", width: 50 },
        ];
        // availableWidth=300: past the single-row total (350+10=360), but
        // still enough for the hostShell+w2 pair (200+50+5=255) to fit.
        const rows = computeComposerRows(slots, "hostShell", 300, 5);
        expect(rows).toEqual([
            { left: ["w1"], right: [] },
            { left: ["w2"], right: ["hostShell"] },
        ]);
    });

    it("degenerate case: only hostShell exists — one row, left empty, right = [hostShell]", () => {
        const rows = computeComposerRows([{ key: "hostShell", width: 50 }], "hostShell", 1000, 5);
        expect(rows).toEqual([{ left: [], right: ["hostShell"] }]);
    });

    it("returns an empty row list for an empty slot pool", () => {
        expect(computeComposerRows([], "hostShell", 1000, 5)).toEqual([]);
    });

    // Codex P1, PR #2812: the two-pointer walk used to pair widest-with-
    // narrowest unconditionally, even when the pair's combined width
    // didn't fit `availableWidth` — the returned row object had both
    // sides "filled," but the real rendered line still overflowed onto
    // two physical lines via the row's own `flex-wrap`, reproducing the
    // one-sided-lines bug through a different mechanism than the one this
    // revision set out to fix. `availableWidth=10` here is genuinely too
    // small for ANY two of these slots to share a line — no pairing can
    // satisfy both "fits" and "paired," so every slot must fall back to
    // its own one-sided row (a third, physical-capacity exception to
    // spec §1, alongside the odd-count and degenerate cases).
    it("falls back to one-sided rows when no candidate pair fits within availableWidth (Codex P1, PR #2812)", () => {
        const slots = [
            { key: "slot0", width: 500 },
            { key: "slot1", width: 10 },
            { key: "slot2", width: 10 },
            { key: "hostShell", width: 10 },
        ];
        const rows = computeComposerRows(slots, "hostShell", 10, 5);
        expect(rows).toEqual([
            { left: ["slot0"], right: [] },
            { left: ["slot1"], right: [] },
            { left: ["slot2"], right: [] },
            { left: [], right: ["hostShell"] },
        ]);
    });

    // Codex P1, PR #2812: the single-row fit check only summed slot
    // widths, never the stats zone that shares the same physical line as
    // a third flex child whenever `rows().length === 1` (spec §3.4).
    // Slots alone fitting `availableWidth` doesn't mean slots-plus-stats
    // do — `reservedWidth` closes that gap.
    it("decides row membership from slot widths alone — the stats zone can no longer force a slot split (supersedes Codex P1, PR #2812)", () => {
        // User-reported regression: with live stats ticking, the strip
        // jumped straight from 1 visual line to 3 (2 slot rows + the
        // stats' own dedicated line) — reservedWidth forcing the SLOT
        // split skipped the strictly-better middle tier of "slots on
        // one line, stats evicted to their own line below" (2 lines).
        // computeComposerRows now takes no reservedWidth; whether the
        // stats zone SHARES the single row is the component's own
        // separate `statsInline` decision.
        const slots = [
            { key: "a", width: 30 },
            { key: "b", width: 20 },
            { key: "hostShell", width: 10 },
        ];
        // Slots alone: 30+20+10 + 2*5(gap) = 70, fits within 80 —
        // single row, regardless of any stats zone.
        expect(computeComposerRows(slots, "hostShell", 80, 5)).toHaveLength(1);
    });

    // The explicit invariant test missing from every prior revision (see
    // this describe block's own doc comment): for ANY combination of
    // slot widths, across both the single-row and multi-row paths, the
    // number of one-sided rows must never exceed the mathematically-
    // justified maximum — 1 if the total slot count is odd (the one
    // named singleton exception, which also covers the n=1 degenerate
    // case, itself odd), 0 if even. This is the direct, mechanical
    // version of the requirement the user had to spell out by hand
    // ("2 lines should have 4 filled sections") — a test that checks
    // this exact property would have caught the bug this revision fixes.
    //
    // Every `availableWidth` below is chosen large enough that every
    // generated pair also satisfies per-pair capacity — the regime this
    // UI actually runs in (pane widths are hundreds of pixels; individual
    // slot widths are tens of pixels). The case where capacity genuinely
    // can't be satisfied (and this parity invariant necessarily doesn't
    // hold) has its own dedicated test above, not a generalized property.
    it.each([
        { widths: [100, 80, 60, 50], availableWidth: 200 },
        { widths: [100, 80, 60, 50], availableWidth: 1000 },
        { widths: [100, 60, 50], availableWidth: 180 },
        { widths: [10, 10, 10, 10, 10], availableWidth: 60 },
        { widths: [500, 10, 10, 10], availableWidth: 530 },
        { widths: [50], availableWidth: 10 },
    ])("no more than the mathematically-allowed one-sided rows for widths=$widths, availableWidth=$availableWidth", ({ widths, availableWidth }) => {
        const slots = widths.map((width, i) => ({ key: i === widths.length - 1 ? "hostShell" : `slot${i}`, width }));
        const rows = computeComposerRows(slots, "hostShell", availableWidth, 5);
        const oneSidedCount = rows.filter((r) => r.left.length === 0 || r.right.length === 0).length;
        const maxAllowed = slots.length % 2 === 1 ? 1 : 0;
        expect(oneSidedCount).toBeLessThanOrEqual(maxAllowed);
    });
});

/**
 * Rev 8 (2026-08-29, user-directed): the model selector (`runtime`) and
 * the Shell toggle (`hostShell`) must never travel as the pane resizes.
 * Both anchor to the BOTTOM row — nearest the composer input — with the
 * model selector on its left and Shell on its right. In the single-row
 * case there is no "bottom," so the constraint degrades to its positional
 * meaning: outermost-left and outermost-right of the one row.
 *
 * This deliberately reverses the "the model selector moving sides is
 * acceptable" call recorded in
 * docs/retro/retro-composer-strip-one-sided-lines-misdiagnosis-2026-08-26.md
 * step 3, which dismissed it once as cosmetic.
 */
describe("computeComposerRows — anchored model selector + Shell (Rev 8)", () => {
    const lastRow = (rows: ComposerRow[]) => rows[rows.length - 1];

    it("single row: the model selector is the outermost LEFT occupant, Shell the outermost RIGHT", () => {
        const slots = [
            { key: "runtime", width: 60 },
            { key: "auth", width: 40 },
            { key: "ctx", width: 40 },
            { key: "hostShell", width: 50 },
        ];
        const rows = computeComposerRows(slots, "hostShell", 500, 5, "runtime");
        expect(rows).toHaveLength(1);
        expect(rows[0].left[0]).toBe("runtime");
        expect(rows[0].right[rows[0].right.length - 1]).toBe("hostShell");
    });

    it("single row: the model selector stays LEFT even when pure width-balance would move it right", () => {
        // Widths chosen so the UNANCHORED balancer genuinely prefers
        // runtime on the right: one wide passive slot alone on the left
        // (100) balances runtime+hostShell (10+10) better than any split
        // that keeps runtime left. That's exactly the "model selector
        // travelled sides" regression the retro dismissed as cosmetic and
        // this constraint now forbids.
        const slots = [
            { key: "runtime", width: 10 },
            { key: "wide", width: 100 },
            { key: "hostShell", width: 10 },
        ];
        const unanchored = computeComposerRows(slots, "hostShell", 500, 5);
        expect(unanchored[0].left).not.toContain("runtime");
        expect(unanchored[0].right).toContain("runtime");

        const anchored = computeComposerRows(slots, "hostShell", 500, 5, "runtime");
        expect(anchored[0].left[0]).toBe("runtime");
        expect(anchored[0].right.at(-1)).toBe("hostShell");
    });

    it("multi-row: the anchors are reserved as the LAST row, model selector left / Shell right", () => {
        const slots = [
            { key: "runtime", width: 60 },
            { key: "w1", width: 100 },
            { key: "w2", width: 80 },
            { key: "w3", width: 40 },
            { key: "hostShell", width: 50 },
        ];
        const rows = computeComposerRows(slots, "hostShell", 200, 5, "runtime");
        expect(lastRow(rows)).toEqual({ left: ["runtime"], right: ["hostShell"] });
    });

    it("multi-row: neither anchor ever appears in any row but the last", () => {
        const slots = [
            { key: "runtime", width: 60 },
            { key: "w1", width: 100 },
            { key: "w2", width: 80 },
            { key: "w3", width: 40 },
            { key: "hostShell", width: 50 },
        ];
        const rows = computeComposerRows(slots, "hostShell", 200, 5, "runtime");
        for (const row of rows.slice(0, -1)) {
            expect([...row.left, ...row.right]).not.toContain("runtime");
            expect([...row.left, ...row.right]).not.toContain("hostShell");
        }
    });

    it("reserving exactly two anchors preserves parity — the singleton budget is unchanged", () => {
        // 5 slots (odd) → still at most ONE one-sided row, even though 2
        // of them are now reserved out of the pairing pool. Reserving a
        // PAIR can't flip the remainder's parity; that's why the anchor
        // constraint doesn't weaken spec §1.
        const slots = [
            { key: "runtime", width: 60 },
            { key: "w1", width: 100 },
            { key: "w2", width: 80 },
            { key: "w3", width: 40 },
            { key: "hostShell", width: 50 },
        ];
        const rows = computeComposerRows(slots, "hostShell", 200, 5, "runtime");
        const oneSided = rows.filter((r) => r.left.length === 0 || r.right.length === 0);
        expect(oneSided.length).toBeLessThanOrEqual(1);
    });

    it("keeps the anchors on ONE row even when they cannot fit side by side (reagent P1, PR #2839)", () => {
        // runtime(120) + hostShell(120) + gap(5) = 245 > availableWidth
        // 200. An earlier revision split these into two adjacent one-sided
        // rows via the generic physical-capacity exception — but only the
        // LAST row is hoisted out of the render's <For>, so that put
        // `runtime` back inside it and destroyed AgentRuntimeDropup on any
        // resize across the split boundary. The row's own flex-wrap
        // renders the same two visual lines without moving either anchor
        // between DOM subtrees, so the row must stay singular here.
        const slots = [
            { key: "runtime", width: 120 },
            { key: "w1", width: 100 },
            { key: "w2", width: 90 },
            { key: "hostShell", width: 120 },
        ];
        const rows = computeComposerRows(slots, "hostShell", 200, 5, "runtime");
        expect(rows[rows.length - 1]).toEqual({ left: ["runtime"], right: ["hostShell"] });
    });

    it("the anchors share exactly one row at EVERY width, so neither can change rows on resize", () => {
        // The property the P1 above is really about: sweep a wide range of
        // available widths and assert the anchors are always together on
        // the final row. If that holds everywhere, no resize can ever move
        // them between the hoisted element and a <For> index.
        const slots = [
            { key: "runtime", width: 120 },
            { key: "w1", width: 100 },
            { key: "w2", width: 90 },
            { key: "auth", width: 40 },
            { key: "hostShell", width: 120 },
        ];
        for (const availableWidth of [1000, 700, 500, 400, 300, 240, 200, 150, 100, 50]) {
            const rows = computeComposerRows(slots, "hostShell", availableWidth, 5, "runtime");
            const last = rows[rows.length - 1];
            expect(last.left).toContain("runtime");
            expect(last.right).toContain("hostShell");
            for (const r of rows.slice(0, -1)) {
                expect([...r.left, ...r.right]).not.toContain("runtime");
                expect([...r.left, ...r.right]).not.toContain("hostShell");
            }
        }
    });

    it("falls back to the pre-anchor behavior when the model selector is absent (controls hidden)", () => {
        const slots = [
            { key: "w1", width: 100 },
            { key: "w2", width: 80 },
            { key: "w3", width: 60 },
            { key: "hostShell", width: 50 },
        ];
        // Same pool, same widths, with and without an anchorLeftKey that
        // matches nothing — identical output, so hiding the runtime slot
        // can never change how the remaining slots lay out.
        expect(computeComposerRows(slots, "hostShell", 200, 5, "runtime")).toEqual(
            computeComposerRows(slots, "hostShell", 200, 5),
        );
    });

    it("keeps Shell rightmost on the last row across a wide→narrow→wide round-trip", () => {
        // Round-trip, not a one-way sweep — the exact verification shape
        // PR #2814's one-way-trap regression proved is necessary here.
        const slots = [
            { key: "runtime", width: 60 },
            { key: "w1", width: 100 },
            { key: "w2", width: 80 },
            { key: "hostShell", width: 50 },
        ];
        const wide = computeComposerRows(slots, "hostShell", 500, 5, "runtime");
        const narrow = computeComposerRows(slots, "hostShell", 200, 5, "runtime");
        const wideAgain = computeComposerRows(slots, "hostShell", 500, 5, "runtime");

        expect(wide).toEqual(wideAgain);
        expect(lastRow(wide).right.at(-1)).toBe("hostShell");
        expect(lastRow(narrow).right.at(-1)).toBe("hostShell");
        expect(lastRow(wide).left[0]).toBe("runtime");
        expect(lastRow(narrow).left[0]).toBe("runtime");
    });
});

/**
 * Edge priority for interactive elements (2026-08-26, user-directed
 * follow-up to Rev 7): on every rendered line, interactive elements
 * (buttons/dropdowns) sit flush against the strip's outer edges, with
 * passive/informational content (auth status, ctx text) placed inward.
 * Two mechanisms, both covered here: side-level ordering of whole slots
 * (`orderKeysForEdgePriority`), and side-dependent internal ordering of
 * the composite ctx/hostShell slots (Compact/Shell on the outer end).
 */
describe("orderKeysForEdgePriority", () => {
    const isInteractive = (interactive: string[]) => (key: string) => interactive.includes(key);

    it("moves interactive slots to the FRONT on the left side (outermost = first)", () => {
        expect(orderKeysForEdgePriority(["auth", "ctx"], "left", isInteractive(["ctx"]))).toEqual(["ctx", "auth"]);
    });

    it("moves interactive slots to the BACK on the right side (outermost = last)", () => {
        expect(orderKeysForEdgePriority(["badge", "auth", "hostShell"], "right", isInteractive(["badge", "hostShell"]))).toEqual([
            "auth",
            "badge",
            "hostShell",
        ]);
    });

    it("is a stable partition — relative order within each group is preserved", () => {
        expect(orderKeysForEdgePriority(["a", "b", "c", "d"], "left", isInteractive(["b", "d"]))).toEqual(["b", "d", "a", "c"]);
        expect(orderKeysForEdgePriority(["a", "b", "c", "d"], "right", isInteractive(["a", "c"]))).toEqual(["b", "d", "a", "c"]);
    });

    it("leaves uniform groups untouched on either side", () => {
        expect(orderKeysForEdgePriority(["a", "b"], "left", () => false)).toEqual(["a", "b"]);
        expect(orderKeysForEdgePriority(["a", "b"], "right", () => true)).toEqual(["a", "b"]);
        expect(orderKeysForEdgePriority([], "left", () => true)).toEqual([]);
    });
});

describe("AgentComposerStrip — interactive elements flush against the row edge", () => {
    // b must FOLLOW a in document order.
    const precedes = (a: Element, b: Element) => (a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;

    it("renders the Compact button BEFORE the ctx text when the ctx slot sits on the left side", () => {
        const { container } = render(() => (
            <AgentComposerStrip {...baseProps} onCompact={() => {}} contextTokens={40_000} contextWindow={200_000} />
        ));
        const compact = screen.getByRole("button", { name: "Compact" });
        const ctxText = container.querySelector(".agent-composer-strip-ctx")!;
        expect(compact.closest(".agent-composer-strip-row-left")).not.toBeNull();
        expect(precedes(compact, ctxText)).toBe(true);
    });

    it("orders a passive slot INSIDE an interactive one on the right side (auth inward of the process badge)", () => {
        // contextTokens populates the ctx slot on the LEFT — without any
        // left-side slot, the "left must never be completely empty"
        // fallback would promote the badge (first right slot in pool
        // order) to the left side, and this test would be asserting
        // against the wrong row entirely.
        const { container } = render(() => (
            <AgentComposerStrip {...baseProps} authStatus="authenticated" processCount={2} contextTokens={40_000} contextWindow={200_000} />
        ));
        const auth = container.querySelector(".agent-composer-strip-auth")!;
        const badge = container.querySelector(".agent-composer-strip-process-badge")!;
        expect(auth.closest(".agent-composer-strip-row-right")).not.toBeNull();
        expect(precedes(auth, badge)).toBe(true);
    });

    it("keeps Shell the outermost element of the right edge", () => {
        const { container } = render(() => (
            <AgentComposerStrip {...baseProps} authStatus="authenticated" processCount={2} agentMode="host" />
        ));
        const shell = screen.getByRole("button", { name: /Shell/i });
        const badge = container.querySelector(".agent-composer-strip-process-badge")!;
        expect(precedes(badge, shell)).toBe(true);
    });

    it("mirrors the hostShell slot's internal order when it lands on the LEFT (degenerate one-slot case): Shell outermost-left, HOST badge inward", () => {
        // Minimal pool: no controls (no blockAtom), no auth, no badge, no
        // ctx — hostShell alone falls back to the left side (the "left
        // must never be completely empty" fallback), putting it on the
        // line's LEFT edge, where Shell must flip to the outer (first)
        // position.
        const { container } = render(() => (
            <AgentComposerStrip logOpen={false} onToggleLog={() => {}} agentMode="host" />
        ));
        const shell = screen.getByRole("button", { name: /Shell/i });
        const hostBadge = container.querySelector(".agent-composer-strip .runtime-badge")!;
        expect(shell.closest(".agent-composer-strip-row-left")).not.toBeNull();
        expect(precedes(shell, hostBadge)).toBe(true);
    });
});

/**
 * The stats-placement decision (ReAgent P2, PR #2817) — extracted as a
 * pure function precisely so this guard exists: this exact stats-width
 * math has regressed twice before (Codex P1 #2812's overflow, the
 * post-#2813 wrapper trap) with no automated coverage.
 */
describe("computeStatsInline", () => {
    it("keeps stats inline when slots-plus-stats-plus-gap fit the single row", () => {
        // slots 70 + stats 20 + gap 5 = 95 <= 100
        expect(computeStatsInline(1, 70, 20, 5, 100)).toBe(true);
    });

    it("evicts stats to their own line — WITHOUT splitting slots — when they no longer fit beside them (the 2-line middle tier)", () => {
        // slots 70 + stats 20 + gap 5 = 95 > 80, but the row count stays
        // 1: the caller's slot rows are untouched, only placement flips.
        expect(computeStatsInline(1, 70, 20, 5, 80)).toBe(false);
    });

    it("empty stats are always inline — an empty zone must never claim a line of its own", () => {
        // Regression shape of the post-#2813 wrapper trap: a zero-width
        // stats measurement must never flip placement, at ANY width.
        expect(computeStatsInline(1, 70, 0, 5, 80)).toBe(true);
        expect(computeStatsInline(1, 9999, 0, 5, 10)).toBe(true);
    });

    it("never inline once the slots themselves need multiple rows", () => {
        expect(computeStatsInline(2, 70, 20, 5, 1000)).toBe(false);
        expect(computeStatsInline(3, 70, 0, 5, 1000)).toBe(false);
    });

    it("boundary: exactly-fitting stats stay inline", () => {
        expect(computeStatsInline(1, 70, 25, 5, 100)).toBe(true);
        expect(computeStatsInline(1, 70, 26, 5, 100)).toBe(false);
    });
});
