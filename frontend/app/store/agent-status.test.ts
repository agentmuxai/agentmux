// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    bestStatus,
    HELD_ORDER,
    isActiveStatus,
    rankAgentStatus,
    STATUS_ORDER,
    subagentStatus,
    type AgentStatus,
    type AgentStatusFacts,
    type AgentStatusKind,
    type HeldReason,
} from "./agent-status";

/** The smallest facts that make `kind` eligible on its own. */
const ALONE: Record<AgentStatusKind, AgentStatusFacts> = {
    "needs-you": { live: true, needsYou: true },
    held: { live: false, held: { compacting: true } },
    working: { live: true },
    interrupted: { live: false, ended: "interrupted" },
    errored: { live: false, ended: "errored" },
    idle: { live: false },
};

/** Both sets of facts at once. At most one of them carries an `ended`. */
function merge(a: AgentStatusFacts, b: AgentStatusFacts): AgentStatusFacts {
    return {
        live: a.live || b.live,
        needsYou: a.needsYou || b.needsYou,
        held: { ...a.held, ...b.held },
        ended: a.ended ?? b.ended,
    };
}

describe("rankAgentStatus", () => {
    it.each(STATUS_ORDER.map((k) => [k]))("%s wins on its own facts", (kind) => {
        expect(rankAgentStatus(ALONE[kind]).kind).toBe(kind);
    });

    // Every pair the facts can hold at once: the better one wins. Interrupted
    // and errored are both an `ended`, so they never meet; idle is the absence
    // of everything, so it never beats anything.
    const pairs: [AgentStatusKind, AgentStatusKind][] = [];
    STATUS_ORDER.forEach((better, i) =>
        STATUS_ORDER.slice(i + 1).forEach((worse) => {
            if (better === "interrupted" && worse === "errored") return;
            pairs.push([better, worse]);
        })
    );
    it.each(pairs)("%s outranks %s", (better, worse) => {
        expect(rankAgentStatus(merge(ALONE[better], ALONE[worse])).kind).toBe(better);
        expect(rankAgentStatus(merge(ALONE[worse], ALONE[better])).kind).toBe(better);
    });

    it("a live turn hides how the last one ended", () => {
        expect(rankAgentStatus({ live: true, ended: "errored" }).kind).toBe("working");
        expect(rankAgentStatus({ live: true, ended: "interrupted" }).kind).toBe("working");
    });

    it("needs-you counts only while a turn is in flight (a stale flag, #4234)", () => {
        expect(rankAgentStatus({ live: false, needsYou: true }).kind).toBe("idle");
        expect(rankAgentStatus({ live: false, needsYou: true, held: { reconnecting: true } })).toEqual({
            kind: "held",
            reason: "reconnecting",
        });
    });

    it("holds with no turn in flight", () => {
        expect(rankAgentStatus({ live: false, held: { reconnecting: true } }).kind).toBe("held");
    });

    const heldPairs: [HeldReason, HeldReason][] = [];
    HELD_ORDER.forEach((better, i) => HELD_ORDER.slice(i + 1).forEach((worse) => heldPairs.push([better, worse])));
    it.each(heldPairs)("held: %s outranks %s", (better, worse) => {
        expect(rankAgentStatus({ live: true, held: { [better]: true, [worse]: true } })).toEqual({ kind: "held", reason: better });
    });

    it("a reason set to false doesn't hold", () => {
        expect(rankAgentStatus({ live: true, held: { reconnecting: false, stopping: false } }).kind).toBe("working");
    });
});

describe("bestStatus", () => {
    it("picks the best kind, and is idle for none", () => {
        const s = (kind: Exclude<AgentStatusKind, "held">): AgentStatus => ({ kind });
        expect(bestStatus([])).toEqual({ kind: "idle" });
        expect(bestStatus([s("idle"), s("interrupted"), s("working")])).toEqual({ kind: "working" });
        expect(bestStatus([s("idle"), s("interrupted")])).toEqual({ kind: "interrupted" });
        expect(bestStatus([{ kind: "held", reason: "stopping" }, s("working")])).toEqual({ kind: "held", reason: "stopping" });
    });
});

describe("subagentStatus", () => {
    it("is working while active and its parent's turn is (or may be) in flight", () => {
        expect(subagentStatus("active", true).kind).toBe("working");
        expect(subagentStatus("active").kind).toBe("working");
    });

    it("is interrupted when abandoned, or active after its parent's turn ended", () => {
        expect(subagentStatus("abandoned", true).kind).toBe("interrupted");
        expect(subagentStatus("abandoned").kind).toBe("interrupted");
        expect(subagentStatus("active", false).kind).toBe("interrupted");
    });

    it("is idle once completed, whatever the parent does", () => {
        expect(subagentStatus("completed", true).kind).toBe("idle");
        expect(subagentStatus("completed", false).kind).toBe("idle");
    });
});

describe("isActiveStatus", () => {
    it("is true for needs-you, held and working only", () => {
        const active = STATUS_ORDER.filter((kind) => isActiveStatus(rankAgentStatus(ALONE[kind])));
        expect(active).toEqual(["needs-you", "held", "working"]);
    });
});
