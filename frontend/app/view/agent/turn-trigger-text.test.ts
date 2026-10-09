// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { parseTurnLedger, type TurnTrigger } from "@/app/store/agent-pane-state/turn-ledger";
import { absorbedSummary, isExternalTrigger, triggerLeadIn, workedVerb } from "./turn-trigger-text";

const t = (kind: TurnTrigger["kind"], from: string | null = null): TurnTrigger => ({ kind, from });

describe("turn trigger words", () => {
    it("a turn the user started needs none", () => {
        for (const kind of ["user", "broadcast", "system"] as const) {
            expect(isExternalTrigger(t(kind))).toBe(false);
            expect(triggerLeadIn(t(kind))).toBeNull();
            expect(workedVerb(t(kind))).toBe("Worked");
        }
        expect(workedVerb(null)).toBe("Worked");
    });

    it("names whoever else started it, at the start and on the Worked line", () => {
        expect(triggerLeadIn(t("agent", "AgentX"))).toBe("↳ jekt from AgentX");
        expect(workedVerb(t("agent", "AgentX"))).toBe("Worked on AgentX's jekt");
        expect(triggerLeadIn(t("service", "github-consumer"))).toBe("↳ github-consumer");
        expect(workedVerb(t("service", "github-consumer"))).toBe("Worked on github-consumer's notice");
        expect(triggerLeadIn(t("schedule", "cron"))).toBe("↳ scheduled run");
        expect(workedVerb(t("schedule", "cron"))).toBe("Worked on a scheduled run");
        const done = 'Background command "npm test" completed (exit code 0)';
        expect(triggerLeadIn(t("task", done))).toBe(`↳ ${done}`);
        expect(workedVerb(t("task", done))).toBe("Worked on a finished task");
    });

    it("an unnamed sender still reads as a sentence", () => {
        expect(triggerLeadIn(t("agent"))).toBe("↳ jekt from another agent");
        expect(workedVerb(t("agent"))).toBe("Worked on a jekt");
        expect(triggerLeadIn(t("task"))).toBe("↳ a background task finished");
    });

    it("counts what joined the turn by kind, and what srv didn't list", () => {
        const joined = [t("agent", "a"), t("user"), t("agent", "b"), t("system")];
        expect(absorbedSummary(joined, 4)).toBe("+2 jekts  ·  +1 your message");
        expect(absorbedSummary([t("service", "x")], 25)).toBe("+1 notice  ·  +24 more");
        expect(absorbedSummary([], 0)).toBeNull();
    });
});

describe("parseTurnLedger: trigger and absorbed", () => {
    it("reads them, and drops what it doesn't know", () => {
        const l = parseTurnLedger({
            turn_id: 1,
            started_at_ms: 1,
            active: true,
            trigger: { kind: "agent", from: "agentx" },
            absorbed: [{ kind: "user" }, { kind: "telepathy" }, null],
        });
        expect(l?.trigger).toEqual({ kind: "agent", from: "agentx" });
        expect(l?.absorbed).toEqual([{ kind: "user", from: null }]);
        expect(parseTurnLedger({ turn_id: 1, started_at_ms: 1, active: true })?.absorbed).toEqual([]);
    });
});
