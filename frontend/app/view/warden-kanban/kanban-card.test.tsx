// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { agentStateWords, agentTone } from "./kanban-card";
import type { KanbanAgentState } from "./kanban-types";

describe("agent state words", () => {
    it("words the states it knows", () => {
        expect(agentStateWords("waiting")).toBe("waiting on a question");
        expect(agentStateWords("gone")).toBe("closed");
        expect(agentStateWords(undefined)).toBe("not here");
    });

    it("reads a state srv added, which this build doesn't word yet, as itself", () => {
        // srv reports limited and signed_out (SPEC_UNRESPONSIVE_AGENTS_2026_10_23 §3.1).
        const added = "limited" as KanbanAgentState;
        expect(agentStateWords(added)).toBe("limited");
        expect(agentTone(added)).toBe("idle");
    });
});
