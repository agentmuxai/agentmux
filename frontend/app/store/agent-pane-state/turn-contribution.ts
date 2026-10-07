// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { TurnTokens } from "@/app/view/agent/types";

/** Characters per token for a call's streamed-output estimate, as Claude Code
 *  uses for its own spinner (`Math.round(responseLength / 4)`). */
const CHARS_PER_TOKEN = 4;

/** The turn's output by its parts: earlier calls' exact counts, plus the latest
 *  call's exact count or, while it is still streaming, its estimate. A new
 *  call's `outputDone` is this figure of the turn so far. */
export function outputSoFar(t: Pick<TurnTokens, "output" | "outputDone" | "streamedChars">): number {
    const current = Math.max(t.output, Math.round((t.streamedChars ?? 0) / CHARS_PER_TOKEN));
    return (t.outputDone ?? 0) + current;
}

/**
 * The working row's one live counter: the turn's output so far, the way Claude
 * Code counts it. It only grows within a turn: every API call re-sends the
 * conversation, so input is the context size, not something the turn produced,
 * and it isn't counted (the composer's context meter shows it).
 *
 * See docs/specs/SPEC_AGENT_TURN_TOKEN_COUNTER_CLAUDE_CONVENTION_2026_10_07.md.
 */
export function turnOutputTokens(
    t: Pick<TurnTokens, "output" | "outputDone" | "streamedChars" | "shownOutput"> | null | undefined,
): number | undefined {
    if (!t) return undefined;
    return Math.max(t.shownOutput ?? 0, outputSoFar(t));
}

/** `next` with its `shownOutput` raised to its current total. Never lowered, so
 *  a call whose exact count comes in under its estimate doesn't move the
 *  counter back. */
export function withShownOutput(next: TurnTokens): TurnTokens {
    return { ...next, shownOutput: turnOutputTokens(next) };
}
