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

    it("words an agent that is out of quota or signed out (SPEC_UNRESPONSIVE_AGENTS_2026_10_23 §3.1)", () => {
        expect(agentStateWords("limited")).toBe("out of quota");
        expect(agentStateWords("signed_out")).toBe("signed out");
        expect(agentTone("limited")).toBe("warn");
        expect(agentTone("signed_out")).toBe("error");
    });

    it("reads a state srv added, which this build doesn't word yet, as itself", () => {
        const added = "hibernating" as KanbanAgentState;
        expect(agentStateWords(added)).toBe("hibernating");
        expect(agentTone(added)).toBe("idle");
    });
});
