// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
vi.mock("@/app/store/mps-history", () => ({ readMuxEventHistory: () => Promise.resolve([]) }));
vi.mock("@/app/notification/sound/sound-service", () => ({ playShutdownPendingTone: () => true }));
import {
    countdownText,
    parseCountdown,
    replayCountdowns,
    secondsLeft,
    withCountdown,
    withoutCountdown,
    type SensitiveCountdown,
} from "./sensitive-pending";

const view = (over: Record<string, unknown> = {}) => ({
    request_id: "r1",
    op: "widget_install",
    summary: "install a widget (widget acme.clock)",
    requester: "Parlo",
    approver: "Camper",
    deadline_ms: 100_000,
    status: "pending",
    ...over,
});

describe("sensitive countdowns", () => {
    it("reads a pending view and nothing else", () => {
        expect(parseCountdown(view())?.summary).toBe("install a widget (widget acme.clock)");
        expect(parseCountdown(view({ status: "proceeding" }))).toBeNull();
        expect(parseCountdown({ request_id: "r1" })).toBeNull();
        expect(parseCountdown(null)).toBeNull();
    });

    it("says who asked, who approved, and what happens", () => {
        const c = parseCountdown(view())!;
        expect(countdownText(c, 12)).toBe("Parlo asked: install a widget (widget acme.clock). Approved by Camper. Starts in 12 s.");
        const unreviewed = parseCountdown(view({ approver: undefined }))!;
        expect(countdownText(unreviewed, 3)).toContain("No super agent reviewed this.");
    });

    it("a request already shown is replaced, and a cleared one removed", () => {
        const a = parseCountdown(view())!;
        const b = parseCountdown(view({ request_id: "r2" }))!;
        const both = withCountdown(withCountdown([], a), b);
        expect(both.map((c) => c.request_id)).toEqual(["r1", "r2"]);
        expect(withCountdown(both, { ...a, deadline_ms: 5 } as SensitiveCountdown)).toHaveLength(2);
        expect(withoutCountdown(both, "r1").map((c) => c.request_id)).toEqual(["r2"]);
    });

    it("a replay shows every pending one not cleared since or past its deadline", () => {
        const a = view({ request_id: "a" });
        const b = view({ request_id: "b" });
        const old = view({ request_id: "old", deadline_ms: 10_000 });
        const ids = (list: SensitiveCountdown[]) => list.map((c) => c.request_id);
        expect(ids(replayCountdowns([a, b, old], [], 50_000))).toEqual(["a", "b"]);
        expect(ids(replayCountdowns([a, b], [{ request_id: "b", outcome: "vetoed" }], 50_000))).toEqual(["a"]);
        expect(ids(replayCountdowns([a], [{ request_id: "other", outcome: "done" }], 50_000))).toEqual(["a"]);
        expect(replayCountdowns([a], [], 100_000)).toEqual([]);
    });

    it("counts whole seconds and never below zero", () => {
        const c = parseCountdown(view())!;
        expect(secondsLeft(c, 99_100)).toBe(1);
        expect(secondsLeft(c, 200_000)).toBe(0);
    });
});
