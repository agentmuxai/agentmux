// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import type { JobView } from "@/types/rpc/JobView";

import { costText, describeCron, failingCount, inWords, runBadge, stateText } from "./jobs-format";
import { mkRun } from "./jobs-fixtures";

describe("describeCron", () => {
    it("says the common shapes in words, in UTC", () => {
        expect(describeCron("* * * * *")).toBe("every minute");
        expect(describeCron("*/15 * * * *")).toBe("every 15 min");
        expect(describeCron("5 * * * *")).toBe("hourly at :05");
        expect(describeCron("0 */6 * * *")).toBe("every 6 h at :00");
        expect(describeCron("0 6 * * *")).toBe("daily 06:00 UTC");
        expect(describeCron("0 6 * * 1")).toBe("Mondays 06:00 UTC");
        expect(describeCron("30 9 * * 1,3")).toBe("Mondays, Wednesdays 09:30 UTC");
        expect(describeCron("0 9 * * 1-5")).toBe("weekdays 09:00 UTC");
        expect(describeCron("0 0 1 * *")).toBe("monthly on day 1 00:00 UTC");
    });

    it("leaves anything else as the expression", () => {
        expect(describeCron("0 0 1 1 *")).toBe("0 0 1 1 *");
        expect(describeCron("0-30/5 9-17 * * *")).toBe("0-30/5 9-17 * * *");
        expect(describeCron("nonsense")).toBe("nonsense");
    });
});

describe("runBadge", () => {
    it("shows the outcome of a run that ran, red only for a failure", () => {
        expect(runBadge(mkRun()).label).toBe("ok");
        expect(runBadge(mkRun()).tone).toBe("ok");
        expect(runBadge(mkRun({ outcome: "findings" })).tone).toBe("warn");
        expect(runBadge(mkRun({ outcome: "unknown" }))).toMatchObject({ label: "unknown", tone: "neutral" });
        expect(runBadge(mkRun({ outcome: "failed", failed: true }))).toMatchObject({ label: "failed", tone: "error" });
        expect(runBadge(mkRun({ infra: "interrupted", failed: true }))).toMatchObject({ label: "failed", tone: "error", detail: "failed: interrupted" });
    });

    it("says what kept a run from running", () => {
        expect(runBadge(mkRun({ state: "missed", reason: "target_stopped", failed: true }))).toMatchObject({
            label: "missed",
            tone: "error",
            detail: "missed: agent not running",
        });
        const skipped = runBadge(mkRun({ state: "skipped", reason: "paused_all", skip_count: 3 }));
        expect(skipped).toMatchObject({ label: "skipped", tone: "neutral", hollow: true, detail: "skipped: Pause all (×3)" });
        expect(runBadge(mkRun({ state: "pending", reason: "target_stopped", ended_at_ms: null })).label).toBe("waiting");
        expect(runBadge(mkRun({ state: "pending", reason: "expired", failed: true })).label).toBe("missed");
        expect(runBadge(mkRun({ state: "pending", reason: "superseded" })).label).toBe("replaced");
        expect(runBadge(mkRun({ state: "running" })).tone).toBe("accent");
    });
});

describe("the rest", () => {
    it("says a job's state, a cost, and a next run", () => {
        expect(stateText("paused_all").label).toBe("Paused by Pause all");
        expect(stateText("paused").label).toBe("Paused");
        expect(stateText("active").tone).toBe("ok");
        expect(costText(0.4219, true)).toBe("$0.42");
        expect(costText(0, false)).toBe("—");
        expect(inWords(3 * 3_600_000, 0)).toBe("in 3 h");
        expect(inWords(12 * 60_000, 0)).toBe("in 12 min");
        expect(inWords(0, 10)).toBe("now");
    });

    it("counts the failing jobs", () => {
        const jobs = [{ failing: true }, { failing: false }, { failing: true }] as JobView[];
        expect(failingCount(jobs)).toBe(2);
        expect(failingCount(null)).toBe(0);
    });
});
