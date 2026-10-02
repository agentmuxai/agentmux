// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { TurnTokens } from "@/app/view/agent/types";

/**
 * What a turn actually ADDED to the conversation on the input side, as opposed
 * to the input it sent.
 *
 * Every API call in a turn re-sends the whole conversation, so the raw
 * `input` of a call (and the whole-turn input the result event reports, which
 * sums every call's) is essentially the size of the context, repeated: a
 * one-line question late in a long session reads as "↑180k". The turn's own
 * contribution is the growth of the context across the turn: the user's
 * message, the tool results, and the assistant's earlier output that became
 * input to the next call.
 *
 *     added = context size at the turn's last call - context size before it began
 *
 * `contextBaseline` is the context size going in, captured by the reducer at
 * the turn's first TokensIn. Returns undefined when there is no baseline (a
 * provider that reports no live usage), so callers can fall back to the raw
 * figure rather than inventing one. Never negative: a compaction mid-turn
 * shrinks the context, which is not a negative contribution.
 *
 * Output is already a per-turn quantity and is shown as reported. See
 * docs/specs/SPEC_AGENT_WORKING_ROW_MONO_SUMMARY_2026_10_02.md section 3.4.
 */
export function turnAddedInput(tokens: Pick<TurnTokens, "input" | "contextBaseline"> | null | undefined): number | undefined {
    if (!tokens || tokens.contextBaseline == null) return undefined;
    return Math.max(0, tokens.input - tokens.contextBaseline);
}
