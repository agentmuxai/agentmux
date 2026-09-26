// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { describeError, formatDescribedError, formatErrorReport, toIsoLocalString } from "./error-report";

describe("formatErrorReport", () => {
    const WHEN = new Date(2026, 8, 25, 14, 30, 5); // month is 0-indexed: September

    it("includes only title and When when nothing else is given", () => {
        const report = formatErrorReport({ title: "Agent failure", when: WHEN });
        expect(report).toBe(`AgentMux error: Agent failure\nWhen: ${toIsoLocalString(WHEN)}`);
    });

    it("omits message/code/where/details lines that have nothing to say", () => {
        const report = formatErrorReport({ title: "X", message: "", code: "", where: "", when: WHEN });
        expect(report).not.toContain("Code:");
        expect(report).not.toContain("Where:");
        expect(report).not.toContain("Details:");
    });

    it("includes every field, in order, when all are given", () => {
        const report = formatErrorReport({
            title: "Agent failure",
            message: "Rate limited",
            code: "HTTP 429",
            where: "Agent pane · pane ab12cd34 · claude",
            when: WHEN,
            details: "line one\nline two",
        });
        expect(report).toBe(
            [
                "AgentMux error: Agent failure",
                "Rate limited",
                "Code: HTTP 429",
                "Where: Agent pane · pane ab12cd34 · claude",
                `When: ${toIsoLocalString(WHEN)}`,
                "",
                "Details:",
                "line one\nline two",
            ].join("\n")
        );
    });

    it("keeps full stderr / stack text, never truncated to what's on screen", () => {
        const longStderr = Array.from({ length: 200 }, (_, i) => `stderr line ${i}`).join("\n");
        const report = formatErrorReport({ title: "Shell failed", when: WHEN, details: longStderr });
        expect(report).toContain("stderr line 0");
        expect(report).toContain("stderr line 199");
        // Every original line break survives.
        expect(report.split("\n").filter((l) => l.startsWith("stderr line")).length).toBe(200);
    });

    it("redacts the whole report, including message and details", () => {
        const report = formatErrorReport({
            title: "Auth failed",
            message: "token=ghp_abcdefghijklmnopqrstuvwxyz0123",
            when: WHEN,
            details: "Authorization: Bearer abc.def.ghi",
        });
        expect(report).not.toContain("ghp_abcdef");
        expect(report).not.toContain("abc.def.ghi");
        expect(report).toContain("[redacted");
    });

    it("defaults When to now when omitted", () => {
        const before = Date.now();
        const report = formatErrorReport({ title: "X" });
        const after = Date.now();
        const match = report.match(/^When: (.+)$/m);
        expect(match).not.toBeNull();
        // Reconstruct a rough timestamp check: the local-time string must at
        // least contain today's year, proving `when` really defaulted to now
        // rather than to some fixed/omitted value.
        const year = new Date(before).getFullYear();
        expect(match![1]).toContain(String(year));
        expect(after - before).toBeLessThan(5000);
    });
});

describe("describeError", () => {
    it("keeps the stack of a real Error, unlike String(error)", () => {
        const err = new Error("boom");
        const d = describeError(err);
        expect(d.name).toBe("Error");
        expect(d.message).toBe("boom");
        expect(d.stack).toContain("boom");
        expect(String(err)).not.toContain(err.stack ?? "\0impossible");
    });

    it("describes a plain thrown string", () => {
        expect(describeError("plain string throw")).toEqual({ name: "Error", message: "plain string throw" });
    });

    it("does not serialize a plain Error to '{}'", () => {
        // The bug this fixes (§6.1): `String(error)` / naive JSON.stringify
        // on an Error instance loses everything but "Error: message" or less.
        const d = describeError(new TypeError("nope"));
        expect(d.name).toBe("TypeError");
        expect(d.message).toBe("nope");
    });

    it("describes an arbitrary thrown object", () => {
        const d = describeError({ code: 42, reason: "bad" });
        expect(d.name).toBe("Error");
        expect(d.message).toContain("42");
        expect(d.message).toContain("bad");
    });

    it("carries a cause's message", () => {
        const cause = new Error("root cause");
        const err = new Error("wrapper", { cause });
        const d = describeError(err);
        expect(d.cause).toBe("root cause");
    });
});

describe("formatDescribedError", () => {
    it("includes name, message, cause and stack when present", () => {
        const text = formatDescribedError({ name: "Error", message: "boom", stack: "at foo\nat bar", cause: "root" });
        expect(text).toContain("Error: boom");
        expect(text).toContain("Caused by: root");
        expect(text).toContain("at foo\nat bar");
    });

    it("omits cause and stack when absent", () => {
        const text = formatDescribedError({ name: "Error", message: "boom" });
        expect(text).toBe("Error: boom");
    });
});
