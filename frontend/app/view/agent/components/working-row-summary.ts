// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What the Working row says once a turn ends (the worked line), and its right-hand readout while it runs. */

import { createMemo } from "solid-js";

import { turnEndedAt } from "@/app/store/agent-pane-state/turn-ledger";
import { formatElapsedCompact } from "@/util/format-time";
import { absorbedSummary, workedVerb } from "../turn-trigger-text";
import { fmtOutputTokens, fmtTurnTokens, ingToEd } from "./status-formatters";
import type { RowMetrics } from "./working-row-metrics";
import { CANCELLABLE_LAUNCH_PHASES } from "./working-row-model";
import type { RowState } from "./working-row-state";

export function useRowSummary(s: RowState, m: RowMetrics) {
    const props = s.props;
    // The turn that just ended, when srv reported it: its figures cover every
    // pass, where `sessionStats` is the last pass's alone.
    const endedTurn = createMemo(() => {
        const l = props.turnLedger;
        if (!l) return null;
        const endedAt = turnEndedAt(l, (s.tick(), Date.now()));
        return endedAt != null ? { l, durationMs: endedAt - l.startedAtMs } : null;
    });

    // The whole turn's figures: srv's, when it counted the passes; else the
    // pane's own sum of each pass's stats, for the same turn.
    const turnFigures = createMemo((): { outputTokens: number; costUsd: number; steps: number } | null => {
        const turn = endedTurn();
        if (!turn) return null;
        if (turn.l.countedPasses > 0) return turn.l;
        const carry = props.turnCarry;
        return carry && carry.turnId === turn.l.turnId && carry.passes > 1 ? carry : null;
    });

    const workedSummary = createMemo((): string | null => {
        const stats = props.sessionStats;
        if (!stats) return null;
        const turn = endedTurn();
        // Named for what started it when that wasn't the user.
        const parts: string[] = ["✓ " + (turn ? workedVerb(turn.l.trigger) : ingToEd(s.lastPhrase()))];
        const durationMs = turn?.durationMs ?? stats.duration_ms;
        if (durationMs != null) parts.push(formatElapsedCompact(Math.max(1_000, durationMs)));
        // The turn's output: the results' exact figures (summed over its
        // calls and passes), where the live row showed it growing.
        const figures = turnFigures();
        const output = figures ? figures.outputTokens : stats.output_tokens;
        if (output != null) parts.push(fmtOutputTokens(output));
        return parts.join("  ·  ");
    });

    const workedSecondary = createMemo((): string | null => {
        const stats = props.sessionStats;
        if (!stats) return null;
        const turn = endedTurn();
        const counted = turnFigures();
        const parts: string[] = [];
        // A provider that reports no cost (Codex) counts 0: show its own, if any.
        const cost = counted && counted.costUsd > 0 ? counted.costUsd : stats.cost_usd;
        if (cost != null) parts.push(`$${cost.toFixed(3)}`);
        // Model calls, which is what `result.num_turns` counts: steps, not turns.
        const steps = counted ? counted.steps : stats.num_turns;
        if (steps) parts.push(`${steps} ${steps === 1 ? "step" : "steps"}`);
        if (turn && turn.l.passes > 1) parts.push(`${turn.l.passes} passes`);
        const joined = turn ? absorbedSummary(turn.l.absorbed, turn.l.inputs) : null;
        if (joined) parts.push(joined);
        return parts.length ? parts.join("  ·  ") : null;
    });

    const rightText = createMemo((): string => {
        // reconnecting/compacting show only the elapsed-time readout, no
        // token counts — matches the old composer-strip behavior these
        // states were relocated from. `reconnecting` takes priority (see
        // its prop doc comment).
        if (props.reconnecting) return formatElapsedCompact(m.reconnectingElapsedMs());
        if (props.compacting) {
            const elapsed = formatElapsedCompact(m.compactingElapsedMs());
            const bar = m.compactionBar();
            if (!bar) return elapsed;
            return bar.over
                ? `${elapsed} · longer than usual`
                : `${elapsed} / ~${formatElapsedCompact(bar.estimateMs)}`;
        }
        const right: string[] = [];
        const output = m.liveOutput();
        if (output != null) right.push(fmtTurnTokens(output, props.loading ? props.turnTokens : null));
        right.push(formatElapsedCompact(m.elapsedMs()));
        return right.join("  ·  ");
    });

    const showCancelLogin = createMemo(
        () =>
            !!props.onCancelLogin &&
            !props.hasAuthUrl &&
            !!props.launchPhase &&
            CANCELLABLE_LAUNCH_PHASES.has(props.launchPhase.kind),
    );

    return { workedSummary, workedSecondary, rightText, showCancelLogin };
}
