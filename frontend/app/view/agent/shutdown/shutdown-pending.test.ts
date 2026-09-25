// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { clearedRequestId, parsePending, pendingTitle, replayPending, secondsLeft } from "./shutdown-pending";

const pending = {
    request_id: "r1",
    block_id: "b1",
    by: "Korp",
    via: "QuitSelf",
    reason: "task finished",
    deadline_ms: 20_000,
};

describe("shutdown-pending (SPEC_AGENT_SELF_QUIT §6.5)", () => {
    it("parses the event and rejects one without an id or deadline", () => {
        expect(parsePending(pending)).toEqual(pending);
        expect(parsePending({ request_id: "r1" })).toBeNull();
        expect(parsePending(null)).toBeNull();
        expect(clearedRequestId({ request_id: "r1", outcome: "kept_by_user" })).toBe("r1");
    });

    it("counts whole seconds down to zero, never below", () => {
        expect(secondsLeft(parsePending(pending)!, 5_000)).toBe(15);
        expect(secondsLeft(parsePending(pending)!, 19_001)).toBe(1);
        expect(secondsLeft(parsePending(pending)!, 25_000)).toBe(0);
    });

    it("a pane opened mid-countdown shows it, unless it was cleared or ran out", () => {
        expect(replayPending(pending, null, 10_000)?.request_id).toBe("r1");
        expect(replayPending(pending, { request_id: "r1" }, 10_000)).toBeNull();
        expect(replayPending(pending, { request_id: "an-older-one" }, 10_000)?.request_id).toBe("r1");
        expect(replayPending(pending, null, 20_000)).toBeNull();
    });

    it("names who asked, and says 'itself' for the agent's own request", () => {
        const p = parsePending(pending)!;
        expect(pendingTitle(p, "camper", "Camper")).toBe("Korp asked to shut down Camper: task finished");
        expect(pendingTitle({ ...p, by: "camper" }, "camper", "Camper")).toBe("Camper asked to shut itself down: task finished");
        expect(pendingTitle({ ...p, reason: "" }, "camper", "Camper")).toBe("Korp asked to shut down Camper");
    });
});
