// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Working row's clocks and counters: elapsed time, the live token count, and the compaction estimate bar. */

import { createMemo, untrack } from "solid-js";

import { turnLiveOutput, turnOpen } from "@/app/store/agent-pane-state/turn-ledger";
import { compactionEstimate, compactionProgress, readCompactionSamples, samplesForModel } from "../compaction-estimate";
import type { RowState } from "./working-row-state";

export function useRowMetrics(s: RowState) {
    const props = s.props;
    // From the turn's start while it is open, so the clock runs across its
    // passes and survives a remount; else from when this row went live.
    const elapsedMs = createMemo(() => {
        const now = (s.tick(), Date.now());
        const l = props.turnLedger;
        if (l && turnOpen(l, now)) return Math.max(0, now - l.startedAtMs);
        const started = s.loadStartMs();
        return started != null ? now - started : 0;
    });
    const liveOutput = createMemo((): number | undefined => {
        const output = turnLiveOutput(props.turnLedger, props.turnTokens, (s.tick(), Date.now()));
        if (output == null) return undefined;
        const turnId = props.turnLedger?.turnId ?? null;
        const floor = turnId != null && s.shownOutput.turnId === turnId ? s.shownOutput.output : 0;
        s.shownOutput = { turnId, output: Math.max(floor, output) };
        return s.shownOutput.output;
    });

    // Live elapsed time since compaction/reconnecting-retry started —
    // relocated from AgentComposerStrip (2026-08-27, Part 2 of
    // SPEC_REMOVE_AGENT_UNRESPONSIVE_DETECTION_2026_08_25.md). Same
    // stopwatch pattern as elapsedMs above.
    const compactingElapsedMs = createMemo(() => {
        const c = props.compacting;
        return c ? (s.tick(), Date.now() - c.startedAt) : 0;
    });
    const reconnectingElapsedMs = createMemo(() => {
        const r = props.reconnecting;
        return r ? (s.tick(), Date.now() - r.startedAt) : 0;
    });
    // Estimated compaction progress (Tier 4). Only while compacting, and never
    // while reconnecting (which takes the row over). With no earlier
    // compactions the estimate is the built-in default.
    // The estimate is keyed on `startedAt` and reads the context size untracked,
    // so it is computed once per compaction and stays put while it runs.
    const compactStartedAt = createMemo(() => props.compacting?.startedAt ?? null);
    const compactionEstimateFor = createMemo(() =>
        compactStartedAt() == null
            ? null
            : compactionEstimate(
                  samplesForModel(readCompactionSamples(), untrack(() => props.compactionModel)),
                  untrack(() => props.compactionContextTokens)
              )
    );
    const compactionBar = createMemo(() => {
        const est = compactionEstimateFor();
        if (!props.compacting || props.reconnecting || !est) return null;
        return { ...est, ...compactionProgress(compactingElapsedMs(), est.estimateMs) };
    });
    const compactionTitle = (): string | undefined => {
        const bar = compactionBar();
        if (!bar) return undefined;
        return bar.fromHistory
            ? "Estimate from your earlier compactions. Claude Code doesn't report compaction progress."
            : "Estimate: a compaction usually takes about a minute. Claude Code doesn't report compaction progress.";
    };

    return { elapsedMs, liveOutput, compactingElapsedMs, reconnectingElapsedMs, compactionBar, compactionTitle };
}

export type RowMetrics = ReturnType<typeof useRowMetrics>;
