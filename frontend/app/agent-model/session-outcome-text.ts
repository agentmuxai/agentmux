// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The words a session-outcome divider shows: the row that draws them and the transcript search read this one place,
 * so a word on screen is a word the search finds.
 */

import type { SessionOutcomeNode } from "./node-types";

export function sessionOutcomeLabel(n: Pick<SessionOutcomeNode, "outcome" | "continued">): string {
    if (n.outcome === "resumed") return "Session continued";
    return n.continued ? "Session continued · reconstructed context" : "New session started";
}

/** The line under the rule; none for a resumed session. */
export function sessionOutcomeDetail(n: Pick<SessionOutcomeNode, "outcome" | "continued">): string | null {
    if (n.outcome === "resumed") return null;
    return n.continued
        ? "The provider couldn't resume the previous session, so this new one was given AgentMux's record of the conversation: a running summary and the recent exchange"
        : "The agent doesn't have the conversation above this line";
}
