// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { actorName, originLine } from "./mcp-origin";

const names = (id: string) => ({ korp: "Korp", a2: "AgentY" })[id];

describe("actorName", () => {
    it("names each kind of actor the srv records", () => {
        expect(actorName("operator", names)).toBe("you");
        expect(actorName("agentmux", names)).toBe("AgentMux");
        expect(actorName("unproven", names)).toBe("a connection that was not verified");
        expect(actorName("person:p1 (AgentMux Web)", names)).toBe("a person using AgentMux Web");
    });

    it("names an agent by its display name, or by its id when the list does not have it", () => {
        expect(actorName("agent:korp", names)).toBe("Korp");
        expect(actorName("agent:gone", names)).toBe("gone");
    });

    it("says unknown for a server or bind made before this was recorded, and for anything unexpected", () => {
        expect(actorName("unknown", names)).toBe("unknown (made before this was recorded)");
        expect(actorName("", names)).toBe("unknown (made before this was recorded)");
    });
});

describe("originLine", () => {
    const at = new Date(2026, 10, 8, 15, 46, 12).getTime();

    it("says who and when for each kind", () => {
        // The date part depends on today (a year is added for an earlier year), the time of day does not.
        expect(originLine("added", "operator", at, names)).toMatch(/^Added by you, .*3:46:12 PM$/);
        expect(originLine("changed", "agent:korp", at, names)).toMatch(/^Last changed by Korp, .*3:46:12 PM$/);
        expect(originLine("bound", "agentmux", at, names)).toMatch(/^Bound by AgentMux, .*3:46:12 PM$/);
    });

    it("leaves the time out when none was recorded", () => {
        expect(originLine("added", "unknown", 0, names)).toBe("Added by unknown (made before this was recorded)");
        expect(originLine("bound", "operator", 0, names)).toBe("Bound by you");
    });
});
