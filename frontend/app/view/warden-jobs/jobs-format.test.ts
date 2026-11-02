// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import type { JobView } from "@/types/rpc/JobView";

import { mkJob, mkRun } from "./jobs-fixtures";
import {
    costText,
    describeCron,
    describeTrigger,
    droppedText,
    failingCount,
    inWords,
    kanbanTask,
    runBadge,
    stateText,
} from "./jobs-format";

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

    it("numbers the days as standard cron does: 0 and 7 are Sunday, 1 is Monday", () => {
        expect(describeCron("0 6 * * 0")).toBe("Sundays 06:00 UTC");
        expect(describeCron("0 6 * * 7")).toBe("Sundays 06:00 UTC");
        expect(describeCron("0 6 * * 1")).toBe("Mondays 06:00 UTC");
        expect(describeCron("0 6 * * 6")).toBe("Saturdays 06:00 UTC");
        expect(describeCron("0 6 * * MON-FRI")).toBe("weekdays 06:00 UTC");
        expect(describeCron("0 6 * * sun")).toBe("Sundays 06:00 UTC");
        expect(describeCron("0 6 * * 8")).toBe("0 6 * * 8");
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
        expect(runBadge(mkRun({ infra: "interrupted", failed: true }))).toMatchObject({
            label: "failed",
            tone: "error",
            detail: "failed: interrupted",
        });
    });

    it("says what kept a run from running", () => {
        expect(runBadge(mkRun({ state: "missed", reason: "target_stopped", failed: true }))).toMatchObject({
            label: "missed",
            tone: "error",
            detail: "missed: agent not running",
        });
        const skipped = runBadge(mkRun({ state: "skipped", reason: "paused_all", skip_count: 3 }));
        expect(skipped).toMatchObject({
            label: "skipped",
            tone: "neutral",
            hollow: true,
            detail: "skipped: Pause all (×3)",
        });
        expect(runBadge(mkRun({ state: "pending", reason: "target_stopped", ended_at_ms: null })).label).toBe(
            "waiting"
        );
        expect(runBadge(mkRun({ state: "pending", reason: "expired", failed: true })).label).toBe("missed");
        expect(runBadge(mkRun({ state: "pending", reason: "superseded" })).label).toBe("replaced");
        expect(runBadge(mkRun({ state: "running" })).tone).toBe("accent");
    });
});

describe("cloud jobs", () => {
    it("reads a cloud job's trigger, and the dropped events in words", () => {
        expect(describeTrigger(mkJob({ source: "cloud", trigger_kind: "manual", expression: "" }))).toBe("when it reports");
        expect(describeTrigger(mkJob({ source: "cloud", trigger_kind: "cron", expression: "0 2 * * *" }))).toBe("daily 02:00 UTC");
        expect(droppedText([])).toBe("");
        expect(droppedText([{ reason: "unknown job", count: 0 }])).toBe("");
        expect(droppedText([{ reason: "bad signature", count: 1 }])).toContain("1 cloud event was dropped");
        expect(droppedText([{ reason: "bad signature", count: 1 }, { reason: "unknown job", count: 4 }])).toContain("5 cloud events were dropped");
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

describe("kanbanTask", () => {
    it("reads the incident's task srv linked to a run, and nothing else", () => {
        expect(kanbanTask(mkRun({ links: { kanban_task: { id: "w1", num: 12 } } }))).toEqual({ id: "w1", num: 12 });
        expect(kanbanTask(mkRun())).toBeNull();
        expect(kanbanTask(mkRun({ links: { kanban_task: { id: "w1", num: "12" } } }))).toBeNull();
    });
});

describe("describeTrigger", () => {
    it("reads a cron expression in the job's own zone", () => {
        expect(describeCron("0 6 * * 1", "Europe/Berlin")).toBe("Mondays 06:00 Europe/Berlin");
        expect(describeTrigger(mkJob({ expression: "0 6 * * *", tz: "America/New_York" }))).toBe(
            "daily 06:00 America/New_York"
        );
        expect(describeTrigger(mkJob())).toBe("daily 06:00 UTC");
    });

    it("says an interval, a single time and a job run by hand", () => {
        const every = (every_secs: number) =>
            describeTrigger(mkJob({ trigger_kind: "every", expression: "", every_secs }));
        expect(every(1800)).toBe("every 30 min");
        expect(every(7200)).toBe("every 2 h");
        expect(every(90)).toBe("every 90 s");
        expect(describeTrigger(mkJob({ trigger_kind: "once", expression: "", once_at_ms: 1_800_000_000_000 }))).toMatch(
            /^once, /
        );
        expect(describeTrigger(mkJob({ trigger_kind: "manual", expression: "" }))).toBe("by hand");
        expect(describeTrigger(mkJob({ source: "drone", expression: "" }))).toBe("by hand");
    });
});
