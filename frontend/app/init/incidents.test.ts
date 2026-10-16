// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Recovery notices (SPEC_RECOVERY_NOTICES_AND_TOASTS_2026_10_16.md §4.2).

import { beforeEach, describe, expect, it, vi } from "vitest";

const pushed: NotificationType[] = [];
const removed: string[] = [];
const handlers = new Map<string, (p: unknown) => void>();
const api = {
    listIncidents: vi.fn(),
    acknowledgeIncident: vi.fn(async () => true),
    listen: vi.fn(async (event: string, fn: (p: unknown) => void) => {
        handlers.set(event, fn);
        return () => {};
    }),
};

vi.mock("@/store/global", () => ({
    getApi: () => api,
    pushNotification: (n: NotificationType) => pushed.push(n),
    removeNotificationById: (id: string) => removed.push(id),
}));

// vi.mock above is hoisted, so this import already sees the mock.
import { hasOpenIncident, incidentMessage, incidentNotification, incidentReport, recentIncidents, surfaceIncidents } from "./incidents";

const crash = {
    id: "1-a",
    at_ms: 1_000,
    kind: "renderer_crashed",
    what: "An AgentMux window crashed",
    cause: "GPU process crashed",
    did: ["Every agent kept running: agents don't run in the window."],
};

describe("recovery notices", () => {
    beforeEach(() => {
        pushed.length = 0;
        removed.length = 0;
    });

    it("say what happened, why, and what AgentMux saved, and stay until closed", () => {
        const n = incidentNotification(crash);
        expect(n.title).toBe("An AgentMux window crashed");
        expect(n.message).toBe("GPU process crashed Every agent kept running: agents don't run in the window.");
        expect(n.persistent).toBe(true);
        expect(n.expiration).toBeUndefined();
        expect(incidentMessage({ ...crash, cause: undefined, did: [], lost: ["An unsent message."] })).toBe("May be lost: An unsent message.");
    });

    it("closing one acknowledges it", () => {
        incidentNotification(crash).onClose?.();
        expect(api.acknowledgeIncident).toHaveBeenCalledWith("1-a");
    });

    it("shows what wasn't closed, oldest first, and follows other windows", async () => {
        const older = { ...crash, id: "0-z", what: "older" };
        const done = { ...crash, id: "2-b", acknowledged_at_ms: 5 };
        api.listIncidents.mockResolvedValue({ incidents: [done, crash, older] });
        await surfaceIncidents();
        expect(pushed.map((n) => n.id)).toEqual(["incident-0-z", "incident-1-a"]);
        // Closed in another window: closed here too.
        handlers.get("incident-acknowledged")?.({ id: "1-a" });
        expect(removed).toEqual(["incident-1-a"]);
        // Recorded while this window is open: shown now.
        handlers.get("incident-recorded")?.({ ...crash, id: "3-c", at_ms: 9_000 });
        expect(pushed.at(-1)?.id).toBe("incident-3-c");

        // The status bar's history (§4.3): every one, closed or not, newest first.
        expect(recentIncidents().map((i) => i.id)).toEqual(["3-c", "2-b", "1-a", "0-z"]);
        expect(recentIncidents().find((i) => i.id === "1-a")?.acknowledged_at_ms).toBeGreaterThan(0);
        expect(hasOpenIncident()).toBe(true);
        for (const id of ["0-z", "3-c"]) handlers.get("incident-acknowledged")?.({ id });
        expect(hasOpenIncident()).toBe(false);
    });

    it("copy as a report with everything the record says", () => {
        const report = incidentReport({ ...crash, lost: ["An unsent message."], evidence: "host log, 2026-10-16T21:31:39Z" });
        expect(report).toBe(
            [
                "AgentMux incident 1-a (renderer_crashed)",
                "When: 1970-01-01T00:00:01.000Z",
                "What: An AgentMux window crashed",
                "Why: GPU process crashed",
                "Did: Every agent kept running: agents don't run in the window.",
                "May be lost: An unsent message.",
                "Evidence: host log, 2026-10-16T21:31:39Z",
            ].join("\n")
        );
    });
});
