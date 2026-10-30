// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The words and numbers of the working row (working-row.tsx). */

import { formatCompactNumber } from "@/util/format-count";
import type { TurnTokens } from "../types";

export function pickThinkingPhrase(_exclude?: string): string {
    return "Working";
}

export function ingToEd(_phrase: string): string {
    return "Worked";
}

/** One turn's output: "2.4k tokens". Claude Code's convention: one number,
 *  the model's output; input is the context re-sent on every call, which the
 *  composer's context meter shows. SPEC_AGENT_TURN_TOKEN_COUNTER_CLAUDE_CONVENTION_2026_10_07.md. */
export function fmtOutputTokens(output: number): string {
    return `${formatCompactNumber(output)} tokens`;
}

/** Live readout: "\u2193 2.4k tokens" while the model streams or its tools run,
 *  "\u2191 2.4k tokens" while a request is in flight, no arrow between two
 *  passes of a turn (nothing is moving). The number only grows. */
export function fmtTurnTokens(output: number, t: TurnTokens | null | undefined): string {
    const arrow = t == null ? "" : t.requesting ? "\u2191 " : "\u2193 ";
    return `${arrow}${fmtOutputTokens(output)}`;
}
